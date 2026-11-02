//! A fake Roblox for testing the game loop end to end.
//!
//! Each screen is a set of named elements drawn as unique glyphs. Clicking an
//! element (or scrolling, or just waiting) moves to another screen, so the
//! real `RunCtx` code walks through lobby -> dialogs -> stage -> end screen
//! exactly as it would in the game, using real pattern matching.

use crate::capture::Frame;
use crate::geom::{Geometry, Hwnd, Point, Rect};
use crate::keys::Key;
use crate::platform::{MouseButton, Platform, WinInfo};
use crate::vision::{Library, Pattern};
use anyhow::Result;
use std::sync::Mutex;

pub struct El {
    pub name: &'static str,
    pub rect: Rect,
    pub click_to: Option<usize>,
}

pub struct Screen {
    pub els: Vec<El>,
    /// After this many captures, go to screen `.1` (loading, walking, a stage ending).
    pub auto: Option<(u32, usize)>,
    /// A wheel-down notch goes to this screen (scrolling a list).
    pub wheel_down: Option<usize>,
}

pub fn el(name: &'static str, x: i32, y: i32, click_to: Option<usize>) -> El {
    El {
        name,
        rect: Rect::new(x, y, 60, 18),
        click_to,
    }
}

pub struct Sim {
    pub screens: Vec<Screen>,
    state: Mutex<(usize, u32)>,
    cursor: Mutex<Point>,
    /// Names clicked, in order.
    pub clicks: Mutex<Vec<String>>,
    pub visited: Mutex<Vec<usize>>,
}

fn glyph(name: &str, x: i32, y: i32) -> bool {
    let mut h: u64 = 1469598103934665603;
    for b in name.bytes() {
        h = (h ^ b as u64).wrapping_mul(1099511628211);
    }
    let v = (h ^ (x as u64 * 0x9E37_79B9) ^ (y as u64 * 0x85EB_CA6B)).wrapping_mul(0xC2B2_AE35);
    (v >> 31) & 1 == 1
}

impl Sim {
    pub fn new(screens: Vec<Screen>) -> Self {
        Sim {
            screens,
            state: Mutex::new((0, 0)),
            cursor: Mutex::new(Point::default()),
            clicks: Mutex::new(vec![]),
            visited: Mutex::new(vec![0]),
        }
    }

    pub fn screen_now(&self) -> usize {
        self.state.lock().unwrap().0
    }

    fn go(&self, to: usize) {
        *self.state.lock().unwrap() = (to, 0);
        self.visited.lock().unwrap().push(to);
    }

    pub fn render(&self, screen: usize) -> Frame {
        let mut f = Frame::solid(800, 599, 30, 25, 20);
        for e in &self.screens[screen].els {
            for y in 0..e.rect.h {
                for x in 0..e.rect.w {
                    let on = glyph(e.name, x, y);
                    let i = (((e.rect.y + y) as u32 * 800 + (e.rect.x + x) as u32) * 4) as usize;
                    let c = if on { 240 } else { 10 };
                    f.bgra[i..i + 3].copy_from_slice(&[c, c, c]);
                }
            }
        }
        f
    }

    /// One pattern per element name (taken from the first screen it's on).
    pub fn library(&self) -> Library {
        let mut lib = Library::default();
        let mut seen = std::collections::HashSet::new();
        for (i, s) in self.screens.iter().enumerate() {
            let f = self.render(i);
            for e in &s.els {
                if seen.insert(e.name) {
                    let mut p = Pattern::text_from(&f, e.name, e.rect, 30, Some(128)).unwrap();
                    p.err0 = 0.0;
                    p.err1 = 0.0;
                    lib.insert(p);
                }
            }
        }
        lib
    }
}

impl Platform for Sim {
    fn list_windows(&self, _: &str) -> Vec<WinInfo> {
        vec![WinInfo {
            hwnd: Hwnd(1),
            pid: 1,
            title: "Roblox".into(),
        }]
    }
    fn geometry(&self, hwnd: Hwnd) -> Option<Geometry> {
        (hwnd == Hwnd(1)).then(|| Geometry {
            hwnd,
            outer: Rect::new(0, 0, 800, 599),
            client: Rect::new(0, 0, 800, 599),
        })
    }
    fn foreground(&self) -> Option<Hwnd> {
        Some(Hwnd(1))
    }
    fn focus(&self, _: Hwnd) -> Result<()> {
        Ok(())
    }
    fn is_minimized(&self, _: Hwnd) -> bool {
        false
    }
    fn move_resize(&self, _: Hwnd, _: Rect) -> Result<()> {
        Ok(())
    }
    fn work_area(&self) -> Rect {
        Rect::new(0, 0, 1920, 1040)
    }
    fn dpi(&self, _: Hwnd) -> u32 {
        96
    }
    fn capture_client(&self, _: Hwnd) -> Result<Frame> {
        let cur = {
            let mut st = self.state.lock().unwrap();
            st.1 += 1;
            st.0
        };
        if let Some((n, to)) = self.screens[cur].auto {
            if self.state.lock().unwrap().1 > n {
                self.go(to);
            }
        }
        Ok(self.render(self.screen_now()))
    }
    fn key(&self, _: Key, _: bool) -> Result<()> {
        Ok(())
    }
    fn mouse_move_abs(&self, x: i32, y: i32) -> Result<()> {
        *self.cursor.lock().unwrap() = Point::new(x, y);
        Ok(())
    }
    fn mouse_move_rel(&self, _: i32, _: i32) -> Result<()> {
        Ok(())
    }
    fn mouse_button(&self, b: MouseButton, down: bool) -> Result<()> {
        if b != MouseButton::Left || down {
            return Ok(());
        }
        let c = *self.cursor.lock().unwrap();
        let cur = self.screen_now();
        if let Some(e) = self.screens[cur].els.iter().find(|e| e.rect.contains(c)) {
            self.clicks.lock().unwrap().push(e.name.to_string());
            if let Some(to) = e.click_to {
                self.go(to);
            }
        }
        Ok(())
    }
    fn wheel(&self, notches: i32) -> Result<()> {
        if notches < 0 {
            if let Some(to) = self.screens[self.screen_now()].wheel_down {
                self.go(to);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Mode, Settings};
    use crate::control::Control;
    use crate::game::run::{Event, RunCtx};
    use crate::game::LiveScreen;
    use crate::input::{Arbiter, ArbiterOptions};
    use crate::movement::Routes;
    use crate::placement::Placements;
    use std::sync::Arc;

    fn scr(els: Vec<El>) -> Screen {
        Screen {
            els,
            auto: None,
            wheel_down: None,
        }
    }

    fn ctx(sim: Arc<Sim>, s: Settings) -> (RunCtx, std::sync::mpsc::Receiver<Event>, Arbiter) {
        let lib = sim.library();
        let arb = Arbiter::start(
            sim.clone(),
            Control::new(),
            ArbiterOptions {
                mouse_settle_ms: 1,
                click_hold_ms: 1,
                max_lease_ms: 400,
                click_mode: crate::input::ClickMode::Safe,
            },
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let screen = LiveScreen::new(sim.clone(), Hwnd(1), &lib);
        let c = RunCtx {
            arb: arb.handle(),
            control: Control::new(),
            main: Hwnd(1),
            clients: vec![(Hwnd(1), screen)],
            settings: s,
            routes: Routes::load(std::path::Path::new("/nonexistent")),
            placements: Placements::default(),
            tx,
            speed: 0.02,
        };
        (c, rx, arb)
    }

    /// Summer Event, Play Here, solo: lobby -> walk -> Join Matchmaking ->
    /// Open Portal -> Start -> loading (map spotted) -> Vote Start shows ->
    /// Settings > Teleport to Spawn > close -> Vote Start -> stage ->
    /// VICTORY -> Next -> Replay -> loading -> the same setup again.
    #[test]
    fn summer_event_full_sequence() {
        let sim = Arc::new(Sim::new(vec![
            /*0 lobby*/ scr(vec![el("play_button", 70, 300, Some(1))]),
            /*1 walking*/
            Screen {
                els: vec![],
                auto: Some((2, 2)),
                wheel_down: None,
            },
            /*2 dialog*/
            scr(vec![
                el("join_matchmaking", 360, 180, None),
                el("play_here", 300, 300, Some(3)),
            ]),
            /*3 portal?*/
            scr(vec![
                el("open_portal", 360, 180, None),
                el("open_button", 300, 330, Some(4)),
                el("open_friends_only", 400, 330, None),
            ]),
            /*4 portal lobby*/
            scr(vec![
                el("waiting_for_players", 60, 230, None),
                el("start_button", 470, 400, Some(5)),
            ]),
            /*5 loading*/
            Screen {
                els: vec![el("loading_summer_beach", 330, 280, None)],
                auto: Some((3, 6)),
                wheel_down: None,
            },
            /*6 vote*/
            scr(vec![
                el("vote_start", 380, 100, Some(9)),
                el("settings_button", 10, 560, Some(7)),
            ]),
            /*7 settings*/
            Screen {
                els: vec![el("settings_close", 570, 140, Some(6))],
                auto: None,
                wheel_down: Some(8),
            },
            /*8 settings scrolled*/
            scr(vec![
                el("tp_spawn", 300, 300, None),
                el("tp_arrow", 478, 307, Some(8)),
                el("settings_close", 570, 140, Some(6)),
            ]),
            /*9 stage*/
            Screen {
                els: vec![],
                auto: Some((6, 10)),
                wheel_down: None,
            },
            /*10 end*/
            scr(vec![
                el("victory", 360, 150, None),
                el("next_button", 380, 460, Some(11)),
            ]),
            /*11 after*/
            scr(vec![
                el("return_to_lobby", 250, 500, Some(0)),
                el("replay", 450, 500, Some(5)),
            ]),
        ]));
        let mut s = Settings::default();
        s.run.mode = Mode::SummerHunt;
        let (c, _rx, _arb) = ctx(sim.clone(), s);
        c.enter_stage().unwrap();
        assert_eq!(sim.screen_now(), 5, "in the loading screen after Start");
        let map = c.stage_setup("").unwrap();
        assert_eq!(map, "Summer Beach", "map read off the loading screen");
        assert_eq!(sim.screen_now(), 9, "voted, stage running");
        let won = c.upgrade_until_end(&[]).unwrap();
        assert!(won);
        let lobby = c.after_end().unwrap();
        assert!(!lobby, "replays by default");
        assert_eq!(c.stage_setup(&map).unwrap(), "Summer Beach");
        assert_eq!(
            *sim.clicks.lock().unwrap(),
            vec![
                "play_button",
                "play_here",
                "open_button",
                "start_button",
                "settings_button",
                "tp_arrow",
                "settings_close",
                "vote_start",
                "next_button",
                "replay",
                "settings_button",
                "tp_arrow",
                "settings_close",
                "vote_start",
            ]
        );
    }

    /// Legend: Areas > Play Area > walk > Select World (Main list) > Legend
    /// toggle > scroll to the world > world > act > Select > Play Here > Start.
    #[test]
    fn legend_stage_full_sequence() {
        let sim = Arc::new(Sim::new(vec![
            /*0 lobby*/ scr(vec![el("areas_button", 740, 470, Some(1))]),
            /*1 areas*/ scr(vec![el("play_area_button", 480, 300, Some(2))]),
            /*2 walking*/
            Screen {
                els: vec![],
                auto: Some((2, 3)),
                wheel_down: None,
            },
            /*3 main list*/
            scr(vec![
                el("select_world", 520, 120, None),
                el("legend_stages_toggle", 520, 60, Some(4)),
            ]),
            /*4 legend list top*/
            Screen {
                els: vec![
                    el("select_world", 520, 120, None),
                    el("main_stages_toggle", 520, 60, None),
                ],
                auto: None,
                wheel_down: Some(5),
            },
            /*5 scrolled*/
            scr(vec![
                el("select_world", 520, 120, None),
                el("world_ruined_city_midnight", 500, 300, Some(6)),
            ]),
            /*6 acts*/ scr(vec![el("act_legend_stage_3", 200, 330, Some(7))]),
            /*7 picked*/ scr(vec![el("select_button", 450, 500, Some(8))]),
            /*8 dialog*/
            scr(vec![
                el("join_matchmaking", 360, 180, None),
                el("play_here", 300, 300, Some(9)),
            ]),
            /*9 start*/ scr(vec![el("start_button", 470, 400, Some(10))]),
            /*10 loading*/
            Screen {
                els: vec![],
                auto: Some((3, 11)),
                wheel_down: None,
            },
            /*11 stage*/
            Screen {
                els: vec![],
                auto: Some((4, 12)),
                wheel_down: None,
            },
            /*12 end*/
            scr(vec![
                el("defeat", 360, 150, None),
                el("next_button", 380, 460, Some(13)),
            ]),
            /*13 after*/
            scr(vec![
                el("return_to_lobby", 250, 500, Some(0)),
                el("replay", 450, 500, Some(10)),
            ]),
        ]));
        let mut s = Settings::default();
        s.run.mode = Mode::Legend;
        s.run.map = "Ruined City (Midnight)".into();
        s.run.act = "Legend Stage 3".into();
        s.run.return_to_lobby = true;
        let (mut c, _rx, _arb) = ctx(sim.clone(), s);
        c.routes.maps.clear(); // no map walk in the fake game
        c.enter_stage().unwrap();
        assert_eq!(sim.screen_now(), 10);
        assert_eq!(
            c.stage_setup("").unwrap(),
            "Ruined City",
            "fixed map for Legend"
        );
        assert!(!c.upgrade_until_end(&[]).unwrap(), "defeat");
        assert!(c.after_end().unwrap(), "back in the lobby");
        assert_eq!(
            *sim.clicks.lock().unwrap(),
            vec![
                "areas_button",
                "play_area_button",
                "legend_stages_toggle",
                "world_ruined_city_midnight",
                "act_legend_stage_3",
                "select_button",
                "play_here",
                "start_button",
                "next_button",
                "return_to_lobby"
            ]
        );
    }

    /// Story and Raid lobby walks pick the world + act and press Start.
    #[test]
    fn story_and_raid_enter_stage() {
        for (mode, map, act, door) in [
            (Mode::Story, "Planet Greenie", "Act 3", "play_area_button"),
            (Mode::Raid, "Sacred Planet", "Act 2", "raids_button"),
        ] {
            let world: &'static str =
                Box::leak(format!("world_{}", crate::vision::key(map)).into_boxed_str());
            let actn: &'static str =
                Box::leak(format!("act_{}", crate::vision::key(act)).into_boxed_str());
            let sim = Arc::new(Sim::new(vec![
                /*0*/ scr(vec![el("areas_button", 740, 470, Some(1))]),
                /*1*/ scr(vec![el(door, 480, 300, Some(2))]),
                /*2 walking*/
                Screen {
                    els: vec![],
                    auto: Some((2, 3)),
                    wheel_down: None,
                },
                /*3 list*/
                Screen {
                    els: vec![el("select_world", 520, 120, None)],
                    auto: None,
                    wheel_down: Some(4),
                },
                /*4 scrolled*/
                scr(vec![
                    el("select_world", 520, 120, None),
                    el(world, 500, 300, Some(5)),
                ]),
                /*5 acts*/ scr(vec![el(actn, 200, 330, Some(6))]),
                /*6*/ scr(vec![el("select_button", 450, 500, Some(7))]),
                /*7*/
                scr(vec![
                    el("join_matchmaking", 360, 180, None),
                    el("play_here", 300, 300, Some(8)),
                ]),
                /*8*/ scr(vec![el("start_button", 470, 400, Some(9))]),
                /*9 loading*/ scr(vec![]),
            ]));
            let mut s = Settings::default();
            s.run.mode = mode;
            s.run.map = map.into();
            s.run.act = act.into();
            let (c, _rx, _arb) = ctx(sim.clone(), s);
            c.enter_stage().unwrap();
            assert_eq!(sim.screen_now(), 9, "{mode:?}");
            assert_eq!(
                *sim.clicks.lock().unwrap(),
                vec![
                    "areas_button",
                    door,
                    world,
                    actn,
                    "select_button",
                    "play_here",
                    "start_button"
                ],
                "{mode:?}"
            );
        }
    }

    /// A card pop-up while placing: the middle card gets picked, then
    /// placing carries on.
    #[test]
    fn card_popup_while_placing() {
        let sim = Arc::new(Sim::new(vec![
            /*0 cards*/
            scr(vec![
                el("choose_card", 360, 100, None),
                el("card_mid", 400, 294, Some(1)),
            ]),
            /*1 stage*/ scr(vec![]),
        ]));
        let mut s = Settings::default();
        s.run.mode = Mode::SummerHunt;
        for u in s.units.iter_mut() {
            u.enabled = false;
        }
        s.units[0].enabled = true;
        s.units[0].place = 2;
        s.place_delay_ms = 0;
        let (c, _rx, _arb) = ctx(sim.clone(), s);
        let placed = c.place_all("").unwrap();
        assert_eq!(placed.len(), 2);
        assert_eq!(
            sim.clicks.lock().unwrap().first().map(|s| s.as_str()),
            Some("card_mid")
        );
        assert_eq!(sim.screen_now(), 1);
    }

    /// VICTORY on screen while placing: stop placing right away.
    #[test]
    fn stage_end_stops_placing() {
        let sim = Arc::new(Sim::new(vec![scr(vec![el("victory", 360, 150, None)])]));
        let mut s = Settings::default();
        for u in s.units.iter_mut() {
            u.enabled = false;
        }
        s.units[0].enabled = true;
        s.units[0].place = 3;
        s.place_delay_ms = 0;
        let (c, _rx, _arb) = ctx(sim.clone(), s);
        assert!(c.place_all("").unwrap().is_empty());
        assert!(c.upgrade_until_end(&[]).unwrap(), "won");
    }

    /// The whole loop keeps going: win -> Replay -> win -> Replay ...
    #[test]
    fn loop_keeps_replaying() {
        let sim = Arc::new(Sim::new(vec![
            /*0 lobby*/ scr(vec![el("play_button", 70, 300, Some(1))]),
            /*1 walking*/
            Screen {
                els: vec![],
                auto: Some((2, 2)),
                wheel_down: None,
            },
            /*2 dialog*/
            scr(vec![
                el("join_matchmaking", 360, 180, None),
                el("play_here", 300, 300, Some(3)),
            ]),
            /*3 portal?*/
            scr(vec![
                el("open_portal", 360, 180, None),
                el("open_button", 300, 330, Some(4)),
            ]),
            /*4 portal lobby*/ scr(vec![el("start_button", 470, 400, Some(5))]),
            /*5 loading*/
            Screen {
                els: vec![],
                auto: Some((2, 6)),
                wheel_down: None,
            },
            /*6 vote*/ scr(vec![el("vote_start", 380, 100, Some(7))]),
            /*7 stage*/
            Screen {
                els: vec![],
                auto: Some((4, 8)),
                wheel_down: None,
            },
            /*8 end*/
            scr(vec![
                el("defeat", 360, 150, None),
                el("next_button", 380, 460, Some(9)),
            ]),
            /*9 after*/
            scr(vec![
                el("return_to_lobby", 250, 500, Some(0)),
                el("replay", 450, 500, Some(5)),
            ]),
        ]));
        let mut s = Settings::default();
        s.run.mode = Mode::SummerHunt;
        s.step_delay_ms = 0;
        let (c, rx, _arb) = ctx(sim.clone(), s);
        let stop = c.control.clone();
        let t = std::thread::spawn(move || c.run());
        let mut ends = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while ends < 3 && std::time::Instant::now() < deadline {
            match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(Event::Loss) | Ok(Event::Win) => ends += 1,
                Ok(Event::Done(why)) => panic!("loop ended: {why}"),
                _ => {}
            }
        }
        stop.stop();
        t.join().unwrap();
        assert_eq!(ends, 3, "3 stages in a row");
        let replays = sim
            .clicks
            .lock()
            .unwrap()
            .iter()
            .filter(|c| *c == "replay")
            .count();
        assert!(replays >= 2, "replayed {replays}x");
    }

    #[test]
    fn missing_button_is_reported_not_hung() {
        let sim = Arc::new(Sim::new(vec![
            scr(vec![el("play_button", 70, 300, Some(1))]),
            Screen {
                els: vec![],
                auto: Some((1, 2)),
                wheel_down: None,
            },
            scr(vec![el("join_matchmaking", 360, 180, None)]), // no play_here on screen
            scr(vec![el("play_here", 1, 1, None)]),            // so the pattern exists
        ]));
        let (c, _rx, _arb) = ctx(sim, Settings::default());
        let e = c.enter_stage().unwrap_err();
        assert!(e.contains("play_here"), "{e}");
    }
}
