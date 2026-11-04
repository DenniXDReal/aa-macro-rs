//! Movement routes: per-map walks after loading in, and lobby walks to each
//! mode's entrance. Same JSON shape as Spiral's routes, so they load as-is:
//!
//! `{"map": "...", "steps": [{"type":"key_down","key":"w"}, {"type":"wait","ms":500}, ...]}`
//!
//! Built-in routes are baked into the exe; your own edits in `routes/` next to
//! the exe win over them.

use crate::geom::{Point, Rect};
use crate::input::{Action, Batch, Priority, Probe};
use crate::keys::Key;
use crate::platform::MouseButton;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) mod gen {
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
    /// Wait for a pattern, then click it (or `fallback` [x, y] when it never shows).
    ClickPattern {
        pattern: String,
        #[serde(default)]
        fallback: Option<[i32; 2]>,
        #[serde(default = "d_timeout")]
        timeout_ms: u64,
        #[serde(default = "left")]
        button: String,
        /// Click this far from the pattern's centre (e.g. a toggle beside a label).
        #[serde(default)]
        offset: [i32; 2],
        /// Click once, never again (Safe clicks re-click a button that is
        /// still there - wrong for a toggle like the settings cog, which
        /// stays on screen and would close again).
        #[serde(default)]
        once: bool,
    },
    /// Wait until one of `patterns` is on screen.
    WaitFor {
        patterns: Vec<String>,
        #[serde(default = "d_timeout")]
        timeout_ms: u64,
        #[serde(default = "yes")]
        required: bool,
    },
    /// Hold `keys` until one of `patterns` shows up (e.g. walk until 'Select World').
    HoldUntil {
        keys: Vec<String>,
        patterns: Vec<String>,
        #[serde(default = "d_long")]
        timeout_ms: u64,
    },
    /// Wheel at (x, y) - or at `anchor` pattern + `offset` - positive notches = up.
    Scroll {
        x: i32,
        y: i32,
        notches: i32,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default)]
        offset: [i32; 2],
    },
    /// Hover (x, y) - or `anchor` + `offset` - and scroll one notch at a time
    /// until `pattern` is visible.
    ScrollUntil {
        pattern: String,
        x: i32,
        y: i32,
        #[serde(default)]
        up: bool,
        #[serde(default = "d_max")]
        max: u32,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default)]
        offset: [i32; 2],
    },
    /// Branch on a pattern being visible right now (optionally only in
    /// `region` [x1, y1, x2, y2]).
    IfImage {
        patterns: Vec<String>,
        #[serde(default)]
        region: Option<[i32; 4]>,
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
fn d_timeout() -> u64 {
    5000
}
fn d_long() -> u64 {
    12000
}
fn d_max() -> u32 {
    25
}
fn yes() -> bool {
    true
}

/// Looks at the live client for the route's pattern steps.
pub trait RouteEnv {
    /// A probe that finds any of `patterns` (inside `region` when given).
    fn probe(&self, patterns: &[String], region: Option<Rect>) -> Probe;
    /// A fixed button's known spot (clicked straight away, no search).
    fn remembered(&self, _pattern: &str) -> Option<Point> {
        None
    }
}

/// Values for `{map}` / `{act}` in pattern names (filled as pattern keys:
/// "Space Center" -> world_space_center).
#[derive(Clone, Debug, Default)]
pub struct Vars {
    pub map: String,
    pub act: String,
    /// Extra pause after every click (menus need a moment to open).
    pub click_wait_ms: u64,
}

impl Vars {
    pub fn fill(&self, name: &str) -> String {
        name.replace("{map}", &crate::vision::key(&self.map))
            .replace("{act}", &crate::vision::key(&self.act))
    }
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

    /// Fixed time the route takes (waits + holds; pattern waits not counted).
    pub fn duration_ms(&self) -> u64 {
        fn d(s: &[Step]) -> u64 {
            s.iter()
                .map(|x| match x {
                    Step::Wait { ms } | Step::Hold { ms, .. } => *ms,
                    Step::Click { .. } | Step::ClickPattern { .. } => 100,
                    Step::IfImage { then, .. } => d(then),
                    _ => 0,
                })
                .sum()
        }
        d(&self.steps)
    }

    /// Every pattern name the route looks for (after `{map}`/`{act}`).
    pub fn patterns(&self, vars: &Vars) -> Vec<String> {
        fn walk(s: &[Step], v: &Vars, out: &mut Vec<String>) {
            for x in s {
                match x {
                    Step::ClickPattern { pattern, .. } => out.push(v.fill(pattern)),
                    Step::ScrollUntil {
                        pattern, anchor, ..
                    } => {
                        out.push(v.fill(pattern));
                        out.extend(anchor.iter().map(|a| v.fill(a)));
                    }
                    Step::Scroll {
                        anchor: Some(a), ..
                    } => out.push(v.fill(a)),
                    Step::WaitFor { patterns, .. } | Step::HoldUntil { patterns, .. } => {
                        out.extend(patterns.iter().map(|p| v.fill(p)))
                    }
                    Step::IfImage {
                        patterns,
                        then,
                        otherwise,
                        ..
                    } => {
                        out.extend(patterns.iter().map(|p| v.fill(p)));
                        walk(then, v, out);
                        walk(otherwise, v, out);
                    }
                    _ => {}
                }
            }
        }
        let mut out = vec![];
        walk(&self.steps, vars, &mut out);
        out.sort();
        out.dedup();
        out
    }

    pub fn to_actions(&self, env: &dyn RouteEnv, vars: &Vars) -> Result<Vec<Action>> {
        let mut out = vec![];
        flatten(&self.steps, env, vars, &mut out)?;
        Ok(out)
    }

    /// One input batch for `window` (movement holds focus for its whole length).
    pub fn to_batch(
        &self,
        window: crate::geom::Hwnd,
        env: &dyn RouteEnv,
        vars: &Vars,
    ) -> Result<Batch> {
        let mut b = Batch::new(window, format!("move: {}", self.map)).priority(Priority::Normal);
        b.long = true;
        b.actions = self.to_actions(env, vars)?;
        Ok(b)
    }
}

fn key(name: &str) -> Result<Key> {
    Key::parse(name).ok_or_else(|| anyhow!("unknown key '{name}' in route"))
}

fn button(b: &str) -> MouseButton {
    if b.eq_ignore_ascii_case("right") {
        MouseButton::Right
    } else {
        MouseButton::Left
    }
}

fn flatten(steps: &[Step], env: &dyn RouteEnv, v: &Vars, out: &mut Vec<Action>) -> Result<()> {
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
            Step::Click { x, y, button: b } => {
                out.push(Action::Click {
                    at: Point::new(*x, *y),
                    button: button(b),
                });
                if v.click_wait_ms > 0 {
                    out.push(Action::Wait(v.click_wait_ms));
                }
            }
            Step::ClickPattern {
                pattern,
                fallback,
                timeout_ms,
                button: b,
                offset,
                once,
            } => {
                // fixed button seen before: click its spot, no searching
                if *offset == [0, 0] {
                    if let Some(at) = env.remembered(&v.fill(pattern)) {
                        out.push(Action::Click {
                            at,
                            button: button(b),
                        });
                        if v.click_wait_ms > 0 {
                            out.push(Action::Wait(v.click_wait_ms));
                        }
                        continue;
                    }
                }
                let mut probe = env
                    .probe(&[v.fill(pattern)], None)
                    .shifted(offset[0], offset[1]);
                // toggles stay on screen after the click: never click twice
                let name = crate::vision::key(&v.fill(pattern));
                if *once || crate::game::buttonmem::is_toggle(&name) {
                    probe.stays = true;
                }
                out.push(Action::ClickWhen {
                    probe,
                    timeout_ms: *timeout_ms,
                    fallback: fallback.map(|[x, y]| Point::new(x, y)),
                    button: button(b),
                });
                if v.click_wait_ms > 0 {
                    out.push(Action::Wait(v.click_wait_ms));
                }
            }
            Step::WaitFor {
                patterns,
                timeout_ms,
                required,
            } => out.push(Action::WaitFor {
                probe: env.probe(
                    &patterns.iter().map(|p| v.fill(p)).collect::<Vec<_>>(),
                    None,
                ),
                timeout_ms: *timeout_ms,
                required: *required,
            }),
            Step::HoldUntil {
                keys,
                patterns,
                timeout_ms,
            } => {
                let ks = keys.iter().map(|k| key(k)).collect::<Result<Vec<_>>>()?;
                out.extend(ks.iter().map(|k| Action::KeyDown(*k)));
                out.push(Action::WaitFor {
                    probe: env.probe(
                        &patterns.iter().map(|p| v.fill(p)).collect::<Vec<_>>(),
                        None,
                    ),
                    timeout_ms: *timeout_ms,
                    required: true,
                });
                out.extend(ks.iter().map(|k| Action::KeyUp(*k)));
            }
            Step::Scroll {
                x,
                y,
                notches,
                anchor,
                offset,
            } => out.push(Action::ScrollUntil {
                probe: Probe::new("scroll", || None),
                at: Point::new(*x, *y),
                anchor: anchor.as_ref().map(|a| env.probe(&[v.fill(a)], None)),
                offset: Point::new(offset[0], offset[1]),
                up: *notches > 0,
                max: notches.unsigned_abs(),
                max_ok: true,
            }),
            Step::ScrollUntil {
                pattern,
                x,
                y,
                up,
                max,
                anchor,
                offset,
            } => out.push(Action::ScrollUntil {
                probe: env.probe(&[v.fill(pattern)], None),
                at: Point::new(*x, *y),
                anchor: anchor.as_ref().map(|a| env.probe(&[v.fill(a)], None)),
                offset: Point::new(offset[0], offset[1]),
                up: *up,
                max: *max,
                max_ok: false,
            }),
            Step::IfImage {
                patterns,
                region,
                then,
                otherwise,
                ..
            } => {
                let mut t = vec![];
                let mut o = vec![];
                flatten(then, env, v, &mut t)?;
                flatten(otherwise, env, v, &mut o)?;
                let region = region.map(|r| Rect::new(r[0], r[1], r[2] - r[0], r[3] - r[1]));
                out.push(Action::Branch {
                    probe: env.probe(
                        &patterns.iter().map(|p| v.fill(p)).collect::<Vec<_>>(),
                        region,
                    ),
                    then: t,
                    otherwise: o,
                });
            }
        }
    }
    Ok(())
}

/// Built-in + user routes.
#[derive(Clone)]
pub struct Routes {
    pub maps: BTreeMap<String, Route>,
    pub lobby: BTreeMap<String, Route>,
    pub user_dir: PathBuf,
    /// Map names whose route came from the user folder.
    pub user_maps: Vec<String>,
}

impl Routes {
    /// Route for a map, matching names loosely ("ruined city" = "Ruined City").
    pub fn find_map(&self, name: &str) -> Option<&Route> {
        let k = crate::vision::key(name);
        self.maps.get(name).or_else(|| {
            self.maps
                .iter()
                .find(|(m, _)| crate::vision::key(m) == k)
                .map(|(_, r)| r)
        })
    }

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
        assert!(r.maps.contains_key("Contracts"));
        assert!(r.maps.contains_key("Space Center") && r.maps.contains_key("Adventurer Town"));
        let env = FakeEnv(vec![]);
        let v = Vars {
            map: "Space Center".into(),
            act: "Act 1".into(),
            ..Default::default()
        };
        for (name, route) in r.maps.iter().chain(r.lobby.iter()) {
            route
                .to_actions(&env, &v)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        for m in ["summer", "story", "legend", "raid", "castle", "challenge"] {
            assert!(r.lobby.contains_key(m), "lobby {m}");
        }
    }

    #[test]
    fn settings_cog_is_clicked_once() {
        let r = Routes::load(Path::new("/nonexistent"));
        let a = r.lobby["tp_spawn"]
            .to_actions(&FakeEnv(vec![]), &Vars::default())
            .unwrap();
        let first = a
            .iter()
            .find_map(|x| match x {
                Action::ClickWhen { probe, .. } => Some(probe.clone()),
                _ => None,
            })
            .unwrap();
        assert!(first.label.contains("settings_button"));
        assert!(first.stays, "Safe click must not click the cog again");
    }

    #[test]
    fn hold_and_branches() {
        let r = Route::parse(
            r#"{"map":"t","steps":[{"type":"hold","keys":["w","shift"],"ms":100},
            {"type":"if_image","patterns":["x"],"region":[0,0,10,10],"then":[{"type":"wait","ms":1}],"else":[{"type":"wait","ms":2}]},
            {"type":"click","x":5,"y":6}]}"#,
        )
        .unwrap();
        let a = r.to_actions(&FakeEnv(vec![]), &Vars::default()).unwrap();
        let w = Key::parse("w").unwrap();
        assert_eq!(a[0], Action::KeyDown(w));
        assert_eq!(a[2], Action::Wait(100));
        match &a[5] {
            Action::Branch {
                then, otherwise, ..
            } => {
                assert_eq!(then, &vec![Action::Wait(1)]);
                assert_eq!(otherwise, &vec![Action::Wait(2)]);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            a[6],
            Action::Click {
                at: Point { x: 5, y: 6 },
                button: MouseButton::Left
            }
        ));
        assert_eq!(r.duration_ms(), 100 + 1 + 100);
    }

    struct FakeEnv(Vec<&'static str>);
    impl RouteEnv for FakeEnv {
        fn probe(&self, patterns: &[String], _r: Option<Rect>) -> Probe {
            let hit = patterns.iter().any(|p| self.0.contains(&p.as_str()));
            Probe::new(patterns.join("|"), move || hit.then_some(Point::new(1, 1)))
        }
    }

    #[test]
    fn placeholders_and_pattern_steps() {
        let r = Route::parse(
            r#"{"map":"Lobby > Story","steps":[
              {"type":"click_pattern","pattern":"areas_button","fallback":[764,487]},
              {"type":"hold_until","keys":["w","d"],"patterns":["select_world"]},
              {"type":"scroll_until","pattern":"world_{map}","x":520,"y":330},
              {"type":"click_pattern","pattern":"act_{act}"}]}"#,
        )
        .unwrap();
        let v = Vars {
            map: "Ruined City (Midnight)".into(),
            act: "Legend Stage 3".into(),
            ..Default::default()
        };
        assert_eq!(
            r.patterns(&v),
            vec![
                "act_legend_stage_3",
                "areas_button",
                "select_world",
                "world_ruined_city_midnight"
            ]
        );
        let a = r.to_actions(&FakeEnv(vec!["select_world"]), &v).unwrap();
        let w = Key::parse("w").unwrap();
        // hold_until: keys down, wait, keys up
        assert_eq!(a[1], Action::KeyDown(w));
        assert!(matches!(&a[3], Action::WaitFor { required: true, .. }));
        assert_eq!(a[4], Action::KeyUp(w));
        assert!(matches!(&a[6], Action::ScrollUntil { up: false, .. }));
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
