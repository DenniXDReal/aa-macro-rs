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
    /// How many Roblox windows (hwnd 1..=n) share this screen.
    pub windows: usize,
    fg: Mutex<Hwnd>,
    /// (window, x, y) of every left click.
    pub raw_clicks: Mutex<Vec<(Hwnd, i32, i32)>>,
    pub visited: Mutex<Vec<usize>>,
    /// Text the fake OCR reads where its box overlaps the area asked for
    /// (empty = no OCR, like a PC without it).
    pub ocr_texts: Mutex<Vec<(Rect, String)>>,
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
            windows: 1,
            fg: Mutex::new(Hwnd(1)),
            raw_clicks: Mutex::new(vec![]),
            visited: Mutex::new(vec![0]),
            ocr_texts: Mutex::new(vec![]),
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
    fn has_ocr(&self) -> bool {
        !self.ocr_texts.lock().unwrap().is_empty()
    }
    fn ocr(&self, _frame: &Frame, area: Rect) -> Option<String> {
        let t: Vec<String> = self
            .ocr_texts
            .lock()
            .unwrap()
            .iter()
            .filter(|(r, _)| r.intersect(&area).is_some())
            .map(|(_, t)| t.clone())
            .collect();
        (!t.is_empty()).then(|| t.join("\n"))
    }
    fn list_windows(&self, _: &str) -> Vec<WinInfo> {
        (1..=self.windows as isize)
            .map(|i| WinInfo {
                hwnd: Hwnd(i),
                pid: i as u32,
                title: "Roblox".into(),
            })
            .collect()
    }
    fn geometry(&self, hwnd: Hwnd) -> Option<Geometry> {
        (hwnd.0 >= 1 && hwnd.0 <= self.windows as isize).then(|| Geometry {
            hwnd,
            outer: Rect::new(0, 0, 800, 599),
            client: Rect::new(0, 0, 800, 599),
        })
    }
    fn foreground(&self) -> Option<Hwnd> {
        Some(*self.fg.lock().unwrap())
    }
    fn focus(&self, h: Hwnd) -> Result<()> {
        *self.fg.lock().unwrap() = h;
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
        self.raw_clicks
            .lock()
            .unwrap()
            .push((*self.fg.lock().unwrap(), c.x, c.y));
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
    use crate::config::{Mode, Settings, Upgrade};
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
                swap_delay_ms: 0,
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
            routes: Routes::load(std::path::Path::new("/nonexistent")).into(),
            resume: None,
            loot_hook: None,
            session: Default::default(),
            step_deadline: Default::default(),
            watch: Default::default(),
            placements: Placements::default(),
            tx,
            speed: 0.02,
            memory: Default::default(),
        };
        (c, rx, arb)
    }

    /// Custom events end to end: a rule file on disk, the OCR area reading
    /// "Wave 10 / 50" (random waves before it), the rule's click fires once
    /// at wave 10 - not again while it stays on screen, again when it comes
    /// back; a rule for another map never fires; a jump request is queued.
    #[test]
    fn custom_events_fire_on_their_text() {
        use super::super::events::{Do, Match, Rule, Rules, When};
        let sim = Arc::new(Sim::new(vec![scr(vec![])]));
        let (c, rx, _arb) = ctx(sim.clone(), Settings::default());
        let rules = Rules {
            rules: vec![
                Rule {
                    name: "wave 10".into(),
                    when: When::Every(1),
                    area: [10, 10, 200, 30],
                    text: r"Wave 10\b".into(),
                    matching: Match::Regex,
                    action: Do::Click { x: 321, y: 123 },
                    ..Default::default()
                },
                Rule {
                    name: "other map".into(),
                    when: When::Every(1),
                    maps: vec!["Navy Bay".into()],
                    action: Do::Click { x: 1, y: 1 },
                    ..Default::default()
                },
                Rule {
                    name: "jump".into(),
                    when: When::BeforeEnd,
                    action: Do::ChangeStep {
                        step: "upgrade".into(),
                    },
                    ..Default::default()
                },
            ],
            type_gap_ms: 1,
        };
        super::super::events::save(&rules).unwrap();
        *crate::game::run::CURRENT_MAP.lock().unwrap() = "Sand Village".into();
        let show = |t: &str| {
            *sim.ocr_texts.lock().unwrap() = vec![(Rect::new(0, 0, 300, 60), t.to_string())];
        };
        let ours = || {
            sim.raw_clicks
                .lock()
                .unwrap()
                .iter()
                .filter(|k| (k.1, k.2) == (321, 123))
                .count()
        };
        // "random" waves before 10: nothing
        let mut seed = 7u32;
        for _ in 0..6 {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            let w = 1 + (seed >> 16) % 9; // 1..=9
            show(&format!("Wave {w} / 50"));
            std::thread::sleep(std::time::Duration::from_millis(1050));
            c.fire_events(When::Every(0)).unwrap();
        }
        assert_eq!(ours(), 0, "no wave 10 yet");
        // wave 10: once, and not again while it stays up
        show("Wave 10 / 50");
        for _ in 0..3 {
            std::thread::sleep(std::time::Duration::from_millis(1050));
            c.fire_events(When::Every(0)).unwrap();
        }
        let logs: Vec<String> = rx
            .try_iter()
            .filter_map(|e| match e {
                Event::Log(l) => Some(l),
                _ => None,
            })
            .collect();
        assert_eq!(
            ours(),
            1,
            "fired once: {logs:?} {:?}",
            sim.raw_clicks.lock().unwrap()
        );
        // gone (wave 11), then back: again. (A look right after input on the
        // same unchanged picture is skipped as "frozen" - other sim tests
        // click Hwnd(1) too - so give each phase a few looks.)
        show("Wave 11 / 50");
        for _ in 0..3 {
            std::thread::sleep(std::time::Duration::from_millis(1050));
            c.fire_events(When::Every(0)).unwrap();
        }
        assert_eq!(ours(), 1, "not while wave 11 shows");
        show("Wave 10 / 50");
        for _ in 0..4 {
            std::thread::sleep(std::time::Duration::from_millis(1050));
            c.fire_events(When::Every(0)).unwrap();
            if ours() == 2 {
                break;
            }
        }
        assert_eq!(ours(), 2, "fired again after it came back");
        // the Navy Bay rule never fired on Sand Village
        assert!(!sim
            .raw_clicks
            .lock()
            .unwrap()
            .iter()
            .any(|k| (k.1, k.2) == (1, 1)));
        // Before End: the step jump is queued
        c.fire_events(When::BeforeEnd).unwrap();
        assert_eq!(
            super::super::events::take_jump(),
            Some(super::super::steps::Step::Upgrade)
        );
        let _ = std::fs::remove_file(super::super::events::path());
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
        assert_eq!(
            map, "Summer Beach",
            "Play Here: map read off the loading screen"
        );
        assert_eq!(sim.screen_now(), 9, "voted, stage running");
        let won = c.upgrade_until_end(&[]).unwrap();
        assert!(won);
        let lobby = c.after_end().unwrap();
        assert!(!lobby, "replays by default");
        assert_eq!(
            c.stage_setup(&map).unwrap(),
            map,
            "same map recognised again"
        );
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
                // same map again: no Teleport to Spawn on a replay
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
        // no map walk in the fake game
        let mut r = (*c.routes.get()).clone();
        r.maps.clear();
        c.routes = r.into();
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
    fn step_machine_keeps_replaying() {
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
        s.step_machine = true;
        let (c, rx, _arb) = ctx(sim.clone(), s);
        let stop = c.control.clone();
        let t = std::thread::spawn(move || c.run());
        let mut ends = 0;
        let mut handovers = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while ends < 3 && std::time::Instant::now() < deadline {
            match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(Event::Loss) | Ok(Event::Win) => ends += 1,
                Ok(Event::Fatal(f)) => eprintln!("FATAL {f}"),
                Ok(Event::Log(l)) if l.starts_with("place > upgrade:") => handovers += 1,
                Ok(Event::Done(why)) => panic!("loop ended: {why}"),
                _ => {}
            }
        }
        stop.stop();
        t.join().unwrap();
        assert_eq!(ends, 3, "3 stages in a row (step machine)");
        assert!(
            handovers >= 3,
            "Place > Upgrade hand-over every round ({handovers})"
        );
        let replays = sim
            .clicks
            .lock()
            .unwrap()
            .iter()
            .filter(|c| *c == "replay")
            .count();
        assert!(replays >= 2, "replayed {replays}x");
    }

    /// Disconnect box: check() stops what's running, the recovery clicks
    /// Reconnect, waits for the lobby and carries on; limits respected.
    #[test]
    fn disconnect_reconnects_to_the_lobby() {
        let sim = Arc::new(Sim::new(vec![
            /*0 disconnected*/
            scr(vec![
                el("disconnect_box", 300, 250, None),
                el("reconnect_button", 330, 330, Some(1)),
            ]),
            /*1 lobby*/ scr(vec![el("play_button", 70, 300, None)]),
        ]));
        let mut s = Settings::default();
        s.recovery.max_reconnects_per_hour = 1;
        let (c, _rx, _arb) = ctx(sim.clone(), s);
        c.watch_once(false);
        let inc = c.watch.first(crate::game::watchdog::Kind::Disconnected);
        assert_eq!(inc.map(|i| i.client), Some(1), "watchdog saw it");
        let e = c.check().unwrap_err();
        assert!(e.starts_with(crate::game::run::DISCONNECTED), "{e}");
        // the step machine stops its step for it, and calls it a disconnect
        use crate::game::steps::{AbortSource, Hooks};
        assert_eq!(c.should_abort(), Some(AbortSource::Disconnect));
        assert!(matches!(
            crate::game::machine::classify(
                &e,
                crate::game::steps::Step::Upgrade,
                std::time::Duration::ZERO
            ),
            crate::game::steps::FailureReason::ExternalAbort {
                source: AbortSource::Disconnect
            }
        ));
        c.recover_disconnect().expect("reconnected");
        assert!(sim
            .clicks
            .lock()
            .unwrap()
            .iter()
            .any(|n| n == "reconnect_button"));
        assert_eq!(sim.screen_now(), 1, "in the lobby");
        assert!(c.watch.incidents().is_empty());
        assert!(c.check().is_ok(), "nothing stops the loop now");
        // 1 an hour: the next one goes to the relaunch instead
        let e = c.recover_disconnect().unwrap_err();
        assert!(e.contains("reconnects already this hour"), "{e}");
    }

    #[test]
    fn reconnect_off_means_relaunch() {
        let sim = Arc::new(Sim::new(vec![scr(vec![el(
            "disconnect_box",
            300,
            250,
            None,
        )])]));
        let mut s = Settings::default();
        s.recovery.reconnect = false;
        let (c, _rx, _arb) = ctx(sim, s);
        assert!(c
            .recover_disconnect()
            .unwrap_err()
            .contains("reconnect is off"));
    }

    /// Captcha: paused + the app told; gone: un-paused + told.
    #[test]
    fn captcha_pauses_until_gone() {
        let sim = Arc::new(Sim::new(vec![
            /*0 captcha, gone after a few looks*/
            Screen {
                els: vec![el("captcha", 300, 250, None)],
                auto: Some((3, 1)),
                wheel_down: None,
            },
            /*1 stage*/ scr(vec![el("vote_start", 400, 100, None)]),
        ]));
        let (c, rx, _arb) = ctx(sim, Settings::default());
        c.watch_once(false);
        assert!(c.control.is_paused(), "paused for the captcha");
        let mut told = false;
        while let Ok(e) = rx.try_recv() {
            told |= matches!(e, Event::Captcha(_));
        }
        assert!(told, "app pinged");
        for _ in 0..6 {
            c.watch_once(false);
        }
        assert!(!c.control.is_paused(), "carries on once it's gone");
        let mut gone = false;
        while let Ok(e) = rx.try_recv() {
            gone |= matches!(e, Event::CaptchaGone);
        }
        assert!(gone);
    }

    /// Priority upgrade, no money: the top unit's level doesn't move -
    /// the macro stays on it (waits for cash, tries again) and never goes
    /// to the next unit or rests it.
    #[test]
    fn priority_upgrade_waits_for_cash() {
        // its panel open; two screens taking turns so the picture changes
        // (a picture that never changes after input reads as a frozen window)
        let sim = Arc::new(Sim::new(vec![
            Screen {
                els: vec![
                    el("unit_close", 600, 120, None),
                    el("tick_a", 700, 40, None),
                ],
                auto: Some((1, 1)),
                wheel_down: None,
            },
            Screen {
                els: vec![
                    el("unit_close", 600, 120, None),
                    el("tick_b", 700, 60, None),
                ],
                auto: Some((1, 0)),
                wheel_down: None,
            },
        ]));
        // the level box always says 3 (no money for 4)
        sim.ocr_texts
            .lock()
            .unwrap()
            .push((Rect::new(170, 228, 142, 24), "UPG [3]".into()));
        let mut s = Settings::default();
        s.step_delay_ms = 0;
        s.run.priority_upgrade = true;
        s.units[0].enabled = true;
        s.units[0].upgrade = Upgrade::Level(5);
        s.units[0].priority = 1;
        s.units[1].enabled = true;
        s.units[1].upgrade = Upgrade::Max;
        s.units[1].priority = 2;
        let (c, rx, _arb) = ctx(sim.clone(), s);
        let ctl = c.control.clone();
        let placed = vec![
            (Hwnd(1), 1, Point::new(400, 300)),
            (Hwnd(1), 2, Point::new(500, 380)),
        ];
        let t = std::thread::spawn(move || {
            let _ = c.upgrade_until_end(&placed);
        });
        // wait for what's checked (3 visits + the cash-wait line) instead of
        // a fixed 6 s - under a busy test run the first visit can be late;
        // 60 s at most
        let want =
            "Upgrade: slot 1 at 400,300 (Level 3) - waiting for cash to reach Target Level 5...";
        let mut logs: Vec<String> = vec![];
        let started = std::time::Instant::now();
        loop {
            logs.extend(rx.try_iter().filter_map(|e| match e {
                Event::Log(l) => Some(l),
                _ => None,
            }));
            let seen = logs.iter().any(|l| l.contains(want));
            if (seen && sim.raw_clicks.lock().unwrap().len() >= 3)
                || started.elapsed() > std::time::Duration::from_secs(60)
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        ctl.stop();
        t.join().unwrap();
        logs.extend(rx.try_iter().filter_map(|e| match e {
            Event::Log(l) => Some(l),
            _ => None,
        }));
        assert!(logs.iter().any(|l| l.contains(want)), "{logs:#?}");
        assert!(
            !logs.iter().any(|l| l.contains("back in a minute")),
            "never rested: {logs:#?}"
        );
        // it went back to slot 1 visit after visit instead of moving on
        // (later clicks may differ: other tests share the fake window's
        // last-picture memory, so a read can come back empty here)
        let clicks = sim.raw_clicks.lock().unwrap().clone();
        assert!(clicks.len() >= 3, "{clicks:?}");
        assert!(
            clicks[..3]
                .iter()
                .all(|(_, x, y)| (*x - 400).abs() <= 20 && (*y - 300).abs() <= 30),
            "{clicks:?}"
        );
    }

    /// Step machine: a step that keeps failing (no Play Here button) ends
    /// in Fatal - the loop pauses itself and tells the app.
    #[test]
    fn step_machine_pauses_on_fatal() {
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
        let mut s = Settings::default();
        s.step_machine = true;
        s.step_delay_ms = 0;
        let (c, rx, _arb) = ctx(sim, s);
        let ctl = c.control.clone();
        let t = std::thread::spawn(move || c.run());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        let mut fatal = None;
        while fatal.is_none() && std::time::Instant::now() < deadline {
            match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(Event::Fatal(why)) => fatal = Some(why),
                Ok(Event::Done(why)) => panic!("loop ended instead of pausing: {why}"),
                _ => {}
            }
        }
        let why = fatal.expect("a Fatal within 2 min");
        assert!(why.starts_with("lobby:"), "{why}");
        assert!(ctl.is_paused(), "paused itself");
        ctl.stop();
        t.join().unwrap();
    }

    /// DEFEAT shows but the Next button isn't recognised: it clicks under
    /// the title until Replay comes up.
    #[test]
    fn next_not_recognised_still_replays() {
        let sim = Arc::new(Sim::new(vec![
            /*0 end*/
            scr(vec![
                el("defeat", 360, 150, None),
                el("odd_next", 360, 355, Some(1)),
            ]),
            /*1 after*/ scr(vec![el("replay", 450, 500, Some(2))]),
            /*2 loading*/ scr(vec![]),
        ]));
        let mut s = Settings::default();
        s.step_delay_ms = 0;
        let (c, _rx, _arb) = ctx(sim.clone(), s);
        assert!(!c.after_end().unwrap());
        assert_eq!(*sim.clicks.lock().unwrap(), vec!["odd_next", "replay"]);
    }

    /// Three Invite buttons in a list: all three are found, wherever the
    /// first one was captured.
    #[test]
    fn finds_every_invite_button() {
        let sim = Arc::new(Sim::new(vec![scr(vec![
            el("party_invite", 600, 120, None),
            el("party_invite", 600, 180, None),
            el("party_invite", 600, 300, None),
        ])]));
        let lib = sim.library();
        let screen = LiveScreen::new(sim.clone(), Hwnd(1), &lib);
        let all = screen.find_all_wide("party_invite");
        assert_eq!(all.len(), 3, "{all:?}");
        assert_eq!(all[2].y, 309);
    }

    /// Three accounts placing + upgrading at once: every unit goes down on
    /// the right window, nothing deadlocks, every placed unit gets upgraded.
    #[test]
    fn three_accounts_place_and_upgrade_together() {
        let mut sim = Sim::new(vec![
            /*0 stage*/
            Screen {
                els: vec![],
                auto: Some((400, 1)),
                wheel_down: None,
            },
            /*1 end*/ scr(vec![el("victory", 360, 150, None)]),
        ]);
        sim.windows = 3;
        let sim = Arc::new(sim);
        let lib = sim.library();
        let arb = Arbiter::start(
            sim.clone(),
            Control::new(),
            ArbiterOptions {
                mouse_settle_ms: 1,
                click_hold_ms: 1,
                max_lease_ms: 5000,
                click_mode: crate::input::ClickMode::Single,
                swap_delay_ms: 0,
            },
        );
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut s = Settings::default();
        for u in s.units.iter_mut() {
            u.enabled = false;
        }
        s.units[0].enabled = true;
        s.units[0].place = 2;
        s.units[0].upgrade = crate::config::Upgrade::Level(2);
        s.units[1].enabled = true;
        s.units[1].place = 1;
        s.placers = vec![1, 2, 3];
        s.place_delay_ms = 0;
        s.step_delay_ms = 0;
        s.place_settle_ms = 100;
        s.place_check_ms = 500;
        let clients = (1..=3)
            .map(|i| (Hwnd(i), LiveScreen::new(sim.clone(), Hwnd(i), &lib)))
            .collect();
        let c = RunCtx {
            arb: arb.handle(),
            control: Control::new(),
            main: Hwnd(1),
            clients,
            settings: s,
            routes: Routes::load(std::path::Path::new("/nonexistent")).into(),
            resume: None,
            loot_hook: None,
            session: Default::default(),
            step_deadline: Default::default(),
            watch: Default::default(),
            placements: Placements::default(),
            tx,
            speed: 0.02,
            memory: Default::default(),
        };
        let t = std::time::Instant::now();
        let placed = c.place_all("").unwrap();
        assert_eq!(placed.len(), 9, "3 units x 3 accounts");
        for h in 1..=3 {
            assert_eq!(
                placed.iter().filter(|p| p.0 == Hwnd(h)).count(),
                3,
                "account {h}"
            );
        }
        // every click landed on the window that was in front at the time,
        // and all three windows got their clicks
        let clicks = sim.raw_clicks.lock().unwrap().clone();
        for h in 1..=3 {
            assert!(clicks.iter().filter(|c| c.0 == Hwnd(h)).count() >= 3);
        }
        assert!(c.upgrade_until_end(&placed).unwrap(), "won");
        assert!(
            t.elapsed() < std::time::Duration::from_secs(60),
            "no deadlock"
        );
    }

    /// Infinity Castle: walk in, Cancel shows, Normal, Start above Cancel.
    #[test]
    fn infinity_castle_enters() {
        let sim = Arc::new(Sim::new(vec![
            /*0*/ scr(vec![el("areas_x", 740, 478, Some(1))]),
            /*1*/ scr(vec![el("castle_door", 350, 322, Some(2))]),
            /*2*/
            scr(vec![
                el("mode_cancel", 380, 540, None),
                el("castle_start", 370, 420, Some(3)),
                el("easy", 400, 368, None),
            ]),
            /*3 loading*/ scr(vec![]),
        ]));
        let mut s = Settings::default();
        s.run.mode = Mode::InfinityCastle;
        s.step_delay_ms = 0;
        let (c, _rx, _arb) = ctx(sim.clone(), s);
        c.enter_stage().unwrap();
        assert_eq!(sim.screen_now(), 3);
        assert_eq!(
            *sim.clicks.lock().unwrap(),
            vec!["areas_x", "castle_door", "easy", "castle_start"]
        );
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
