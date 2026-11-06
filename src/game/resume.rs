//! Save / resume the current step. Every step change is written to
//! `runtime/last_state.json` (temp file + rename, so it's never half
//! written). When the macro starts again soon after (`RESUME_WINDOW_SECS`)
//! while a stage is still running, it picks up at that step - e.g. straight
//! back to upgrading the units it had placed - instead of voting, walking and
//! placing all over again. The same is used after a hiccup mid-stage.

use super::trace::StepNow;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const RESUME_WINDOW_SECS: u64 = 300;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Saved {
    pub step: String,
    pub step_params: serde_json::Value,
    pub client_states: Vec<serde_json::Value>,
    /// Unix seconds.
    pub updated_at: u64,
}

impl Saved {
    pub fn age_secs(&self) -> u64 {
        now_secs().saturating_sub(self.updated_at)
    }
    /// A step worth resuming (in a stage, past the walk).
    pub fn resumable(&self) -> bool {
        matches!(self.step.as_str(), "place" | "upgrade")
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn path() -> PathBuf {
    super::data_dir().join("runtime").join("last_state.json")
}

/// Write `s` atomically.
pub fn save_to(p: &Path, s: &Saved) -> std::io::Result<()> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(s).unwrap_or_default())?;
    std::fs::rename(&tmp, p)
}

pub fn load_from(p: &Path) -> Option<Saved> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

/// The saved state if it's recent enough to resume from.
pub fn recent() -> Option<Saved> {
    load_from(&path()).filter(|s| s.age_secs() <= RESUME_WINDOW_SECS && s.resumable())
}

/// Forget it (a fresh start, or the round finished).
pub fn clear() {
    let _ = std::fs::remove_file(path());
}

/// Hooked into `trace::step`: persist every step change.
fn on_step(s: &StepNow) {
    let clients = crate::input::generation_all()
        .iter()
        .enumerate()
        .map(|(i, (_, g))| serde_json::json!({ "n": i + 1, "generation": g }))
        .collect();
    let saved = Saved {
        step: s.name.clone(),
        step_params: s.params.clone(),
        client_states: clients,
        updated_at: now_secs(),
    };
    if let Err(e) = save_to(&path(), &saved) {
        log::warn!("last_state.json not saved: {e}");
    }
}

/// Start saving step changes (once per app).
pub fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| super::trace::on_step(on_step));
}

/// Placed units from the "upgrade" step's params: (client number, slot, x, y).
pub fn placed(s: &Saved) -> Vec<(usize, usize, i32, i32)> {
    s.step_params["placed"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| {
                    let v = v.as_array()?;
                    Some((
                        v.first()?.as_u64()? as usize,
                        v.get(1)?.as_u64()? as usize,
                        v.get(2)?.as_i64()? as i32,
                        v.get(3)?.as_i64()? as i32,
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_window() {
        let p = std::env::temp_dir().join(format!("aa-resume-{}.json", std::process::id()));
        let s = Saved {
            step: "upgrade".into(),
            step_params: serde_json::json!({ "map": "Rain Village", "placed": [[1, 2, 300, 250], [2, 1, 410, 260]] }),
            client_states: vec![],
            updated_at: now_secs() - 10,
        };
        save_to(&p, &s).unwrap();
        let back = load_from(&p).unwrap();
        assert_eq!(back, s);
        assert!(back.resumable() && back.age_secs() < RESUME_WINDOW_SECS);
        assert_eq!(placed(&back), vec![(1, 2, 300, 250), (2, 1, 410, 260)]);
        let old = Saved {
            updated_at: now_secs() - RESUME_WINDOW_SECS - 5,
            ..s
        };
        assert!(old.age_secs() > RESUME_WINDOW_SECS);
        assert!(!Saved {
            step: "lobby".into(),
            ..back
        }
        .resumable());
        let _ = std::fs::remove_file(&p);
    }
}
