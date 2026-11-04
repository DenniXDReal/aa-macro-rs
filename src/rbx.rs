//! Built-in account manager: launch Roblox accounts straight from the macro,
//! no Roblox Account Manager needed. Same calls RAM makes:
//!
//! 1. `.ROBLOSECURITY` cookie -> CSRF token -> one-time auth ticket
//!    (auth.roblox.com/v1/authentication-ticket)
//! 2. `roblox-player:` launch link with the ticket + PlaceLauncher URL
//!    (normal game, private server, or follow a user).
//!
//! Cookies are kept encrypted with Windows DPAPI (secrets.dat, only this
//! Windows user can read them) and are only ever sent to *.roblox.com over
//! HTTPS. Never logged.

use anyhow::{anyhow, bail, Context, Result};
use std::time::Duration;

pub fn cookie_key(username: &str) -> String {
    format!("cookie:{}", username.trim().to_lowercase())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .max_redirects(0)
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .new_agent()
}

/// Accept the raw value or a full "_|WARNING...|_..." / ".ROBLOSECURITY=..." paste.
pub fn clean_cookie(s: &str) -> String {
    let s = s.trim().trim_matches('"');
    let s = s.strip_prefix(".ROBLOSECURITY=").unwrap_or(s);
    s.split(';').next().unwrap_or("").trim().to_string()
}

fn cookie_header(cookie: &str) -> String {
    format!(".ROBLOSECURITY={}", clean_cookie(cookie))
}

/// Who this cookie belongs to: (user id, username).
pub fn whoami(cookie: &str) -> Result<(u64, String)> {
    let mut r = agent()
        .get("https://users.roblox.com/v1/users/authenticated")
        .header("Cookie", &cookie_header(cookie))
        .call()
        .map_err(|e| anyhow!("Roblox unreachable: {e}"))?;
    let code = r.status().as_u16();
    let body = r.body_mut().read_to_string().unwrap_or_default();
    if code == 401 {
        bail!("that cookie is expired or wrong - log in again and copy a fresh one");
    }
    if code != 200 {
        bail!("Roblox said HTTP {code}");
    }
    let v: serde_json::Value = serde_json::from_str(&body)?;
    let id = v["id"]
        .as_u64()
        .ok_or_else(|| anyhow!("no user id in the reply"))?;
    let name = v["name"].as_str().unwrap_or_default().to_string();
    Ok((id, name))
}

/// The CSRF token Roblox wants on POSTs (it answers 403 + the token).
fn csrf(cookie: &str) -> Result<String> {
    let r = agent()
        .post("https://auth.roblox.com/v1/authentication-ticket/")
        .header("Cookie", &cookie_header(cookie))
        .header("Referer", "https://www.roblox.com/")
        .header("Content-Type", "application/json")
        .send("{}".as_bytes())
        .map_err(|e| anyhow!("Roblox unreachable: {e}"))?;
    r.headers()
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            anyhow!(
                "no CSRF token (HTTP {}) - is the cookie still valid?",
                r.status().as_u16()
            )
        })
}

/// Where an account is right now (Roblox presence).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    Offline,
    /// On the website / app, not in a game.
    Online,
    /// In a game (`place` when Roblox shows it).
    InGame {
        place: Option<u64>,
    },
    Studio,
    Unknown,
}

impl Presence {
    pub fn label(self) -> &'static str {
        match self {
            Presence::Offline => "offline",
            Presence::Online => "online, not in a game",
            Presence::InGame { .. } => "in game",
            Presence::Studio => "in Studio",
            Presence::Unknown => "unknown",
        }
    }
}

/// Presence of `user_id`, asked with its own cookie (so the place shows).
pub fn presence(cookie: &str, user_id: u64) -> Result<Presence> {
    let token = csrf(cookie)?;
    let mut r = agent()
        .post("https://presence.roblox.com/v1/presence/users")
        .header("Cookie", &cookie_header(cookie))
        .header("x-csrf-token", &token)
        .header("Content-Type", "application/json")
        .send(format!("{{\"userIds\":[{user_id}]}}").as_bytes())
        .map_err(|e| anyhow!("Roblox unreachable: {e}"))?;
    let code = r.status().as_u16();
    let body = r.body_mut().read_to_string().unwrap_or_default();
    if code != 200 {
        bail!("presence: HTTP {code}");
    }
    Ok(parse_presence(&body))
}

pub fn parse_presence(body: &str) -> Presence {
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let p = &v["userPresences"][0];
    match p["userPresenceType"].as_u64() {
        Some(0) => Presence::Offline,
        Some(1) => Presence::Online,
        Some(2) => Presence::InGame {
            place: p["rootPlaceId"].as_u64().or_else(|| p["placeId"].as_u64()),
        },
        Some(3) => Presence::Studio,
        _ => Presence::Unknown,
    }
}

/// One-time ticket that logs the Roblox client in.
pub fn auth_ticket(cookie: &str) -> Result<String> {
    let token = csrf(cookie)?;
    let mut r = agent()
        .post("https://auth.roblox.com/v1/authentication-ticket/")
        .header("Cookie", &cookie_header(cookie))
        .header("X-CSRF-TOKEN", &token)
        .header("Referer", "https://www.roblox.com/")
        .header("RBXAuthenticationNegotiation", "1")
        .header("Content-Type", "application/json")
        .send("{}".as_bytes())
        .map_err(|e| anyhow!("Roblox unreachable: {e}"))?;
    if let Some(t) = r
        .headers()
        .get("rbx-authentication-ticket")
        .and_then(|v| v.to_str().ok())
    {
        return Ok(t.to_string());
    }
    let code = r.status().as_u16();
    let body: String = r
        .body_mut()
        .read_to_string()
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect();
    if code == 401 {
        bail!("Roblox says the login is expired (HTTP 401) - add the account again");
    }
    bail!("Roblox gave no login ticket (HTTP {code}) {body}")
}

/// User id for a username (for following the main).
pub fn user_id(username: &str) -> Result<u64> {
    let mut r = agent()
        .post("https://users.roblox.com/v1/usernames/users")
        .header("Content-Type", "application/json")
        .send(
            serde_json::json!({"usernames": [username], "excludeBannedUsers": true})
                .to_string()
                .as_bytes(),
        )
        .map_err(|e| anyhow!("Roblox unreachable: {e}"))?;
    let v: serde_json::Value = serde_json::from_str(&r.body_mut().read_to_string()?)?;
    v["data"][0]["id"]
        .as_u64()
        .ok_or_else(|| anyhow!("no Roblox user called {username}"))
}

/// Where to send the account.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A public server of the place.
    Game { place: u64 },
    /// A private server (from its link).
    Private {
        place: u64,
        access: String,
        link: String,
    },
    /// Whatever server this user id is in.
    Follow { user: u64 },
}

/// Pull the code out of a private server link: the new
/// `roblox.com/share?code=X&type=Server` kind or the old
/// `...?privateServerLinkCode=X` kind.
pub fn link_code(link: &str) -> Option<(bool, String)> {
    let l = link.trim();
    let grab = |key: &str| {
        l.split(['?', '&'])
            .find_map(|kv| kv.strip_prefix(key))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    if let Some(c) = grab("privateServerLinkCode=") {
        return Some((false, c));
    }
    if l.contains("/share") {
        return grab("code=").map(|c| (true, c));
    }
    None
}

/// Turn a private server link into a launch target.
pub fn private_target(cookie: &str, place: u64, link: &str) -> Result<Target> {
    let (share, code) =
        link_code(link).ok_or_else(|| anyhow!("that doesn't look like a private server link"))?;
    if share {
        // new share link: ask Roblox what it points to
        let token = csrf(cookie)?;
        let mut r = agent()
            .post("https://apis.roblox.com/sharelinks/v1/resolve-link")
            .header("Cookie", &cookie_header(cookie))
            .header("X-CSRF-TOKEN", &token)
            .header("Content-Type", "application/json")
            .header("Referer", "https://www.roblox.com/")
            .send(
                serde_json::json!({"linkId": code, "linkType": "Server"})
                    .to_string()
                    .as_bytes(),
            )
            .map_err(|e| anyhow!("Roblox unreachable: {e}"))?;
        let body = r.body_mut().read_to_string().unwrap_or_default();
        let v: serde_json::Value =
            serde_json::from_str(&body).context("resolving the share link")?;
        let d = &v["privateServerInviteData"];
        let link_code = d["linkCode"].as_str().ok_or_else(|| {
            anyhow!(
                "Roblox didn't accept that share link: {}",
                body.chars().take(160).collect::<String>()
            )
        })?;
        let place = d["placeId"].as_u64().unwrap_or(place);
        let access = access_code(cookie, place, link_code);
        return Ok(Target::Private {
            place,
            access,
            link: link_code.to_string(),
        });
    }
    // old link: the game page carries the access code
    let access = access_code(cookie, place, &code);
    Ok(Target::Private {
        place,
        access,
        link: code,
    })
}

/// The private server's access code from its game page (empty if the page
/// doesn't show it - Roblox then goes by the link code alone).
fn access_code(cookie: &str, place: u64, link_code: &str) -> String {
    let Ok(mut r) = agent()
        .get(&format!(
            "https://www.roblox.com/games/{place}?privateServerLinkCode={link_code}"
        ))
        .header("Cookie", &cookie_header(cookie))
        .call()
    else {
        return String::new();
    };
    let page = r.body_mut().read_to_string().unwrap_or_default();
    page.find("joinPrivateGame(")
        .and_then(|i| page[i..].split('\'').nth(1))
        .filter(|c| c.len() >= 30 && c.chars().all(|ch| ch.is_ascii_hexdigit() || ch == '-'))
        .unwrap_or("")
        .to_string()
}

fn enc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                o.push(b as char)
            }
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

/// The `roblox-player:` link Roblox's launcher understands.
pub fn launch_uri(ticket: &str, target: &Target, tracker: &str, now_ms: u128) -> String {
    let place_launcher = match target {
        Target::Game { place } => format!(
            "https://assetgame.roblox.com/game/PlaceLauncher.ashx?request=RequestGame&browserTrackerId={tracker}&placeId={place}&isPlayTogetherGame=false"
        ),
        Target::Private { place, access, link } => format!(
            "https://assetgame.roblox.com/game/PlaceLauncher.ashx?request=RequestPrivateGame&placeId={place}&accessCode={access}&linkCode={link}"
        ),
        Target::Follow { user } => format!(
            "https://assetgame.roblox.com/game/PlaceLauncher.ashx?request=RequestFollowUser&userId={user}"
        ),
    };
    format!(
        "roblox-player:1+launchmode:play+gameinfo:{ticket}+launchtime:{now_ms}+placelauncherurl:{}+browsertrackerid:{tracker}+robloxLocale:en_us+gameLocale:en_us+channel:+LaunchExp:InApp",
        enc(&place_launcher)
    )
}

/// A stable random-looking browser tracker id per account (like RAM's).
pub fn tracker_for(user_id: u64) -> String {
    let mut x = user_id.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xA5A5_5A5A;
    x ^= x >> 29;
    format!("{}{}", 100_000 + x % 75_000, 100_000 + (x >> 20) % 800_000)
}

/// Log the account in and open Roblox at `target`.
pub fn launch(
    cookie: &str,
    user_id: u64,
    target: &Target,
    open: &dyn Fn(&str) -> Result<()>,
) -> Result<()> {
    let ticket = auth_ticket(cookie)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    open(&launch_uri(&ticket, target, &tracker_for(user_id), now)).context("starting Roblox")
}

/// Check a saved login end to end without starting Roblox: who it is, and
/// that Roblox hands out a login ticket for it.
pub fn check(cookie: &str) -> Result<String> {
    let (id, name) = whoami(cookie).context("reading the account")?;
    auth_ticket(cookie).context("getting a login ticket")?;
    Ok(format!("{name} (id {id}) - login works"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_kinds() {
        assert_eq!(
            parse_presence(
                r#"{"userPresences":[{"userPresenceType":2,"rootPlaceId":8304191830}]}"#
            ),
            Presence::InGame {
                place: Some(8304191830)
            }
        );
        assert_eq!(
            parse_presence(r#"{"userPresences":[{"userPresenceType":1}]}"#),
            Presence::Online
        );
        assert_eq!(parse_presence("junk"), Presence::Unknown);
    }

    #[test]
    fn cookies_and_links() {
        assert_eq!(
            clean_cookie(" .ROBLOSECURITY=_|WARNING|_abc; path=/ "),
            "_|WARNING|_abc"
        );
        assert_eq!(clean_cookie("\"_|WARNING|_xyz\""), "_|WARNING|_xyz");
        assert_eq!(
            link_code("https://www.roblox.com/share?code=7a1b2c&type=Server"),
            Some((true, "7a1b2c".into()))
        );
        assert_eq!(
            link_code("https://www.roblox.com/games/8304191830/Anime-Adventures?privateServerLinkCode=1234567890"),
            Some((false, "1234567890".into()))
        );
        assert_eq!(link_code("https://www.roblox.com/games/1"), None);
    }

    #[test]
    fn launch_link_shape() {
        let u = launch_uri(
            "TICKET",
            &Target::Private {
                place: 8304191830,
                access: "a-b".into(),
                link: "99".into(),
            },
            "123456789012",
            1700000000000,
        );
        assert!(u.starts_with("roblox-player:1+launchmode:play+gameinfo:TICKET+launchtime:1700000000000+placelauncherurl:https%3A%2F%2Fassetgame.roblox.com"));
        assert!(u.contains(
            "RequestPrivateGame%26placeId%3D8304191830%26accessCode%3Da-b%26linkCode%3D99"
        ));
        assert!(u.ends_with("+browsertrackerid:123456789012+robloxLocale:en_us+gameLocale:en_us+channel:+LaunchExp:InApp"));
        let f = launch_uri("T", &Target::Follow { user: 42 }, "1", 0);
        assert!(f.contains("RequestFollowUser%26userId%3D42"));
        assert_eq!(tracker_for(5), tracker_for(5));
        assert_eq!(tracker_for(5).len(), 12);
    }
}
