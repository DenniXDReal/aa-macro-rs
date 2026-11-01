//! Movement routes: per-map walks after loading in, and lobby walks to each
//! mode's entrance. Same JSON shape as Spiral's routes, so they load as-is:
//!
//! `{"map": "...", "steps": [{"type":"key_down","key":"w"}, {"type":"wait","ms":500}, ...]}`
//!
//! Built-in routes are baked into the exe; your own edits in `routes/` next to
//! the exe win over them.

use crate::geom::Point;
use crate::input::{Action, Batch, Priority};
use crate::keys::Key;
use crate::platform::MouseButton;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod gen {
    include!(concat!(env!("OUT_DIR"), "/assets_gen.rs"));
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Step {
    KeyDown {
        key: String,
    },
    KeyUp {
        key: String,
    },
    Wait {
        ms: u64,
    },
    /// Hold several keys together for `ms`.
    Hold {
        keys: Vec<String>,
        ms: u64,
    },
    Click {
        x: i32,
        y: i32,
        #[serde(default = "left")]
        button: String,
    },
    /// Branch on a pattern being visible in `region` [x1, y1, x2, y2].
    IfImage {
        patterns: Vec<String>,
        region: [i32; 4],
        #[serde(default)]
        err1: f32,
        #[serde(default)]
        err0: f32,
        #[serde(default)]
        then: Vec<Step>,
        #[serde(default, rename = "else")]
        otherwise: Vec<Step>,
    },
}

fn left() -> String {
    "left".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Route {
    pub map: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub steps: Vec<Step>,
}

impl Route {
    pub fn parse(json: &str) -> Result<Route> {
        serde_json::from_str(json).context("bad route json")
    }

    /// Total time the route takes (taking the `then` branch of conditions).
    pub fn duration_ms(&self) -> u64 {
        fn d(s: &[Step]) -> u64 {
            s.iter()
                .map(|x| match x {
                    Step::Wait { ms } | Step::Hold { ms, .. } => *ms,
                    Step::Click { .. } => 100,
                    Step::IfImage { then, .. } => d(then),
                    _ => 0,
                })
                .sum()
        }
        d(&self.steps)
    }

    /// Flatten to arbiter actions. `pick` decides an `if_image` branch.
    pub fn to_actions(
        &self,
        pick: &mut dyn FnMut(&[String], [i32; 4]) -> bool,
    ) -> Result<Vec<Action>> {
        let mut out = vec![];
        flatten(&self.steps, pick, &mut out)?;
        Ok(out)
    }

    /// One input batch for `window` (movement must hold focus for its whole length).
    pub fn to_batch(
        &self,
        window: crate::geom::Hwnd,
        pick: &mut dyn FnMut(&[String], [i32; 4]) -> bool,
    ) -> Result<Batch> {
        let mut b = Batch::new(window, format!("move: {}", self.map)).priority(Priority::Normal);
        b.actions = self.to_actions(pick)?;
        Ok(b)
    }
}

fn key(name: &str) -> Result<Key> {
    Key::parse(name).ok_or_else(|| anyhow!("unknown key '{name}' in route"))
}

fn flatten(
    steps: &[Step],
    pick: &mut dyn FnMut(&[String], [i32; 4]) -> bool,
    out: &mut Vec<Action>,
) -> Result<()> {
    for s in steps {
        match s {
            Step::KeyDown { key: k } => out.push(Action::KeyDown(key(k)?)),
            Step::KeyUp { key: k } => out.push(Action::KeyUp(key(k)?)),
            Step::Wait { ms } => out.push(Action::Wait(*ms)),
            Step::Hold { keys, ms } => {
                let ks = keys.iter().map(|k| key(k)).collect::<Result<Vec<_>>>()?;
                out.extend(ks.iter().map(|k| Action::KeyDown(*k)));
                out.push(Action::Wait(*ms));
                out.extend(ks.iter().map(|k| Action::KeyUp(*k)));
            }
            Step::Click { x, y, button } => out.push(Action::Click {
                at: Point::new(*x, *y),
                button: if button.eq_ignore_ascii_case("right") {
                    MouseButton::Right
                } else {
                    MouseButton::Left
                },
            }),
            Step::IfImage {
                patterns,
                region,
                then,
                otherwise,
                ..
            } => {
                let branch = if pick(patterns, *region) {
                    then
                } else {
                    otherwise
                };
                flatten(branch, pick, out)?;
            }
        }
    }
    Ok(())
}

/// Built-in + user routes.
pub struct Routes {
    pub maps: BTreeMap<String, Route>,
    pub lobby: BTreeMap<String, Route>,
    pub user_dir: PathBuf,
    /// Map names whose route came from the user folder.
    pub user_maps: Vec<String>,
}

impl Routes {
    pub fn user_dir_default() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("routes")))
            .unwrap_or_else(|| PathBuf::from("routes"))
    }

    pub fn load(user_dir: &Path) -> Self {
        let mut maps = BTreeMap::new();
        for (stem, json) in gen::ROUTES {
            match Route::parse(json) {
                Ok(r) => {
                    maps.insert(stem.to_string(), r);
                }
                Err(e) => log::error!("built-in route {stem}: {e}"),
            }
        }
        let mut lobby = BTreeMap::new();
        for (stem, json) in gen::LOBBY {
            if let Ok(r) = Route::parse(json) {
                lobby.insert(stem.to_string(), r);
            }
        }
        let mut user_maps = vec![];
        if let Ok(rd) = std::fs::read_dir(user_dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }
                let stem = p.file_stem().unwrap().to_string_lossy().to_string();
                match std::fs::read_to_string(&p)
                    .map_err(anyhow::Error::from)
                    .and_then(|t| Route::parse(&t))
                {
                    Ok(r) => {
                        if let Some(l) = stem.strip_prefix("lobby_") {
                            lobby.insert(l.to_string(), r);
                        } else {
                            maps.insert(stem.clone(), r);
                        }
                        user_maps.push(stem);
                    }
                    Err(e) => log::error!("route {}: {e}", p.display()),
                }
            }
        }
        Routes {
            maps,
            lobby,
            user_dir: user_dir.to_path_buf(),
            user_maps,
        }
    }

    pub fn save_user(&mut self, name: &str, r: &Route) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.user_dir)?;
        let p = self.user_dir.join(format!("{name}.json"));
        std::fs::write(&p, serde_json::to_string_pretty(r)?)?;
        if !self.user_maps.iter().any(|m| m == name) {
            self.user_maps.push(name.to_string());
        }
        Ok(p)
    }
}

/// Story / Legend / Raid map + act click positions (client coords, scroll notches).
#[derive(Clone, Debug, Deserialize)]
pub struct StagePick {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub scrolls: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ModeStages {
    pub maps: Vec<StagePick>,
    pub acts: Vec<StagePick>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Stages {
    pub modes: BTreeMap<String, ModeStages>,
}

impl Stages {
    pub fn builtin() -> Stages {
        serde_json::from_str(gen::STAGES).expect("assets/stages.json is valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_route_parses_and_converts() {
        let r = Routes::load(Path::new("/nonexistent"));
        assert!(r.maps.len() >= 30, "only {} routes", r.maps.len());
        assert!(!r.maps.contains_key("Contracts"));
        assert!(r.maps.contains_key("Space Center") && r.maps.contains_key("Adventurer Town"));
        for (name, route) in &r.maps {
            route
                .to_actions(&mut |_, _| true)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            route
                .to_actions(&mut |_, _| false)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        for m in ["summer", "story", "legend", "raid", "castle", "challenge"] {
            assert!(r.lobby.contains_key(m), "lobby {m}");
        }
    }

    #[test]
    fn hold_and_branches() {
        let r = Route::parse(
            r#"{"map":"t","steps":[{"type":"hold","keys":["w","shift"],"ms":100},
            {"type":"if_image","patterns":["x"],"region":[0,0,10,10],"then":[{"type":"wait","ms":1}],"else":[{"type":"wait","ms":2}]},
            {"type":"click","x":5,"y":6}]}"#,
        )
        .unwrap();
        let a = r.to_actions(&mut |_, _| false).unwrap();
        let w = Key::parse("w").unwrap();
        assert_eq!(a[0], Action::KeyDown(w));
        assert_eq!(a[2], Action::Wait(100));
        assert_eq!(a[5], Action::Wait(2));
        assert!(matches!(
            a[6],
            Action::Click {
                at: Point { x: 5, y: 6 },
                button: MouseButton::Left
            }
        ));
        assert_eq!(r.duration_ms(), 100 + 1 + 100);
    }

    #[test]
    fn stage_tables_load() {
        let s = Stages::builtin();
        assert!(s.modes["story"]
            .maps
            .iter()
            .any(|m| m.name == "Space Center"));
        assert_eq!(s.modes["raid"].acts.len(), 5);
    }

    #[test]
    fn user_route_overrides_builtin() {
        let dir = std::env::temp_dir().join(format!("aa-routes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut r = Routes::load(&dir);
        let mine = Route {
            map: "Space Center".into(),
            source: "user".into(),
            steps: vec![Step::Wait { ms: 1 }],
        };
        r.save_user("Space Center", &mine).unwrap();
        r.save_user("lobby_summer", &mine).unwrap();
        let again = Routes::load(&dir);
        assert_eq!(again.maps["Space Center"], mine);
        assert_eq!(again.lobby["summer"], mine);
        std::fs::remove_dir_all(&dir).ok();
    }
}
