//! Which Roblox account is a client? Each Roblox process writes a log file
//! when it starts (%LOCALAPPDATA%\Roblox\logs) and, once in a game, a
//! `userid:<id>` line. Match the log to the process by start time, read the
//! user id, then turn it into a username (RAM's list, else Roblox's API).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub fn logs_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Roblox").join("logs"))
}

/// Last user id mentioned in a log text.
pub fn user_id_in(text: &str) -> Option<u64> {
    let mut best: Option<(usize, u64)> = None;
    for pat in ["userid:", "\"userId\":", "userId="] {
        let mut from = 0;
        while let Some(i) = text[from..].find(pat) {
            let at = from + i + pat.len();
            let digits: String = text[at..]
                .chars()
                .skip_while(|c| *c == ' ')
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(id) = digits.parse::<u64>() {
                if id > 0 && best.is_none_or(|b| at > b.0) {
                    best = Some((at, id));
                }
            }
            from = at;
        }
    }
    best.map(|b| b.1)
}

/// The player log created closest after `started` (within 40 s).
pub fn log_for_start(dir: &Path, started: SystemTime) -> Option<PathBuf> {
    let mut best: Option<(Duration, PathBuf)> = None;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        let name = p.file_name()?.to_string_lossy().to_lowercase();
        if !name.contains("player") || !name.ends_with(".log") {
            continue;
        }
        let Ok(md) = e.metadata() else { continue };
        let Ok(created) = md.created().or_else(|_| md.modified()) else {
            continue;
        };
        // the log appears a moment after the process starts
        let gap = match created.duration_since(started) {
            Ok(d) => d,
            Err(e) if e.duration() < Duration::from_secs(2) => Duration::ZERO,
            Err(_) => continue,
        };
        if gap > Duration::from_secs(40) {
            continue;
        }
        if best.as_ref().is_none_or(|b| gap < b.0) {
            best = Some((gap, p));
        }
    }
    best.map(|b| b.1)
}

/// User id for a Roblox process that started at `started`.
pub fn user_id_for(started: SystemTime) -> Option<u64> {
    let dir = logs_dir()?;
    let log = log_for_start(&dir, started)?;
    // Roblox keeps the file open; std opens with shared read/write on Windows
    let bytes = std::fs::read(log).ok()?;
    user_id_in(&String::from_utf8_lossy(&bytes))
}

/// Username for a user id from Roblox's public API.
pub fn username_online(id: u64) -> Option<String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .build()
        .new_agent();
    let mut r = agent
        .get(&format!("https://users.roblox.com/v1/users/{id}"))
        .call()
        .ok()?;
    let v: serde_json::Value = serde_json::from_str(&r.body_mut().read_to_string().ok()?).ok()?;
    v.get("name")?.as_str().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_last_user_id() {
        let t = "2026-11-01T10:00:00.1Z,1.2,1a2c,6 [FLog::Output] ! Joining game 'abc' place 8304191830 at 1.2.3.4\n\
                 [FLog::GameJoinLoadTime] Report game_join_loadtime: placeid:8304191830, loadtime:9000, gameid:x, userid:123456, ...\n\
                 later teleport userid:7891011, placeid:1";
        assert_eq!(user_id_in(t), Some(7891011));
        assert_eq!(user_id_in("{\"userId\":42,\"x\":1}"), Some(42));
        assert_eq!(user_id_in("nothing here"), None);
    }

    #[test]
    fn matches_log_by_start_time() {
        let dir = std::env::temp_dir().join(format!("aa-logs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let before = SystemTime::now();
        std::fs::write(
            dir.join("0.1_20261101T100000Z_Player_ABCDE_last.log"),
            "userid:5",
        )
        .unwrap();
        std::fs::write(dir.join("0.1_Studio_last.log"), "userid:9").unwrap();
        let got = log_for_start(&dir, before).unwrap();
        assert!(got.to_string_lossy().contains("Player_ABCDE"));
        // a process that started long after the log can't be it
        assert!(log_for_start(&dir, before + Duration::from_secs(600)).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
