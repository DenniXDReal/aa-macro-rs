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
    /// Loot report after every run (sent by the game loop, which waits for
    /// the upload before Replay). Replaces the plain win / loss message.
    pub loot_report: bool,
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
            loot_report: true,
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
    /// Embed fields: (name, value, inline).
    pub fields: Vec<(String, String, bool)>,
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
    if !m.fields.is_empty() {
        embed["fields"] = m
            .fields
            .iter()
            .map(|(n, v, inline)| serde_json::json!({"name": n, "value": v, "inline": inline}))
            .collect();
    }
    let mut p = serde_json::json!({
        "username": "Anime Adventure Macro",
        "content": mention,
        "embeds": [embed],
        "allowed_mentions": {"users": if mention.is_empty() { vec![] } else { vec![ping_user.trim().to_string()] }},
    });
    if has_image {
        // tell Discord which uploaded file the embed's attachment:// means
        p["attachments"] = serde_json::json!([{"id": 0, "filename": "screenshot.png"}]);
    }
    p
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
        let mut resp = resp;
        let why = resp.body_mut().read_to_string().unwrap_or_default();
        let why: String = why.chars().take(200).collect();
        bail!("Discord refused the webhook (HTTP {code}) {why}");
    }
    Ok(())
}

/// `send`, and when that fails with a picture: once more without it.
/// Ok(note) = sent (note says the picture was dropped).
pub fn send_with_fallback(url: &str, ping_user: &str, m: &mut Message) -> Result<Option<String>> {
    let res = send(url, ping_user, m);
    match res {
        Ok(()) => Ok(None),
        Err(e) if m.screenshot.is_some() => {
            m.screenshot = None;
            send(url, ping_user, m)?;
            Ok(Some(format!("sent without the picture ({e})")))
        }
        Err(e) => Err(e),
    }
}

/// Send on a background thread and wait for it, at most `max`: the caller
/// knows the upload finished (or gave up) before it moves on, and a hung
/// Discord can't hold it longer than `max`.
pub fn send_and_wait(
    url: &str,
    ping_user: &str,
    m: Message,
    max: Duration,
) -> Result<Option<String>> {
    let (tx, rx) = mpsc::channel();
    let (url, ping) = (url.to_string(), ping_user.to_string());
    std::thread::Builder::new()
        .name("webhook-now".into())
        .spawn(move || {
            let mut m = m;
            let _ = tx.send(send_with_fallback(&url, &ping, &mut m).map_err(|e| e.to_string()));
        })
        .map_err(|e| anyhow!("webhook thread: {e}"))?;
    match rx.recv_timeout(max) {
        Ok(r) => r.map_err(|e| anyhow!(e)),
        Err(_) => bail!(
            "Discord didn't answer within {} s - moving on (it may still arrive)",
            max.as_secs()
        ),
    }
}

/// Background sender. `notify` never blocks; when the queue is full the
/// message is dropped (and logged).
pub struct Notifier {
    tx: SyncSender<(String, String, Message)>,
}

impl Notifier {
    /// `report` gets a line for every message (sent / failed) for the app's log.
    pub fn start(report: Option<mpsc::Sender<String>>) -> Self {
        let (tx, rx): (SyncSender<(String, String, Message)>, Receiver<_>) = mpsc::sync_channel(16);
        std::thread::Builder::new()
            .name("webhook".into())
            .spawn(move || {
                let say = |s: String| {
                    log::info!("{s}");
                    if let Some(r) = &report {
                        let _ = r.send(s);
                    }
                };
                for (url, ping, mut m) in rx {
                    // the picture is the usual trouble maker: retried without it
                    match send_with_fallback(&url, &ping, &mut m) {
                        Ok(None) => say(format!("Discord: '{}' sent", m.title)),
                        Ok(Some(note)) => say(format!("Discord: '{}' {note}", m.title)),
                        Err(e) => say(format!("Discord: '{}' failed - {e}", m.title)),
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
            fields: vec![("Pearls".into(), "15".into(), true)],
        };
        let p = payload(&m, "123456789", true);
        assert_eq!(p["content"], "<@123456789>");
        assert_eq!(
            p["embeds"][0]["image"]["url"],
            "attachment://screenshot.png"
        );
        assert_eq!(p["embeds"][0]["color"], 0x4ADE80);
        assert_eq!(p["attachments"][0]["filename"], "screenshot.png");
        assert_eq!(p["embeds"][0]["fields"][0]["name"], "Pearls");
        assert_eq!(p["embeds"][0]["fields"][0]["inline"], true);
        let p2 = payload(&m, "@everyone", false);
        assert_eq!(p2["content"], "");
        assert!(p2["embeds"][0].get("image").is_none());
        assert!(p2.get("attachments").is_none());
    }

    #[test]
    fn screenshot_encodes_as_png() {
        let b = png(&Frame::solid(4, 3, 1, 2, 3)).unwrap();
        assert_eq!(&b[..8], b"\x89PNG\r\n\x1a\n");
    }
}
