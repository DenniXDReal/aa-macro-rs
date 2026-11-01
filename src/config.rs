//! Settings file (`aa-macro.json` next to the exe). Missing keys get defaults.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Roblox process to look for.
    pub process_name: String,
    /// Client size every pattern / spot is captured for.
    pub client_w: i32,
    pub client_h: i32,
    /// Max instances driven at once.
    pub max_instances: usize,
    /// Arbiter: warn when one input batch holds focus longer than this.
    pub max_lease_ms: u64,
    /// Arbiter: give up when focus can't be taken within this.
    pub focus_timeout_ms: u64,
    /// Delay after a cursor move before clicking (Roblox hover needs it).
    pub mouse_settle_ms: u64,
    /// How long a click holds the button.
    pub click_hold_ms: u64,
    /// Multi-instance unlock object names (held while `unlock` runs).
    pub singleton_names: Vec<String>,
    pub instances: Vec<InstanceProfile>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Drives its own farm.
    Independent,
    /// Party host: drives the stage, waits for followers in the lobby.
    Host,
    /// Party follower: only walks into the host's lobby, then idles.
    Follower,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct InstanceProfile {
    pub label: String,
    pub role: Role,
    pub enabled: bool,
}

impl Default for InstanceProfile {
    fn default() -> Self {
        Self {
            label: String::new(),
            role: Role::Independent,
            enabled: true,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            process_name: "RobloxPlayerBeta.exe".into(),
            client_w: 800,
            client_h: 599,
            max_instances: 4,
            max_lease_ms: 400,
            focus_timeout_ms: 600,
            mouse_settle_ms: 35,
            click_hold_ms: 40,
            singleton_names: vec!["ROBLOX_singletonMutex".into()],
            instances: (1..=4)
                .map(|i| InstanceProfile {
                    label: format!("Instance {i}"),
                    ..Default::default()
                })
                .collect(),
        }
    }
}

impl Settings {
    pub fn default_path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("aa-macro.json")))
            .unwrap_or_else(|| PathBuf::from("aa-macro.json"))
    }

    /// Load, or write defaults when the file doesn't exist.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if !path.exists() {
            let s = Settings::default();
            s.save(path)?;
            return Ok(s);
        }
        let txt =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&txt).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_json_gets_defaults() {
        let s: Settings = serde_json::from_str(r#"{"max_instances": 2}"#).unwrap();
        assert_eq!(s.max_instances, 2);
        assert_eq!(s.client_w, 800);
        assert_eq!(s.instances.len(), 4);
    }

    #[test]
    fn roundtrip_file() {
        let dir = std::env::temp_dir().join(format!("aa-macro-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("aa-macro.json");
        let s = Settings::load_or_create(&p).unwrap();
        assert!(p.exists());
        let again = Settings::load_or_create(&p).unwrap();
        assert_eq!(s.process_name, again.process_name);
        std::fs::remove_dir_all(&dir).ok();
    }
}
