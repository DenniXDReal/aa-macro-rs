//! Discord webhook notifications: start/stop, win/loss (+ screenshot),
//! disconnects, a summary. Sent straight to Discord from a background
//! thread, so a slow or broken webhook never blocks the macro.

use crate::capture::Frame;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::time::Duration;

pub const URL_KEY: &str = "discord_webhook";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct WebhookSettings {
    pub enabled: bool,
    /// Discord user ID to @mention on important events (digits only).
    pub ping_user: String,
    pub on_result: bool,
    pub on_disconnect: bool,
    pub on_start_stop: bool,
    pub screenshot: bool,
}

impl Default for WebhookSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            ping_user: String::new(),
            on_result: true,
            on_disconnect: true,
            on_start_stop: true,
            screenshot: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Info,
    Win,
    Loss,
    Problem,
}

impl Kind {
    fn color(self) -> u32 {
        match self {
            Kind::Info => 0x8B5CF6,
            Kind::Win => 0x4ADE80,
            Kind::Loss => 0xF87171,
            Kind::Problem => 0xFBBF24,
        }
    }
}

pub struct Message {
    pub kind: Kind,
    pub title: String,
    pub body: String,
    pub screenshot: Option<Frame>,
    pub ping: bool,
}

/// `https://discord.com/api/webhooks/<id>/<token>` (also ptb./canary., discordapp.com).
pub fn valid_url(url: &str) -> bool {
    let u = url.trim();
    let rest = [
        "https://discord.com/api/webhooks/",
        "https://ptb.discord.com/api/webhooks/",
        "https://canary.discord.com/api/webhooks/",
        "https://discordapp.com/api/webhooks/",
    ]
    .iter()
    .find_map(|p| u.strip_prefix(p));
    let Some(rest) = rest else { return false };
    let mut parts = rest.split('/');
    let id = parts.next().unwrap_or("");
    let token = parts.next().unwrap_or("");
    (17..=20).contains(&id.len())
        && id.chars().all(|c| c.is_ascii_digit())
        && (20..=100).contains(&token.len())
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && parts.next().is_none_or(|x| x.is_empty())
}

/// JSON payload for Discord (embed + optional mention + attachment ref).
pub fn payload(m: &Message, ping_user: &str, has_image: bool) -> serde_json::Value {
    let mention = if m.ping
        && !ping_user.trim().is_empty()
        && ping_user.trim().chars().all(|c| c.is_ascii_digit())
    {
        format!("<@{}>", ping_user.trim())
    } else {
        String::new()
    };
    let mut embed = serde_json::json!({
        "title": m.title,
        "description": m.body,
        "color": m.kind.color(),
        "footer": {"text": format!("{} {} · {}", crate::gui::APP_NAME, crate::gui::APP_BY, crate::update::version_label())},
    });
    if has_image {
        embed["image"] = serde_json::json!({"url": "attachment://screenshot.png"});
    }
    serde_json::json!({
        "username": "Anime Adventure Macro",
        "content": mention,
        "embeds": [embed],
        "allowed_mentions": {"users": if mention.is_empty() { vec![] } else { vec![ping_user.trim().to_string()] }},
    })
}

fn png(frame: &Frame) -> Result<Vec<u8>> {
    let mut rgba = frame.bgra.clone();
    for p in rgba.chunks_exact_mut(4) {
        p.swap(0, 2);
        p[3] = 255;
    }
    let mut out = std::io::Cursor::new(Vec::new());
    image::write_buffer_with_format(
        &mut out,
        &rgba,
        frame.width,
        frame.height,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )?;
    Ok(out.into_inner())
}

/// Send one message now (blocking). Used by the background thread and Test.
pub fn send(url: &str, ping_user: &str, m: &Message) -> Result<()> {
    if !valid_url(url) {
        bail!("that isn't a Discord webhook URL (https://discord.com/api/webhooks/...)");
    }
    let image = m.screenshot.as_ref().map(png).transpose()?;
    let body = payload(m, ping_user, image.is_some()).to_string();
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .new_agent();
    let resp = match image {
        None => agent.post(url.trim()).header("Content-Type", "application/json").send(body.as_bytes()),
        Some(img) => {
            let boundary = "----aamacro7d3f1c";
            let mut data = Vec::new();
            data.extend_from_slice(
                format!("--{boundary}\r\nContent-Disposition: form-data; name=\"payload_json\"\r\nContent-Type: application/json\r\n\r\n{body}\r\n").as_bytes(),
            );
            data.extend_from_slice(
                format!("--{boundary}\r\nContent-Disposition: form-data; name=\"files[0]\"; filename=\"screenshot.png\"\r\nContent-Type: image/png\r\n\r\n").as_bytes(),
            );
            data.extend_from_slice(&img);
            data.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
            agent.post(url.trim()).header("Content-Type", &format!("multipart/form-data; boundary={boundary}")).send(&data[..])
        }
    }
    .map_err(|e| anyhow!("Discord unreachable: {e}"))?;
    let code = resp.status().as_u16();
    if !(200..300).contains(&code) {
        bail!("Discord refused the webhook (HTTP {code})");
    }
    Ok(())
}

/// Background sender. `notify` never blocks; when the queue is full the
/// message is dropped (and logged).
pub struct Notifier {
    tx: SyncSender<(String, String, Message)>,
}

impl Notifier {
    pub fn start() -> Self {
        let (tx, rx): (SyncSender<(String, String, Message)>, Receiver<_>) = mpsc::sync_channel(16);
        std::thread::Builder::new()
            .name("webhook".into())
            .spawn(move || {
                for (url, ping, m) in rx {
                    if let Err(e) = send(&url, &ping, &m) {
                        log::warn!("webhook: {e}");
                    }
                    std::thread::sleep(Duration::from_millis(700)); // stay under Discord's rate limit
                }
            })
            .expect("spawn webhook thread");
        Self { tx }
    }

    pub fn notify(&self, url: &str, ping_user: &str, m: Message) {
        if let Err(TrySendError::Full(_)) =
            self.tx
                .try_send((url.to_string(), ping_user.to_string(), m))
        {
            log::warn!("webhook queue full - message dropped");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_validation() {
        assert!(valid_url(
            "https://discord.com/api/webhooks/123456789012345678/abcDEF_ghi-jkl1234567890xyz"
        ));
        assert!(valid_url(
            " https://ptb.discord.com/api/webhooks/123456789012345678/abcDEF_ghi-jkl1234567890xyz "
        ));
        assert!(!valid_url(
            "https://evil.com/api/webhooks/123456789012345678/abcDEF_ghi-jkl1234567890xyz"
        ));
        assert!(!valid_url("https://discord.com/api/webhooks/12/abc"));
        assert!(!valid_url(
            "https://discord.com/api/webhooks/123456789012345678/abcDEF_ghi-jkl1234567890xyz/extra"
        ));
    }

    #[test]
    fn payload_mentions_only_digits() {
        let m = Message {
            kind: Kind::Win,
            title: "Victory".into(),
            body: "wave 30".into(),
            screenshot: None,
            ping: true,
        };
        let p = payload(&m, "123456789", true);
        assert_eq!(p["content"], "<@123456789>");
        assert_eq!(
            p["embeds"][0]["image"]["url"],
            "attachment://screenshot.png"
        );
        assert_eq!(p["embeds"][0]["color"], 0x4ADE80);
        let p2 = payload(&m, "@everyone", false);
        assert_eq!(p2["content"], "");
        assert!(p2["embeds"][0].get("image").is_none());
    }

    #[test]
    fn screenshot_encodes_as_png() {
        let b = png(&Frame::solid(4, 3, 1, 2, 3)).unwrap();
        assert_eq!(&b[..8], b"\x89PNG\r\n\x1a\n");
    }
}
