//! Roblox Account Manager (ic3w0lf22) integration through its local web API.
//!
//! The macro never sees or stores cookies: RAM keeps them, the macro only asks
//! RAM to launch an account into your private server. Needs, in RAM's
//! settings: Developer → Enable Web Server, Web Server → Allow LaunchAccount +
//! Allow GetAccounts, and a password of 6+ characters. Turn on RAM's own
//! "Multi Roblox" (General tab) so several clients can run.

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Anime Adventures place id.
pub const AA_PLACE_ID: u64 = 8304191830;
pub const PASSWORD_KEY: &str = "ram_password";
pub const PRIVATE_SERVER_KEY: &str = "private_server_link";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountRole {
    Main,
    Alt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSlot {
    /// RAM username.
    pub username: String,
    pub role: AccountRole,
    pub enabled: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct V2 {
    success: bool,
    message: String,
}

pub struct Ram {
    base: String,
    password: String,
    agent: ureq::Agent,
}

impl Ram {
    pub fn new(port: u16, password: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        Self {
            base: format!("http://127.0.0.1:{port}"),
            password: password.to_string(),
            agent,
        }
    }

    fn call(&self, method: &str, params: &[(&str, &str)]) -> Result<String> {
        let mut req = self
            .agent
            .get(&format!("{}/v2/{method}", self.base))
            .query("Password", &self.password);
        for (k, v) in params {
            req = req.query(*k, *v);
        }
        let mut resp = req.call().map_err(|e| {
            anyhow!("Account Manager not reachable ({e}) - is RAM open with its web server on?")
        })?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_to_string()?;
        let parsed: V2 = serde_json::from_str(&body)
            .map_err(|_| anyhow!("unexpected reply from RAM ({status}): {body}"))?;
        if !parsed.success {
            bail!("{}", explain(&parsed.message));
        }
        Ok(parsed.message)
    }

    /// True when RAM's web server answers.
    pub fn running(&self) -> bool {
        self.agent
            .get(&format!("{}/Running", self.base))
            .call()
            .map(|mut r| {
                r.body_mut()
                    .read_to_string()
                    .map(|t| t.contains("true") || t.contains("running"))
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    }

    /// Usernames saved in RAM.
    pub fn accounts(&self) -> Result<Vec<String>> {
        let m = self.call("GetAccounts", &[])?;
        Ok(m.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect())
    }

    /// Launch `account` into Anime Adventures, into the private server when
    /// `private_server` (a roblox.com share/VIP link) is given.
    pub fn launch(&self, account: &str, private_server: Option<&str>) -> Result<String> {
        let place = AA_PLACE_ID.to_string();
        let mut p = vec![("Account", account), ("PlaceId", place.as_str())];
        if let Some(link) = private_server.filter(|l| !l.trim().is_empty()) {
            p.push(("JobId", link.trim()));
            p.push(("JoinVIP", "true"));
        }
        self.call("LaunchAccount", &p)
    }
}

fn explain(msg: &str) -> String {
    if msg.contains("Invalid Password") {
        "RAM rejected the password - set the same 6+ character password in RAM (Settings > Web Server) and here".into()
    } else if msg.contains("not allowed") {
        format!("{msg} - tick it in RAM: Settings > Web Server")
    } else if msg.contains("Invalid Account") {
        format!("{msg} - log that account in again inside RAM")
    } else {
        msg.to_string()
    }
}

/// Order to launch: the main first, then alts, skipping disabled ones.
pub fn launch_order(slots: &[AccountSlot]) -> Vec<&AccountSlot> {
    let mut v: Vec<&AccountSlot> = slots
        .iter()
        .filter(|s| s.enabled && !s.username.trim().is_empty())
        .collect();
    v.sort_by_key(|s| s.role != AccountRole::Main);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    /// Tiny fake RAM: answers each request with a canned body, returns the
    /// request lines it saw.
    fn fake_ram(replies: Vec<&'static str>) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let mut seen = vec![];
            for body in replies {
                let (mut s, _) = l.accept().unwrap();
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                seen.push(line.trim().to_string());
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h == "\r\n" || h.is_empty() {
                        break;
                    }
                }
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                s.write_all(resp.as_bytes()).unwrap();
            }
            seen
        });
        (port, h)
    }

    #[test]
    fn lists_and_launches() {
        let (port, h) = fake_ram(vec![
            r#"{"Success":true,"Message":"MainAcc,Alt1,Alt2"}"#,
            r#"{"Success":true,"Message":"Launched Alt1 to 8304191830"}"#,
        ]);
        let ram = Ram::new(port, "secret1");
        assert_eq!(ram.accounts().unwrap(), vec!["MainAcc", "Alt1", "Alt2"]);
        ram.launch(
            "Alt1",
            Some("https://www.roblox.com/share?code=abc&type=Server"),
        )
        .unwrap();
        let seen = h.join().unwrap();
        assert!(seen[0].starts_with("GET /v2/GetAccounts?Password=secret1"));
        assert!(seen[1].contains("Account=Alt1"));
        assert!(seen[1].contains("PlaceId=8304191830"));
        assert!(seen[1].contains("JoinVIP=true"));
        assert!(seen[1].contains("JobId=https%3A%2F%2Fwww.roblox.com%2Fshare%3Fcode%3Dabc"));
    }

    #[test]
    fn errors_are_explained() {
        let (port, h) = fake_ram(vec![r#"{"Success":false,"Message":"Invalid Password"}"#]);
        let e = Ram::new(port, "x").accounts().unwrap_err().to_string();
        assert!(e.contains("6+ character password"), "{e}");
        h.join().unwrap();
        assert!(Ram::new(1, "x").accounts().is_err());
    }

    #[test]
    fn main_launches_first() {
        let s = vec![
            AccountSlot {
                username: "a1".into(),
                role: AccountRole::Alt,
                enabled: true,
            },
            AccountSlot {
                username: "off".into(),
                role: AccountRole::Alt,
                enabled: false,
            },
            AccountSlot {
                username: "m".into(),
                role: AccountRole::Main,
                enabled: true,
            },
        ];
        let o: Vec<&str> = launch_order(&s)
            .iter()
            .map(|x| x.username.as_str())
            .collect();
        assert_eq!(o, vec!["m", "a1"]);
    }
}
