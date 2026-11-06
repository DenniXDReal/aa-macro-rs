//! Failure dumps: when a step fails (an `Err`, an OCR read below the bar, a
//! pattern that never showed with `dump_on_miss`, later nav-stuck), save what
//! every client looked like to `debug/<YYYYMMDD_HHMMSS>_<step>/`:
//!
//! - `client_<n>_frame.png`        the full client picture
//! - `client_<n>_crop_<what>.png`  the crop the failed read used
//! - `client_<n>_rect.json`        search rect, best score, pattern / OCR text
//! - `state.json`                  step, params, input generation, time in step
//! - `log.txt`                     the last log lines (per client)
//!
//! The same (step, reason) within `DUMP_COOLDOWN_SECS` only bumps a counter
//! in the existing dump's `state.json` (no disk-fill during a spin). The
//! newest `FAILURE_DUMP_RETENTION` dumps are kept, older ones pruned at start.

use crate::capture::Frame;
use crate::geom::Rect;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

pub const FAILURE_DUMP_RETENTION: usize = 50;
pub const DUMP_COOLDOWN_SECS: u64 = 5;
/// Log lines written to each dump.
pub const DUMP_LOG_LINES: usize = 200;

/// One read that failed on a client: what it looked at and what it got.
#[derive(Clone, Debug)]
pub struct Crop {
    /// File-name part: `client_<n>_crop_<what>.png`.
    pub what: String,
    pub rect: Rect,
    /// Pattern name / OCR text / best score - anything useful.
    pub info: serde_json::Value,
}

/// One client in a dump.
pub struct ClientShot {
    /// Client number (1 = main).
    pub n: usize,
    pub frame: Option<Frame>,
    pub crops: Vec<Crop>,
}

pub fn root() -> PathBuf {
    super::data_dir().join("debug")
}

static RECENT: Mutex<Option<HashMap<(String, String), (Instant, PathBuf)>>> = Mutex::new(None);

/// Save a dump (or bump the counter of a recent identical one). Returns
/// the folder. Never fails the caller: errors only mean no dump.
pub fn dump(step: &str, reason: &str, clients: Vec<ClientShot>) -> Option<PathBuf> {
    dump_in(&root(), step, reason, clients)
}

pub fn dump_in(root: &Path, step: &str, reason: &str, clients: Vec<ClientShot>) -> Option<PathBuf> {
    let key = (step.to_string(), short(reason, 80));
    {
        let mut g = RECENT.lock().unwrap_or_else(|e| e.into_inner());
        let m = g.get_or_insert_with(HashMap::new);
        if let Some((t, dir)) = m.get(&key) {
            if t.elapsed().as_secs() < DUMP_COOLDOWN_SECS && dir.exists() {
                bump(dir);
                let dir = dir.clone();
                m.insert(key, (Instant::now(), dir.clone()));
                return Some(dir);
            }
        }
    }
    let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let mut dir = root.join(format!("{stamp}_{}", safe(step)));
    let mut k = 2;
    while dir.exists() {
        dir = root.join(format!("{stamp}_{}_{k}", safe(step)));
        k += 1;
    }
    std::fs::create_dir_all(&dir).ok()?;
    let now = super::trace::current();
    let state = serde_json::json!({
        "step": step,
        "reason": reason,
        "step_params": now.as_ref().map(|s| s.params.clone()),
        "step_now": now.as_ref().map(|s| s.name.clone()),
        "elapsed_in_step_ms": now.as_ref().map(|s| s.since.elapsed().as_millis() as u64),
        "generation": crate::input::generation_all()
            .iter()
            .map(|(h, g)| serde_json::json!([format!("{h:?}"), g]))
            .collect::<Vec<_>>(),
        "time": chrono::Local::now().to_rfc3339(),
        "repeat_count": 1,
    });
    let _ = std::fs::write(
        dir.join("state.json"),
        serde_json::to_string_pretty(&state).unwrap_or_default(),
    );
    let mut log = super::trace::last_lines(DUMP_LOG_LINES, None).join("\n");
    for c in &clients {
        if let Some(f) = &c.frame {
            let _ = f.save_png(&dir.join(format!("client_{}_frame.png", c.n)));
            for cr in &c.crops {
                if let Some(a) =
                    cr.rect
                        .intersect(&Rect::new(0, 0, f.width as i32, f.height as i32))
                {
                    let crop = f.crop(a.x as u32, a.y as u32, a.w as u32, a.h as u32);
                    let _ = crop.save_png(&dir.join(format!(
                        "client_{}_crop_{}.png",
                        c.n,
                        safe(&cr.what)
                    )));
                }
            }
        }
        let rects: Vec<serde_json::Value> = c
            .crops
            .iter()
            .map(|cr| {
                serde_json::json!({
                    "what": cr.what,
                    "rect": [cr.rect.x, cr.rect.y, cr.rect.w, cr.rect.h],
                    "info": cr.info,
                })
            })
            .collect();
        let _ = std::fs::write(
            dir.join(format!("client_{}_rect.json", c.n)),
            serde_json::to_string_pretty(&rects).unwrap_or_default(),
        );
        let lines = super::trace::last_lines(DUMP_LOG_LINES, Some(c.n));
        let _ = std::fs::write(
            dir.join(format!("client_{}_log.txt", c.n)),
            lines.join("\n"),
        );
    }
    if log.is_empty() {
        log = "(no log lines yet)".into();
    }
    let _ = std::fs::write(dir.join("log.txt"), log);
    RECENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(key, (Instant::now(), dir.clone()));
    Some(dir)
}

/// Same failure again soon: count it in the existing dump.
fn bump(dir: &Path) {
    let p = dir.join("state.json");
    let Ok(s) = std::fs::read_to_string(&p) else {
        return;
    };
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&s) else {
        return;
    };
    let n = v["repeat_count"].as_u64().unwrap_or(1) + 1;
    v["repeat_count"] = n.into();
    v["last_repeat"] = chrono::Local::now().to_rfc3339().into();
    let _ = std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap_or_default());
}

/// Keep the newest `keep` dumps (folder names start with the time stamp).
pub fn prune(root: &Path, keep: usize) -> usize {
    let Ok(rd) = std::fs::read_dir(root) else {
        return 0;
    };
    let mut dirs: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    let extra = dirs.len().saturating_sub(keep);
    let mut gone = 0;
    for d in dirs.into_iter().take(extra) {
        if std::fs::remove_dir_all(&d).is_ok() {
            gone += 1;
        }
    }
    gone
}

fn safe(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect()
}

fn short(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(n: usize) -> ClientShot {
        ClientShot {
            n,
            frame: Some(Frame::solid(40, 30, 10, 20, 30)),
            crops: vec![Crop {
                what: "upg_num".into(),
                rect: Rect::new(5, 5, 10, 8),
                info: serde_json::json!({"ocr": "UPG [?"}),
            }],
        }
    }

    #[test]
    fn writes_files_and_rate_limits() {
        let root = std::env::temp_dir().join(format!("aa-dump-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = dump_in(&root, "upgrade", "level unreadable", vec![shot(1), shot(2)]).unwrap();
        for f in [
            "state.json",
            "log.txt",
            "client_1_frame.png",
            "client_1_crop_upg_num.png",
            "client_2_rect.json",
        ] {
            assert!(d.join(f).exists(), "{f}");
        }
        // same failure again right away: same folder, counter 2
        let d2 = dump_in(&root, "upgrade", "level unreadable", vec![shot(1)]).unwrap();
        assert_eq!(d, d2);
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(d.join("state.json")).unwrap()).unwrap();
        assert_eq!(v["repeat_count"], 2);
        // another reason: a new folder
        let d3 = dump_in(&root, "upgrade", "something else", vec![shot(1)]).unwrap();
        assert_ne!(d, d3);
        assert_eq!(prune(&root, 1), 1);
        let _ = std::fs::remove_dir_all(&root);
    }
}
