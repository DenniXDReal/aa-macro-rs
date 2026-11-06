//! The game loop (v1): lobby walk -> matchmaking / portal -> load in ->
//! map walk -> top-down camera -> place (every placing client) -> upgrade by
//! priority until the end screen -> Next -> Replay or back to the lobby.
//!
//! Everything it looks for is one of your patterns; [`plan`] lists them per
//! step so the Home page can show what's still missing before you Start.

use super::placing::{camera_batch, spots_key, Keys};
use super::LiveScreen;
use crate::config::{Mode, PlayChoice, Settings, Upgrade};
use crate::control::Control;
use crate::geom::{Hwnd, Point};
use crate::input::{Action, ArbiterHandle, Batch};
use crate::keys::Key;
use crate::movement::{Routes, Vars};
use crate::placement::Placements;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

/// Lobby walk used by a mode.
pub fn lobby_route(mode: Mode) -> &'static str {
    match mode {
        Mode::SummerHunt => "summer",
        Mode::Story => "story",
        Mode::Legend => "legend",
        Mode::Raid => "raid",
        Mode::InfinityCastle => "castle",
        Mode::Challenge | Mode::Dungeon => "challenge",
    }
}

/// Map-detect patterns are called `loading_<map>` (or `loading_<map>_2`
/// for a second look of the same map). This turns one back into a map name.
pub fn map_from_pattern(p: &str) -> String {
    let rest = p.trim_start_matches("loading_");
    let rest = match rest.rsplit_once('_') {
        Some((a, n)) if n.chars().all(|c| c.is_ascii_digit()) => a,
        _ => rest,
    };
    rest.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Which known map name the OCR text mentions (longest match wins, so
/// "Future City (Tyrant)" beats "Future City").
pub fn match_map(text: &str, known: &[String]) -> Option<String> {
    let flat = |s: &str| -> String {
        s.to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect()
    };
    let t = flat(text);
    known
        .iter()
        .filter(|m| {
            let k = flat(m);
            k.len() >= 5 && t.contains(&k)
        })
        .max_by_key(|m| flat(m).len())
        .cloned()
}

/// Pattern name used to recognise `map` while it loads.
pub fn loading_pattern(map: &str) -> String {
    format!("loading_{}", crate::vision::key(&route_map(map)))
}

/// Map name as the per-map routes know it ("Ruined City (Midnight)" -> "Ruined City").
pub fn route_map(name: &str) -> String {
    name.split(" (").next().unwrap_or(name).trim().to_string()
}

fn vars(s: &Settings) -> Vars {
    Vars {
        map: s.run.map.clone(),
        act: s.run.act.clone(),
        click_wait_ms: s.step_delay_ms,
    }
}

/// One step of the sequence and the patterns it needs (`optional` = it
/// carries on without them).
#[derive(Clone, Debug)]
pub struct PlanStep {
    pub label: String,
    pub needs: Vec<String>,
    pub optional: bool,
}

/// The whole sequence for the chosen mode, step by step.
pub fn plan(s: &Settings, routes: &Routes) -> Vec<PlanStep> {
    let mode = s.run.mode;
    let v = vars(s);
    let step = |label: &str, needs: &[&str], optional: bool| PlanStep {
        label: label.into(),
        needs: needs.iter().map(|n| n.to_string()).collect(),
        optional,
    };
    let mut out = vec![];
    if s.run.own_team && s.run.party_invite {
        out.push(step(
            "Party: main + > spam Invite until none left > alt 1 Accept > alt 2 Accept ... > back to main > close party menu > game mode",
            &["party_plus", "party_invite", "party_invited", "party_accept", "party_close"],
            true,
        ));
    }
    let lobby = lobby_route(mode);
    match routes.lobby.get(lobby) {
        Some(r) => out.push(PlanStep {
            label: format!("Walk: {}", r.map),
            needs: r.patterns(&v),
            optional: false,
        }),
        None => out.push(step(&format!("Walk: no '{lobby}' lobby route"), &[], false)),
    }
    if matches!(mode, Mode::InfinityCastle | Mode::Challenge) {
        out.push(step(
            if mode == Mode::InfinityCastle {
                "Entrance (Cancel shows) > Normal / Hard > Start"
            } else {
                "Entrance (Cancel shows) > Start"
            },
            &["mode_cancel"],
            false,
        ));
    } else {
        match s.run.play {
            PlayChoice::PlayHere => out.push(step(
                "Join Matchmaking > Play Here",
                &["join_matchmaking", "play_here"],
                false,
            )),
            PlayChoice::FindMatch => out.push(step(
                "Join Matchmaking > Find Match (you type the captcha)",
                &["join_matchmaking", "find_match", "captcha"],
                false,
            )),
        }
        if mode == Mode::SummerHunt && s.run.play == PlayChoice::PlayHere {
            let open = if s.run.own_team {
                "open_friends_only"
            } else {
                "open_button"
            };
            out.push(step("Open Portal? > Open", &["open_portal", open], false));
            if s.run.own_team && !s.run.party_invite {
                out.push(step(
                    "Alts walk in and press E at the portal",
                    &["portal_join_e"],
                    false,
                ));
            }
            out.push(step("Portal lobby > Start", &["start_button"], false));
        } else if s.run.play == PlayChoice::PlayHere {
            out.push(step("Start", &["start_button"], false));
        }
    }
    let fixed = matches!(mode, Mode::Story | Mode::Legend | Mode::Raid);
    if fixed {
        out.push(step(
            &format!("Loading: check it's {}", route_map(&s.run.map)),
            &[],
            true,
        ));
    } else {
        out.push(step(
            "Loading: work out the map (random each hour) from its loading screen",
            &[],
            true,
        ));
    }
    out.push(step("Camera top-down", &[], true));
    out.push(step(
        "Settings > Teleport to Spawn > close",
        &[
            "settings_button",
            "tp_spawn",
            "tp_spawn_button",
            "settings_close",
        ],
        true,
    ));
    let walk = if fixed {
        let m = route_map(&s.run.map);
        if routes.find_map(&m).is_some() {
            format!("Map walk: {m}")
        } else {
            format!("Map walk: none for {m} (stays at spawn)")
        }
    } else {
        "Map walk: the detected map's route (none = stays at spawn)".into()
    };
    out.push(step(&walk, &[], true));
    let units = s.units.iter().filter(|u| u.enabled && u.place > 0).count();
    out.push(step("Vote start", &["vote_start"], true));
    out.push(step(
        &format!(
            "Place {units} unit slot(s) on {} client(s): FARM first, UPG [0] badge check, Auto ability on BUFF, cards picked, stops if VICTORY / DEFEAT",
            s.placers.len().max(1)
        ),
        &["unit_placed", "auto_ability"],
        true,
    ));
    out.push(step(
        "Upgrade by priority (R): counts only when UPG [n] goes up, 1-8 or until MAX, FARM first; VICTORY / DEFEAT watched all the time; cards picked",
        &["choose_card", "card_confirm", "upgrade_max", "upg_1", "upg_2"],
        true,
    ));
    out.push(step(
        "End: VICTORY / DEFEAT > Next (your defeat_next_btn picture, else its green)",
        &["victory", "defeat"],
        false,
    ));
    if s.run.return_to_lobby {
        out.push(step(
            "Return to lobby, then repeat",
            &["return_to_lobby"],
            false,
        ));
    } else {
        out.push(step(
            "Main: Next > Replay, wait for Vote start, repeat from Teleport to Spawn",
            &["replay"],
            false,
        ));
    }
    out
}

pub enum Event {
    Log(String),
    Win,
    Loss,
    /// Rewards read off the end screen (sent right before Win / Loss).
    Drops(Vec<(String, u64)>),
    /// A finished run's loot (End step); `reported` = the game loop sent
    /// the Discord loot report itself.
    Run(super::loot::RunStats, bool),
    /// Something needs the user (captcha) - ping them.
    Attention(String),
    /// Step machine: a step went Fatal - the loop paused itself (F4 resumes).
    Fatal(String),
    /// Captcha on a client: the watchdog paused the macro until it's gone.
    Captcha(String),
    /// The captcha is gone: the watchdog un-paused.
    CaptchaGone,
    /// Disconnect recovery: what happened (Reconnecting / back / failed).
    Reconnect(String),
    Done(String),
}

#[derive(Clone)]
pub struct RunCtx {
    pub arb: ArbiterHandle,
    pub control: Control,
    pub main: Hwnd,
    /// Every client: (hwnd, screen). Index = client number - 1.
    pub clients: Vec<(Hwnd, LiveScreen)>,
    pub settings: Settings,
    /// Swapped by hot-reload when your route files change.
    pub routes: crate::hotreload::LiveRoutes,
    pub placements: Placements,
    pub tx: Sender<Event>,
    /// 1.0 = real timing; tests use a small value.
    pub speed: f32,
    /// Spots that failed before (shared with the app, saved to disk).
    pub memory: std::sync::Arc<std::sync::Mutex<crate::spotmem::SpotMemory>>,
    /// Pick up at this saved step (a restart mid-stage), see `resume`.
    pub resume: Option<super::resume::Saved>,
    /// Discord loot report after every run: (webhook URL, ping user id).
    /// None = off.
    pub loot_hook: Option<(String, String)>,
    /// Run count + totals since Start.
    pub session: std::sync::Arc<std::sync::Mutex<super::loot::Session>>,
    /// Recovery watchdog's findings (disconnect / captcha per client).
    pub watch: std::sync::Arc<super::watchdog::Watch>,
    /// Step machine: the running step's hard deadline. `check()` / `sleep()`
    /// fail with "step timed out" past it (the old loop never sets it).
    pub step_deadline: std::sync::Arc<std::sync::Mutex<Option<crate::control::Deadline>>>,
}

/// The End step's once-only parts, kept across its retries: Win / Loss and
/// the end-screen read happen once, the loot report once.
#[derive(Default)]
pub(crate) struct EndOnce {
    pub reported: std::sync::atomic::AtomicBool,
    pub rewarded: std::sync::atomic::AtomicBool,
    /// End-screen drops + pearls, as first read.
    pub read: std::sync::Mutex<(Vec<(String, u64)>, Option<u64>)>,
}

impl RunCtx {
    /// "Challenge every 30 min" is on, this isn't a Challenge run already,
    /// and a reset (:00 / :30 local) has passed since the last one.
    pub(crate) fn challenge_owed(&self) -> bool {
        self.settings.run.challenge_every_30
            && self.settings.run.mode != Mode::Challenge
            && super::challenge::due()
    }

    /// One Challenge run from the lobby (its own team if it has one), then
    /// back to the lobby; the farm carries on after. A failed Challenge is
    /// logged and skipped until the next reset - the farm goes on.
    pub(crate) fn run_challenge_now(&self) -> R<()> {
        super::challenge::mark_done();
        self.log("Challenge reset - one Challenge run, then back to the farm");
        let mut c = self.clone();
        c.settings.team_for_mode(Mode::Challenge);
        c.settings.run.mode = Mode::Challenge;
        c.settings.run.challenge_every_30 = false;
        c.settings.run.return_to_lobby = true;
        c.resume = None;
        match c.run_loop(
            Some(StepLimit {
                runs: 1,
                until: None,
            }),
            true,
        ) {
            Ok(()) => self.log("Challenge done - back to the farm"),
            Err(e) if e == "stopped" || e.starts_with(DISCONNECTED) => return Err(e),
            Err(e) => {
                self.log(format!("WARN Challenge failed: {e} - back to the farm"));
                if self.where_am_i() != "lobby" {
                    let _ = self.return_to_lobby_now();
                }
            }
        }
        Ok(())
    }
}

/// Pause between placing and upgrading, after placement mode is closed.
pub const PLACE_TO_UPGRADE_MS: u64 = 1500;

/// Next button pictures, tried in this order (capture with F6).
pub const NEXT_PICS: &[&str] = &["defeat_next_btn", "victory_next_btn", "next_button"];

/// Start of what `check()` returns when a client is disconnected.
pub const DISCONNECTED: &str = "disconnected";

/// What `check()` / `sleep()` return past the step deadline.
pub const STEP_TIMED_OUT: &str = "step timed out";

type R<T> = Result<T, String>;

/// OCR of the whole screen costs a moment: at most every 2 s per client.
fn ocr_due(h: Hwnd) -> bool {
    ocr_due_for(h, 0)
}

/// Same, per purpose (0 = cards / weather, 1 = VICTORY / DEFEAT).
fn ocr_due_for(h: Hwnd, what: u8) -> bool {
    static LAST: std::sync::Mutex<Vec<(Hwnd, u8, Instant)>> = std::sync::Mutex::new(Vec::new());
    let mut v = LAST.lock().unwrap();
    match v.iter_mut().find(|e| e.0 == h && e.1 == what) {
        Some(e) if e.2.elapsed() < Duration::from_millis(if what == 0 { 1000 } else { 2000 }) => {
            false
        }
        Some(e) => {
            e.2 = Instant::now();
            true
        }
        None => {
            v.push((h, what, Instant::now()));
            true
        }
    }
}

/// Placed units as JSON (client number, slot, x, y) for the step state.
pub(crate) fn placed_json(
    clients: &[(Hwnd, super::LiveScreen)],
    placed: &[(Hwnd, usize, Point)],
) -> serde_json::Value {
    placed
        .iter()
        .map(|(h, slot, p)| {
            let n = clients.iter().position(|c| c.0 == *h).unwrap_or(0) + 1;
            serde_json::json!([n, slot, p.x, p.y])
        })
        .collect()
}

/// When a weather shift banner was last seen: a card pick right after it
/// uses the weather order (buffs first).
static LAST_SHIFT: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

/// Accounts still upgrading (0 = all done: idle clicks keep things awake).
/// After closing a unit panel: its closing animation eats clicks, so the
/// next unit's click waits this long (your log: the first click on a unit
/// right after the last one's Cancel never opened its panel).
const PANEL_CLOSE_MS: u64 = 700;

/// A card / weather vote screen was seen: upgrade clicks on that client
/// wait until this time (the vote screen stays ~10 s).
static CARDS_UP: std::sync::Mutex<Vec<(Hwnd, Instant)>> = std::sync::Mutex::new(Vec::new());

/// Mark client `h`'s card / weather vote screen as up for the next 12 s.
fn cards_up(h: Hwnd) {
    let mut v = CARDS_UP.lock().unwrap_or_else(|e| e.into_inner());
    v.retain(|(c, _)| *c != h);
    v.push((h, Instant::now() + Duration::from_secs(12)));
}

/// Where to click unit placed at `at` on try `n`: the spot itself, then
/// a bit above it (the body / nameplate - the camera tilt puts the model
/// above its ground point), then up-left and up-right of that.
pub(crate) fn unit_click_point(at: Point, n: u32) -> Point {
    let (dx, dy) = match n {
        0 => (0, 0),
        1 => (0, -14),
        2 => (-12, -24),
        _ => (12, -24),
    };
    Point::new((at.x + dx).clamp(1, 798), (at.y + dy).clamp(1, 597))
}

/// Priority upgrade, no money for the next level: wait this long (panel
/// closed) before trying the same unit again.
const CASH_WAIT_MS: u64 = 4000;
/// ... but at most this long on one level: then it's counted as done (a
/// MAX the macro can't see would otherwise stall the upgrades for good).
const CASH_WAIT_LIMIT: Duration = Duration::from_secs(10 * 60);

/// Re-clicks of a unit whose panel didn't open, before moving on.
const PANEL_RETRIES: u32 = 3;
static UPGRADING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Clients in the middle of a unit sequence (open unit .. Cancel, while
/// placing or upgrading). Anti-AFK waits for these: its clicks + Cancel
/// would close the unit panel halfway through a read.
static SEQ: std::sync::Mutex<Vec<Hwnd>> = std::sync::Mutex::new(Vec::new());
static SEQ_CV: std::sync::Condvar = std::sync::Condvar::new();

/// Held for one unit sequence on a client; dropping it frees the client.
pub(crate) struct SeqGuard(Hwnd);
impl Drop for SeqGuard {
    fn drop(&mut self) {
        if let Ok(mut v) = SEQ.lock() {
            v.retain(|h| *h != self.0);
        }
        SEQ_CV.notify_all();
    }
}
/// Take client `h` for a unit sequence, waiting up to `max` for whoever has
/// it (None = still busy after `max`).
pub(crate) fn seq_lock(h: Hwnd, max: Duration) -> Option<SeqGuard> {
    let until = Instant::now() + max;
    let mut v = SEQ.lock().unwrap_or_else(|e| e.into_inner());
    while v.contains(&h) {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        v = SEQ_CV
            .wait_timeout(v, left.min(Duration::from_millis(200)))
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
    v.push(h);
    Some(SeqGuard(h))
}

fn keys_ref(k: &Keys) -> &Keys {
    k
}

/// When a card was last picked (any client) - one pick does it for all.
static LAST_PICK: std::sync::Mutex<Vec<(Hwnd, Instant)>> = std::sync::Mutex::new(Vec::new());

impl RunCtx {
    pub(crate) fn log(&self, s: impl Into<String>) {
        let s = s.into();
        super::trace::log(&s);
        let _ = self.tx.send(Event::Log(s));
    }

    /// Failure dump of every client: frame + `crops` (and their rects) on each
    /// one, marked `failing_client` for `h`. Logged.
    pub(crate) fn dump_failure(
        &self,
        step: &str,
        reason: &str,
        h: Option<Hwnd>,
        crops: Vec<super::dump::Crop>,
    ) {
        let shots = self
            .clients
            .iter()
            .enumerate()
            .map(|(i, (ch, scr))| super::dump::ClientShot {
                n: i + 1,
                frame: scr.frame(),
                // the same rects on every client (same window size), so the
                // other clients' crops show what they had there at the time
                crops: crops
                    .iter()
                    .map(|c| {
                        let mut c = c.clone();
                        let failing = h.is_none() || Some(*ch) == h;
                        if let Some(o) = c.info.as_object_mut() {
                            o.insert("failing_client".into(), failing.into());
                        } else {
                            c.info =
                                serde_json::json!({ "failing_client": failing, "info": c.info });
                        }
                        c
                    })
                    .collect(),
            })
            .collect();
        if let Some(d) = super::dump::dump(step, reason, shots) {
            let _ = self
                .tx
                .send(Event::Log(format!("failure dump: {}", d.display())));
        }
    }
    pub(crate) fn screen(&self, h: Hwnd) -> &LiveScreen {
        &self.clients.iter().find(|c| c.0 == h).expect("client").1
    }
    pub(crate) fn check(&self) -> R<()> {
        self.control
            .checkpoint()
            .map_err(|_| "stopped".to_string())?;
        // the watchdog saw Roblox's disconnect box: stop what we're doing,
        // the loop reconnects
        if let Some(i) = self.watch.stopping_disconnect() {
            return Err(format!("{DISCONNECTED} (client {})", i.client));
        }
        self.past_deadline()
    }
    /// Err once the step machine's deadline for this step has passed.
    fn past_deadline(&self) -> R<()> {
        let mut d = self.step_deadline.lock().unwrap_or_else(|e| e.into_inner());
        if d.as_mut().is_some_and(|dl| dl.expired()) {
            Err(STEP_TIMED_OUT.into())
        } else {
            Ok(())
        }
    }
    pub(crate) fn sleep(&self, ms: u64) -> R<()> {
        self.past_deadline()?;
        self.control
            .sleep((ms as f32 * self.speed) as u64)
            .map_err(|_| "stopped".to_string())
    }
    fn secs(&self, s: f64) -> f64 {
        s * self.speed as f64
    }
    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }
    /// Wait up to `ms` for any of `names` on `h`; returns (name, centre).
    fn wait_any(&self, h: Hwnd, names: &[&str], ms: u64) -> R<Option<(String, Point)>> {
        let s = self.screen(h);
        let mut dl = self
            .control
            .deadline(self.secs(ms as f64 / 1000.0).max(0.05));
        loop {
            self.check()?;
            for n in names {
                if let Some(p) = s.find(&[n.to_string()], None) {
                    return Ok(Some((n.to_string(), p)));
                }
            }
            if dl.expired() {
                // a waited-for pattern never showed: dump it when asked to
                if ms >= 1000 {
                    let crops: Vec<super::dump::Crop> = names
                        .iter()
                        .filter_map(|n| s.pattern(n))
                        .filter(|p| p.dump_on_miss || self.settings.dump_on_miss)
                        .map(|p| super::dump::Crop {
                            what: p.name.clone(),
                            rect: p.search,
                            info: serde_json::json!({"pattern": p.name, "waited_ms": ms, "found": false}),
                        })
                        .collect();
                    if !crops.is_empty() {
                        self.dump_failure(
                            &super::trace::current_name(),
                            &format!("{} never showed", names.join(" / ")),
                            Some(h),
                            crops,
                        );
                    }
                }
                return Ok(None);
            }
            self.sleep(150)?;
        }
    }
    /// Like `wait_any` for one name, but looks over the whole screen (and
    /// reads the label with OCR as a backup).
    fn wait_wide(&self, h: Hwnd, name: &str, ms: u64) -> R<Option<Point>> {
        let s = self.screen(h);
        let names = [name.to_string()];
        let mut dl = self
            .control
            .deadline(self.secs(ms as f64 / 1000.0).max(0.05));
        loop {
            self.check()?;
            if let Some(p) = s.find_wide(&names) {
                return Ok(Some(p));
            }
            if dl.expired() {
                return Ok(None);
            }
            self.sleep(200)?;
        }
    }
    fn click(&self, h: Hwnd, p: Point, why: &str) -> R<()> {
        crate::debugbus::click(h, p, why);
        self.arb
            .run(Batch::new(h, why).click(p.x, p.y))
            .map_err(|e| format!("{why}: {e}"))?;
        // give the game time to react before the next look / click
        self.sleep(self.settings.step_delay_ms)?;
        Ok(())
    }
    /// Wait for `name` then click it; error when it never shows.
    fn click_when(&self, h: Hwnd, name: &str, ms: u64) -> R<()> {
        // fixed button seen before: straight to its spot
        if let Some(p) = super::buttonmem::get(name) {
            return self.click(h, p, name);
        }
        match self.wait_any(h, &[name], ms)? {
            Some((_, p)) => {
                self.click(h, p, name)?;
                // a toggle stays on screen: clicking again would undo it
                if self.settings.click_mode == crate::input::ClickMode::Safe
                    && !super::buttonmem::is_toggle(name)
                {
                    // still there, same place = the click didn't register
                    self.sleep(400)?;
                    if let Some(q) = self.screen(h).find(&[name.to_string()], None) {
                        if (q.x - p.x).abs() <= 6 && (q.y - p.y).abs() <= 6 {
                            self.log(format!("'{name}' click didn't take - clicking again"));
                            self.click(h, q, name)?;
                        }
                    }
                }
                Ok(())
            }
            None => Err(format!("'{name}' never showed up")),
        }
    }

    /// Lobby -> in the stage.
    pub fn enter_stage(&self) -> R<()> {
        let s = &self.settings;
        let mode = s.run.mode;
        let key = lobby_route(mode);
        let routes = self.routes.get();
        let route = routes
            .lobby
            .get(key)
            .ok_or(format!("no lobby route '{key}'"))?;
        self.log(format!("walking: {}", route.map));
        let batch = route
            .to_batch(self.main, self.screen(self.main), &vars(s))
            .map_err(|e| e.to_string())?;
        self.arb
            .run(batch)
            .map_err(|e| format!("lobby walk: {e}"))?;
        // Infinity Castle / Challenge: walk into the pod, no matchmaking dialog
        if matches!(mode, Mode::InfinityCastle | Mode::Challenge) {
            return self.enter_pod(mode, route);
        }
        if mode == Mode::Dungeon {
            return Err("Dungeon: getting into a dungeon from the lobby isn't set up yet (needs your screenshots of it) - the floor itself runs on the step machine once you're in".into());
        }
        // Join Matchmaking?
        match s.run.play {
            PlayChoice::PlayHere => self.click_when(self.main, "play_here", 6000)?,
            PlayChoice::FindMatch => {
                self.click_when(self.main, "find_match", 6000)?;
                if self.wait_any(self.main, &["captcha"], 3000)?.is_some() {
                    let _ = self.tx.send(Event::Attention(
                        "Captcha - type it in Roblox, the macro waits".into(),
                    ));
                    // wait until the captcha is gone (typed) - up to 2 min
                    let mut dl = self.control.deadline(self.secs(120.0));
                    while self
                        .screen(self.main)
                        .find(&Self::names(&["captcha"]), None)
                        .is_some()
                    {
                        if dl.expired() {
                            return Err("captcha not typed in time".into());
                        }
                        self.sleep(500)?;
                    }
                }
                self.log("matchmaking - waiting to load");
                return Ok(());
            }
        }
        if mode == Mode::SummerHunt {
            let open = if s.run.own_team {
                "open_friends_only"
            } else {
                "open_button"
            };
            if self.wait_any(self.main, &["open_portal"], 5000)?.is_some() {
                self.click_when(self.main, open, 4000)?;
            }
            // in a party the alts come along by themselves - they don't move
            if s.run.own_team && !s.run.party_invite {
                self.alts_join_portal()?;
                self.to_main()?;
            }
        }
        self.click_when(self.main, "start_button", 10000)?;
        self.log("started - loading");
        Ok(())
    }

    /// Infinity Castle / Challenge (from the AHK macro): walk until the pod's
    /// Cancel button shows (walk again up to 3x), castle: pick Normal / Hard,
    /// then Start (your start_button, else the spot above Cancel).
    fn enter_pod(&self, mode: Mode, route: &crate::movement::Route) -> R<()> {
        let s = &self.settings;
        let mut tries = 0;
        loop {
            if self
                .wait_any(self.main, &["mode_cancel", "start_button"], 8000)?
                .is_some()
            {
                break;
            }
            tries += 1;
            if tries >= 3 {
                return Err("never reached the entrance (no Cancel / Start) after 3 walks".into());
            }
            self.log("entrance not reached - walking again");
            let b = route
                .to_batch(self.main, self.screen(self.main), &vars(s))
                .map_err(|e| e.to_string())?;
            self.arb.run(b).map_err(|e| format!("lobby walk: {e}"))?;
        }
        if mode == Mode::InfinityCastle {
            let (p, name) = if s.run.castle_hard {
                (Point::new(485, 375), "Hard")
            } else {
                (Point::new(418, 375), "Normal")
            };
            self.log(format!("Infinity Castle: {name}"));
            self.click(self.main, p, "castle difficulty")?;
        }
        let main = self.screen(self.main);
        let start = main
            .find_wide(&Self::names(&["start_button"]))
            .or_else(|| {
                main.find_wide(&Self::names(&["mode_cancel"]))
                    .map(|c| Point::new(c.x - 10, c.y - 120))
            })
            .ok_or("no Start button at the entrance")?;
        self.click(self.main, start, "start")?;
        self.log("started - loading");
        Ok(())
    }

    /// Alts walk into the main's portal and press E.
    fn alts_join_portal(&self) -> R<()> {
        let e = Key::parse("e").expect("e");
        let w = Key::parse("w").expect("w");
        for (h, scr) in self.clients.iter().filter(|c| c.0 != self.main) {
            self.log("alt joining the portal");
            let probe = {
                let s = scr.clone();
                crate::input::Probe::new("portal_join_e", move || {
                    s.find(&["portal_join_e".to_string()], None)
                })
            };
            let b = Batch::new(*h, "alt: join portal")
                .then(Action::KeyDown(w))
                .then(Action::WaitFor {
                    probe,
                    timeout_ms: 12000,
                    required: false,
                })
                .then(Action::KeyUp(w))
                .then(Action::KeyTap {
                    key: e,
                    hold_ms: 80,
                });
            let mut b = b;
            b.long = true;
            self.arb.run(b).map_err(|e| format!("alt join: {e}"))?;
        }
        Ok(())
    }

    /// Card pick screen open on `h`? Take the green card, else the middle
    /// one, then Confirm. Returns true when it picked.
    pub fn handle_cards(&self, h: Hwnd) -> R<bool> {
        let scr = self.screen(h);
        // card screen: your choose_card capture, or OCR reading "Choose a card"
        let mut weather = String::new();
        let mut shift: Option<String> = None;
        let mut shift_cards = false;
        let seen =
            if scr.has("choose_card") && scr.find(&Self::names(&["choose_card"]), None).is_some() {
                true
            } else if ocr_due(h) {
                let f = scr.frame();
                let text = f.as_ref().and_then(|f| scr.ocr_text_fast(f));
                // Mahoraga's adaptation panel shares this OCR read
                if let (Some(t), Some(f)) = (&text, &f) {
                    if self.settings.adaptations.enabled && super::adaptation::showing(t) {
                        self.handle_adaptation(h, f)?;
                        return Ok(true);
                    }
                }
                match text {
                    Some(t) => match super::card_screen_text(&t) {
                        Some(w) => {
                            weather = w;
                            true
                        }
                        None => {
                            shift = super::weather_shift_text(&t);
                            // cards to vote on with it ("Vote ends in")
                            shift_cards = t.to_lowercase().contains("vote");
                            false
                        }
                    },
                    None => false,
                }
            } else {
                false
            };
        if shift.is_some() || seen {
            cards_up(h);
        }
        if let Some(w) = shift {
            self.weather_shift(h, &w)?;
            // the shift comes with 3 cards to vote on: your weather list,
            // once per account
            let voted = LAST_PICK
                .lock()
                .unwrap()
                .iter()
                .any(|(c, t)| *c == h && t.elapsed() < Duration::from_secs(30));
            if !voted && shift_cards {
                {
                    let mut l = LAST_PICK.lock().unwrap();
                    l.retain(|(c, _)| *c != h);
                    l.push((h, Instant::now()));
                }
                self.weather_cards(h)?;
                return Ok(true);
            }
            return Ok(false);
        }
        if !seen {
            return Ok(false);
        }
        // every account picks (and votes) for itself. This one picked in the
        // last 45 s and the screen is still up: it's the vote, others are
        // still voting - wait (a bit) for it to close instead of picking again
        let since = LAST_PICK
            .lock()
            .unwrap()
            .iter()
            .find(|(c, _)| *c == h)
            .map(|(_, t)| t.elapsed());
        if since.is_some_and(|t| t < Duration::from_secs(45)) {
            let mut dl = self.control.deadline(self.secs(8.0));
            while !dl.expired() {
                self.sleep(700)?;
                let up = (scr.has("choose_card")
                    && scr.find(&Self::names(&["choose_card"]), None).is_some())
                    || scr
                        .frame()
                        .and_then(|f| scr.ocr_text_fast(&f))
                        .is_some_and(|t| super::card_screen_text(&t).is_some());
                if !up {
                    break;
                }
            }
            return Ok(true);
        }
        {
            let mut l = LAST_PICK.lock().unwrap();
            l.retain(|(c, _)| *c != h);
            l.push((h, Instant::now()));
        }
        // close the unit panel first if one is open (its X is on screen)
        if let Some(p) = scr
            .has("unit_close")
            .then(|| scr.find_wide(&Self::names(&["unit_close"])))
            .flatten()
        {
            self.click(h, p, "close unit panel")?;
            self.sleep(400)?;
        }
        let Some(f) = scr.frame() else {
            return Ok(false);
        };
        if weather.is_empty() {
            weather = scr.ocr_card_screen(&f).unwrap_or_default();
        }
        if !weather.is_empty() {
            self.log(format!("card screen - weather: {weather}"));
        }
        // read the three card titles and take the first one in your order
        let names = Self::read_cards(scr, &f);
        // weather shift cards (Grounded, Mud, ...) on this screen: your
        // weather list decides (Settings > Weather)
        let shift = LAST_SHIFT
            .lock()
            .unwrap()
            .is_some_and(|t| t.elapsed() < Duration::from_secs(90));
        if names.iter().all(|n| n.is_none())
            && (shift
                || Self::read_weather_cards(scr, &f)
                    .iter()
                    .any(|n| n.is_some()))
        {
            self.weather_cards(h)?;
            return Ok(true);
        }
        let prio = if self.settings.run.card_priority.is_empty() {
            super::cards::default_priority()
        } else {
            self.settings.run.card_priority.clone()
        };
        // the highest card of your list on offer (exact name first, else
        // the same card at another tier)
        let pick = super::cards::choose(&names, &prio);
        let (i, p, fallback) = match pick {
            Some(i) => {
                let b = super::cards::CARD_BOXES[i];
                (i, Point::new(b.x + b.w / 2, b.y + b.h / 2), "")
            }
            None => {
                let (p, green) = super::cards::pick(&f);
                let i = super::cards::CARD_BOXES
                    .iter()
                    .position(|b| p.x >= b.x && p.x < b.x + b.w)
                    .unwrap_or(1);
                (
                    i,
                    p,
                    if green {
                        "the green one"
                    } else {
                        "the middle one"
                    },
                )
            }
        };
        self.log(super::cards::picker_line(
            "priority list",
            &names,
            &prio,
            pick,
            i,
            fallback,
        ));
        self.vote_card(h, i, p)?;
        Ok(true)
    }

    /// Vote for card `i` (centre `p`): there's no confirm button - clicking
    /// the card IS the vote (clicking again would take it back). The count
    /// under it should go up; if it didn't, click once more. Then wait for
    /// the vote to end (the screen closes by itself).
    fn vote_card(&self, h: Hwnd, i: usize, p: Point) -> R<()> {
        let scr = self.screen(h);
        let count = || -> Option<u32> {
            let f = scr.frame()?;
            let t = scr.ocr_rect(&f, super::cards::count_box(i))?;
            let d: String = t
                .chars()
                .map(|c| match c {
                    'O' | 'o' => '0',
                    'l' | 'I' | '|' => '1',
                    c => c,
                })
                .filter(|c| c.is_ascii_digit())
                .collect();
            d.parse().ok()
        };
        let before = count();
        self.click(h, p, "vote card")?;
        self.sleep(900)?;
        let after = count();
        match (before, after) {
            (Some(b), Some(a)) if a > b => self.log(format!("vote counted ({b} -> {a})")),
            (Some(b), Some(a)) if a == b => {
                self.log("vote not counted yet - clicking the card once more");
                self.click(h, p, "vote card (again)")?;
            }
            _ => {}
        }
        // the vote ends by itself (timer under the cards); not waited for
        // here so the other accounts get to vote in time
        Ok(())
    }

    /// The three card names on a card screen (OCR): each title box, else
    /// the names found anywhere on the cards; red cards take their tier
    /// from the pearls line. None = that card not readable.
    fn read_cards(scr: &super::LiveScreen, f: &crate::capture::Frame) -> [Option<String>; 3] {
        let mut names: [Option<String>; 3] = std::array::from_fn(|i| {
            scr.ocr_rect(f, super::cards::title_box(i))
                .and_then(|t| super::cards::card_name(&t))
        });
        // a title box missed: look for the names anywhere on the cards
        if names.iter().any(|n| n.is_none()) {
            if let Some(lines) = scr.ocr_lines(f) {
                let more = super::cards::names_from_lines(&lines);
                for i in 0..3 {
                    if names[i].is_none() {
                        names[i] = more[i].clone();
                    }
                }
            }
        }
        // red cards: the tier from the pearls line beats the title's I/II/III
        for i in 0..3 {
            if let Some(n) = names[i].clone() {
                if let Some(t) = scr
                    .ocr_rect(f, super::cards::body_box(i))
                    .and_then(|b| super::cards::tier_from_pearls(&b))
                {
                    names[i] = Some(super::cards::with_tier(&n, t));
                }
            }
        }
        names
    }

    /// The three weather shift card names (Grounded, Mud, ...): each title
    /// strip, else the names found anywhere on the cards.
    fn read_weather_cards(
        scr: &super::LiveScreen,
        f: &crate::capture::Frame,
    ) -> [Option<String>; 3] {
        let mut names: [Option<String>; 3] = std::array::from_fn(|i| {
            scr.ocr_rect(f, super::cards::title_box(i))
                .and_then(|t| super::weather::card_name(&t))
        });
        if names.iter().any(|n| n.is_none()) {
            if let Some(lines) = scr.ocr_lines(f) {
                let more = super::weather::names_from_lines(&lines);
                for i in 0..3 {
                    if names[i].is_none() {
                        names[i] = more[i].clone();
                    }
                }
            }
        }
        names
    }

    /// "The weather shifts..." cards: the one highest on your weather list
    /// (Settings > Weather); none of them on it = the leftmost.
    fn weather_cards(&self, h: Hwnd) -> R<()> {
        let scr = self.screen(h);
        let Some(f) = scr.frame() else { return Ok(()) };
        let names = Self::read_weather_cards(scr, &f);
        let prio = super::weather::load();
        let (i, rank) = super::weather::pick(&names, &prio);
        self.log(format!(
            "weather shift: {}",
            super::cards::picker_line(
                "weather list",
                &names,
                &prio,
                rank.map(|_| i),
                i,
                "the leftmost",
            )
        ));
        let b = super::cards::CARD_BOXES[i];
        self.click(h, Point::new(b.x + b.w / 2, b.y + b.h / 2), "weather card")?;
        // the pick counts once Vote (green, under the cards) is pressed:
        // your weather_vote picture, else the button by its colour
        // a moment for the pick to register and Vote to show
        self.sleep(500)?;
        let mut dl = self.control.deadline(self.secs(2.5));
        loop {
            let at = scr
                .has("weather_vote")
                .then(|| scr.find_picture_wide(&Self::names(&["weather_vote"])))
                .flatten()
                .or_else(|| scr.frame().and_then(|f| super::weather_vote_button(&f)));
            if let Some(p) = at {
                self.click(h, p, "weather vote")?;
                self.log("weather shift: Vote pressed");
                break;
            }
            if dl.expired() {
                self.log("weather shift: no Vote button showed - the card click is all it got");
                break;
            }
            self.sleep(300)?;
        }
        Ok(())
    }

    /// Mahoraga's "Choose an Adaptation": take the first choice in your
    /// order for the badge shown that isn't grey. Never stops the run.
    fn handle_adaptation(&self, h: Hwnd, f: &crate::capture::Frame) -> R<()> {
        let scr = self.screen(h);
        let Some(lines) = scr.ocr_lines(f) else {
            return Ok(());
        };
        let Some(panel) = super::adaptation::read(&lines, f) else {
            return Ok(());
        };
        let badge = panel.badge.map(|b| b.label()).unwrap_or("?");
        match super::adaptation::choose(&panel, &self.settings.adaptations) {
            Some(c) => {
                self.log(format!("adaptation ({badge}): {}", c.name));
                if let Err(e) = self.click(h, c.at, "adaptation") {
                    self.log(format!("adaptation choice failed: {e}"));
                }
            }
            None if panel.choices.is_empty() => {
                self.log("adaptation panel up but its choices aren't readable")
            }
            None => self.log("adaptation: every choice is grey - nothing to pick"),
        }
        Ok(())
    }

    /// Weather shift banner: close the unit panel (if open), log it. Once
    /// per 30 s.
    fn weather_shift(&self, h: Hwnd, weather: &str) -> R<()> {
        {
            static LAST: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
            let mut l = LAST.lock().unwrap();
            if l.is_some_and(|t| t.elapsed() < Duration::from_secs(30)) {
                return Ok(());
            }
            *l = Some(Instant::now());
        }
        *LAST_SHIFT.lock().unwrap() = Some(Instant::now());
        let scr = self.screen(h);
        if let Some(p) = scr
            .has("unit_close")
            .then(|| scr.find_wide(&Self::names(&["unit_close"])))
            .flatten()
        {
            self.click(h, p, "close unit panel")?;
            self.sleep(400)?;
        }
        self.log(format!(
            "weather shift: {}",
            if weather.is_empty() { "?" } else { weather }
        ));
        Ok(())
    }

    /// Card screen on any client (alts first); back to the main after an alt.
    pub(crate) fn cards_all(&self) -> R<()> {
        let all: Vec<Hwnd> = self.clients.iter().map(|c| c.0).collect();
        let mut alt_done = false;
        // every account picks its own card (all of them vote)
        for h in self.main_last(all) {
            if self.handle_cards(h)? && h != self.main {
                alt_done = true;
            }
        }
        if alt_done {
            self.to_main()?;
        }
        Ok(())
    }

    /// Main invites every alt into its party, each alt accepts.
    pub fn party_up(&self) -> R<()> {
        let alts: Vec<Hwnd> = self
            .clients
            .iter()
            .map(|c| c.0)
            .filter(|h| *h != self.main)
            .collect();
        if alts.is_empty() {
            return Ok(());
        }
        let main = self.screen(self.main);
        if !main.has("party_plus") || !main.has("party_invite") {
            self.log("party: capture party_plus + party_invite first - skipping");
            return Ok(());
        }
        self.log(format!("party: inviting {} alt(s)", alts.len()));
        // 1. click + once, then stop looking for it
        self.click_when(self.main, "party_plus", 5000)?;
        // 2. wait for the first Invite button
        let mut dl = self.control.deadline(self.secs(8.0));
        while main.find_all_wide("party_invite").is_empty() && !dl.expired() {
            self.sleep(400)?;
        }
        if main.find_all_wide("party_invite").is_empty() {
            self.log("party: no Invite button showed up - carrying on without a party");
            return Ok(());
        }
        // 3. spam-click every Invite until none is left (each turns into Invited)
        let sent = self.spam_invites()?;
        let invited = !main.has("party_invited")
            || main.find(&Self::names(&["party_invited"]), None).is_some();
        self.log(if invited {
            format!("party: {sent} invite click(s), no Invite left, Invited shown")
        } else {
            "party: no Invite left but no 'Invited' either - trying the alts anyway".into()
        });
        // 4. one alt at a time: swap to it and Accept, through every alt.
        //    Only if one missed the invite: re-invite on the main, retry once.
        let mut waiting: Vec<Hwnd> = alts.clone();
        for round in 1..=2 {
            let mut missed = vec![];
            for (i, h) in waiting.iter().enumerate() {
                self.log(format!(
                    "party: alt {} of {} - accepting",
                    i + 1,
                    waiting.len()
                ));
                match self.wait_any(*h, &["party_accept"], 15000)? {
                    Some((_, _)) => {
                        // click_when re-clicks if the pop-up is still there
                        if let Err(e) = self.click_when(*h, "party_accept", 3000) {
                            self.log(format!("party: {e}"));
                        }
                        self.sleep(1000)?;
                        if self
                            .screen(*h)
                            .find(&Self::names(&["party_accept"]), None)
                            .is_some()
                        {
                            missed.push(*h);
                        }
                    }
                    None => missed.push(*h),
                }
            }
            if missed.is_empty() {
                break;
            }
            if round == 2 {
                self.log(format!(
                    "party: {} alt(s) still not in the party",
                    missed.len()
                ));
                break;
            }
            // only if an alt missed it: back to the main, invite again, retry those
            self.to_main()?;
            self.spam_invites()?;
            self.log(format!(
                "party: {} alt(s) didn't get the invite{}",
                missed.len(),
                if round == 1 { " - inviting again" } else { "" }
            ));
            waiting = missed;
        }
        // 5. back to the main, close the party menu, then Play / Areas / Create lobby
        self.to_main()?;
        if main.has("party_close") {
            match self.wait_any(self.main, &["party_close"], 3000)? {
                Some((_, p)) => self.click(self.main, p, "close party menu")?,
                None => self.log("party: close button not seen - menu left open"),
            }
        } else {
            self.log("party: capture party_close to close the party menu");
        }
        self.log("party: done - back on the main, going to the game mode");
        Ok(())
    }

    /// On the main: click every Invite button until none is left for a
    /// moment. Returns how many were clicked.
    fn spam_invites(&self) -> R<u32> {
        let main = self.screen(self.main);
        let mut sent = 0;
        let mut empty_scans = 0;
        // scan the whole screen for every Invite (one per alt), click them all,
        // scan again - until a couple of scans in a row find none
        for _ in 0..10 {
            self.check()?;
            let all = main.find_all_wide("party_invite");
            if all.is_empty() {
                empty_scans += 1;
                if empty_scans >= 2 {
                    break;
                }
                self.sleep(700)?;
                continue;
            }
            empty_scans = 0;
            self.log(format!("party: {} Invite button(s) on screen", all.len()));
            for p in all {
                self.arb
                    .run(Batch::new(self.main, "invite").click(p.x, p.y))
                    .map_err(|e| format!("invite: {e}"))?;
                sent += 1;
                self.sleep(400)?;
            }
            self.sleep(600)?;
        }
        Ok(sent)
    }

    /// Bring the main client back to the front (after working on alts).
    pub(crate) fn to_main(&self) -> R<()> {
        self.arb
            .run(Batch::new(self.main, "back to main"))
            .map_err(|e| format!("back to main: {e}"))?;
        self.sleep(300)?;
        Ok(())
    }

    /// Alts first, the main last - so the sequence ends on the main.
    fn main_last(&self, v: Vec<Hwnd>) -> Vec<Hwnd> {
        let mut v: Vec<Hwnd> = v.into_iter().filter(|h| *h != self.main).collect();
        v.push(self.main);
        v
    }

    /// Clients that move / place: the main, then every placing client.
    pub(crate) fn movers(&self) -> Vec<Hwnd> {
        let mut v = vec![self.main];
        for n in &self.settings.placers {
            if let Some((h, _)) = self.clients.get(n.saturating_sub(1)) {
                if !v.contains(h) {
                    v.push(*h);
                }
            }
        }
        v
    }

    /// From the lobby leaving (or Replay) until Vote Start is pressed:
    /// work out the map while it loads, top-down camera, Teleport to Spawn,
    /// walk the map's route, then Vote Start. Returns the map ("" = unknown).
    /// Stage set-up: map, camera, Teleport to Spawn, the map's walk, vote.
    pub fn stage_setup(&self, prev_map: &str) -> R<String> {
        let map = self.stage_prepare(prev_map)?;
        self.stage_walk(&map)?;
        self.stage_vote()?;
        Ok(map)
    }

    /// Stage set-up up to the walk: wait for the stage, spot the map, camera,
    /// map picture, Teleport to Spawn (new map only).
    pub fn stage_prepare(&self, prev_map: &str) -> R<String> {
        let s = &self.settings;
        let main = self.screen(self.main);
        let gone = Self::names(&[
            "victory",
            "defeat",
            "replay",
            "start_button",
            "waiting_for_players",
            "join_matchmaking",
            "captcha",
        ]);
        let mut dl = self.control.deadline(self.secs(90.0));
        while main.find(&gone, None).is_some() {
            if dl.expired() {
                return Err("the end / lobby screen is still up after 90 s".into());
            }
            self.sleep(500)?;
        }
        let fixed = matches!(s.run.mode, Mode::Story | Mode::Legend | Mode::Raid);
        let detect = main.with_prefix("loading_");
        let vote = Self::names(&["vote_start"]);
        let mut map: Option<String> = None;
        // map names the OCR can recognise: every map with a route + your loading_ captures
        let mut known: Vec<String> = self.routes.get().maps.keys().cloned().collect();
        known.extend(detect.iter().map(|p| map_from_pattern(p)));
        known.extend(
            crate::movement::Stages::builtin()
                .modes
                .values()
                .flat_map(|m| m.maps.iter().map(|p| p.name.clone())),
        );
        // event / special maps from AA Spiral's loading screens
        known.extend(
            [
                "Assassin Park",
                "Cursed Womb",
                "Haunted Mansion",
                "Walled City",
                "Spirit Invasion",
                "Sky Island",
            ]
            .map(String::from),
        );
        let mut ocr_vote = false;
        // Find Match puts every Summer map's loading screen / title as
        // "Planet Greenie (Summer)" - only then ignore them (Play Here is fine)
        let summer = s.run.mode == Mode::SummerHunt && s.run.play == PlayChoice::FindMatch;
        {
            // poll until Vote Start shows; spot the map meanwhile (your
            // loading_ captures first, else OCR of the map name)
            let mut dl = self.control.deadline(self.secs(120.0));
            let started = Instant::now();
            loop {
                self.check()?;
                // Summer Event: every loading screen says Planet Greenie, so the
                // map is read from the stage title once in (below), not here
                if map.is_none() && !detect.is_empty() && !summer {
                    for p in &detect {
                        if main.find(std::slice::from_ref(p), None).is_some() {
                            let m = map_from_pattern(p);
                            self.log(format!("map: {m}"));
                            map = Some(m);
                            break;
                        }
                    }
                }
                // OCR every 2 s: map name while unknown, and "Vote Start" always
                // (backup for when the captured picture doesn't match)
                if ocr_due_for(self.main, 2) {
                    if let Some(text) = main.frame().and_then(|f| main.ocr_all(&f)) {
                        if map.is_none() && !summer {
                            if let Some(m) = match_map(&text, &known) {
                                self.log(format!("map (read by OCR): {m}"));
                                map = Some(m);
                            }
                        }
                        let flat = text.to_lowercase();
                        if flat.contains("vote") && flat.contains("start") {
                            ocr_vote = true;
                        }
                    }
                }
                if main.find_vote_yes().is_some() || main.find_wide(&vote).is_some() {
                    break;
                }
                if ocr_vote {
                    self.log("Vote Start seen (OCR)");
                    break;
                }
                // map known but Vote Start never matched: don't hang on it
                if map.is_some() && started.elapsed() > Duration::from_secs_f64(self.secs(45.0)) {
                    self.log("Vote Start not seen after 45 s - carrying on");
                    break;
                }
                if !main.has("vote_start") && map.is_some() {
                    self.sleep(6000)?;
                    break;
                }
                if !main.has("vote_start")
                    && detect.is_empty()
                    && started.elapsed() > Duration::from_secs_f64(self.secs(25.0))
                {
                    break;
                }
                if dl.expired() {
                    self.log("no Vote Start after 2 min - carrying on");
                    break;
                }
                self.sleep(300)?;
            }
        }
        // Summer Event: loading screen and title both say "Planet Greenie
        // (Summer)" on every map - recognise it from the minimap instead
        if s.run.mode == Mode::SummerHunt && map.is_none() {
            let lib = super::minimap::library(&super::minimap::dir());
            let mut seen = None;
            for _ in 0..4 {
                if let Some(f) = main.frame() {
                    if let Some((m, d)) = super::minimap::recognise(&f, &lib) {
                        self.log(format!("map (minimap): {m} [{d:.0}]"));
                        map = Some(m);
                        break;
                    }
                    // the title may still name it on some maps
                    if let Some(m) = main
                        .ocr_title(&f)
                        .and_then(|t| match_map(&t, &known))
                        .filter(|m| m != "Planet Greenie")
                    {
                        self.log(format!("map (stage title): {m}"));
                        map = Some(m);
                        break;
                    }
                    seen = Some(f);
                }
                self.sleep(700)?;
            }
            if map.is_none() {
                // new one: give it a number by itself, no naming needed
                if let Some(n) =
                    seen.and_then(|f| super::minimap::save_new(&f, &super::minimap::dir()))
                {
                    self.log(format!(
                        "new Summer map - remembered as '{n}' (rename on the Patterns page if you like)"
                    ));
                    map = Some(n);
                }
            }
        } else if let (Some(m), Some(f)) = (map.as_ref(), main.frame()) {
            // other modes know the map: learn its minimap for free
            let name = if s.run.mode == Mode::SummerHunt {
                format!("{}_summerevent", route_map(m))
            } else {
                route_map(m)
            };
            if super::minimap::save_named_if_missing(&f, &super::minimap::dir(), &name) {
                self.log(format!("minimap of {name} learned"));
            }
        }
        let map = match (map, fixed) {
            (Some(m), true) if route_map(&m) != route_map(&s.run.map) => {
                self.log(format!(
                    "loaded {m}, expected {} - using {m}",
                    route_map(&s.run.map)
                ));
                m
            }
            (Some(m), _) => m,
            (None, true) => route_map(&s.run.map),
            (None, false) if !prev_map.is_empty() => prev_map.to_string(),
            (None, false) => {
                self.log("map not recognised - staying at spawn, spiral placing (capture its loading screen as loading_<map>)");
                String::new()
            }
        };
        for h in self.main_last(self.movers()) {
            self.arb
                .run(camera_batch(h, &s.camera))
                .map_err(|e| format!("camera: {e}"))?;
        }
        // first time on this map: keep a top-down picture of it (maps/)
        // for planning exact unit spots later
        if !map.is_empty() {
            let dir = super::data_dir().join("maps");
            let base = crate::vision::key(&route_map(&map));
            let path = if s.run.mode == Mode::SummerHunt && !map.starts_with("Summer map") {
                dir.join(format!("{base}_summerevent.png"))
            } else {
                dir.join(format!("{base}.png"))
            };
            if !path.exists() {
                self.sleep(800)?;
                if let Some(f) = self.screen(self.main).frame() {
                    let _ = std::fs::create_dir_all(&dir);
                    if f.save_png(&path).is_ok() {
                        self.log(format!("map picture saved: {}", path.display()));
                    }
                }
            }
        }
        // replay on the same map: everyone is at spawn already - no need to
        // open the settings for Teleport to Spawn. New map: do it.
        if !prev_map.is_empty() && route_map(prev_map) == route_map(&map) {
            self.log("same map as last run - skipping Teleport to Spawn");
        } else {
            for h in self.main_last(self.movers()) {
                self.tp_spawn(h)?;
            }
        }
        Ok(map)
    }

    /// The map's route on every mover (alts first). A failed walk is logged,
    /// placing goes on from wherever they stand.
    pub fn stage_walk(&self, map: &str) -> R<()> {
        let s = &self.settings;
        if let Some(r) = (!map.is_empty())
            .then(|| self.routes.get().find_map(map).cloned())
            .flatten()
        {
            for h in self.main_last(self.movers()) {
                self.log(format!("map walk: {}", r.map));
                let b = r
                    .to_batch(h, self.screen(h), &vars(s))
                    .map_err(|e| e.to_string())?;
                if let Err(e) = self.arb.run(b) {
                    self.log(format!("map walk: {e} - placing from here"));
                }
            }
        } else if let Some(d) = (!map.is_empty() && s.minimap_nav)
            .then(|| super::mapnav::find(map))
            .flatten()
            .filter(|d| !d.walk.is_empty())
        {
            // no recorded route: walk by the minimap to the map file's points
            for h in self.main_last(self.movers()) {
                self.log(format!(
                    "minimap walk: {} ({} point(s))",
                    d.map,
                    d.walk.len()
                ));
                match self.minimap_walk(h, &d) {
                    Ok(()) => self.log("minimap walk: there"),
                    Err(e) if e == "stopped" || e.starts_with(DISCONNECTED) => return Err(e),
                    Err(e) => self.log(format!("minimap walk: {e} - placing from here")),
                }
            }
        }
        Ok(())
    }

    /// Where this client's arrow is on its minimap (the one nearest
    /// `near`, else the biggest), or None.
    fn arrow_at(&self, h: Hwnd, near: Option<(f32, f32)>) -> Option<(f32, f32)> {
        let f = self.screen(h).frame()?;
        let r = super::mapnav::read_client(&f)?;
        match near {
            Some(n) => r.players.iter().map(|p| p.at).min_by(|a, b| {
                let da = (a.0 - n.0).powi(2) + (a.1 - n.1).powi(2);
                let db = (b.0 - n.0).powi(2) + (b.1 - n.1).powi(2);
                da.total_cmp(&db)
            }),
            None => r.players.first().map(|p| p.at),
        }
    }

    /// Hold `keys` for `ms` on `h`.
    fn hold_keys(&self, h: Hwnd, keys: &[&str], ms: u64) -> R<()> {
        let ks: Vec<crate::keys::Key> = keys
            .iter()
            .filter_map(|k| crate::keys::Key::parse(k))
            .collect();
        let mut b = Batch::new(h, "minimap walk");
        for k in &ks {
            b = b.then(Action::KeyDown(*k));
        }
        b = b.wait(ms);
        for k in ks.iter().rev() {
            b = b.then(Action::KeyUp(*k));
        }
        self.arb
            .run(b)
            .map(|_| ())
            .map_err(|e| format!("minimap walk: {e}"))
    }

    /// Walk client `h` to the map file's `walk` points by its minimap:
    /// find which way W goes (a short W press, watching the arrows - the
    /// one that moved is this client's), then pulse the keys towards each
    /// point, correcting every look; stuck (no move for 600 ms) = back up
    /// and strafe. 90 s at most.
    pub(crate) fn minimap_walk(&self, h: Hwnd, d: &super::mapnav::MapData) -> R<()> {
        use super::mapnav::{read_client, Cmd, KeyDirs, Nav, W};
        // which arrow is mine, and which way W goes
        let before: Vec<(f32, f32)> = self
            .screen(h)
            .frame()
            .and_then(|f| read_client(&f))
            .map(|r| r.players.into_iter().map(|p| p.at).collect())
            .unwrap_or_default();
        if before.is_empty() {
            return Err("no arrow on the minimap".into());
        }
        let mut dirs = None;
        let mut me = before[0];
        for ms in [400, 800] {
            self.hold_keys(h, &[W], ms)?;
            self.sleep(150)?;
            let after: Vec<(f32, f32)> = self
                .screen(h)
                .frame()
                .and_then(|f| read_client(&f))
                .map(|r| r.players.into_iter().map(|p| p.at).collect())
                .unwrap_or_default();
            // the arrow that moved the most (from where it was)
            let best = before
                .iter()
                .filter_map(|b| {
                    let a = after.iter().min_by(|x, y| {
                        let dx = (x.0 - b.0).powi(2) + (x.1 - b.1).powi(2);
                        let dy = (y.0 - b.0).powi(2) + (y.1 - b.1).powi(2);
                        dx.total_cmp(&dy)
                    })?;
                    let mv = (a.0 - b.0, a.1 - b.1);
                    (mv.0.hypot(mv.1) < 25.0).then_some((*a, mv))
                })
                .max_by(|x, y| x.1 .0.hypot(x.1 .1).total_cmp(&y.1 .0.hypot(y.1 .1)));
            if let Some((a, mv)) = best {
                me = a;
                if let Some(k) = KeyDirs::from_w_move(mv.0, mv.1) {
                    dirs = Some(k);
                    break;
                }
            }
        }
        let Some(dirs) = dirs else {
            return Err("the arrow didn't move on W (blocked, or not this map?)".into());
        };
        self.log(format!(
            "minimap walk: W goes {:.0}° on the minimap",
            super::mapnav::bearing(dirs.w.0, dirs.w.1)
        ));
        let points: Vec<(f32, f32)> = d.walk.iter().map(|p| (p[0] as f32, p[1] as f32)).collect();
        let mut nav = Nav::new(dirs, points);
        let started = Instant::now();
        let mut dl = self.control.deadline(self.secs(90.0));
        let mut lost = 0;
        loop {
            self.check()?;
            if dl.expired() {
                return Err(format!("not there after 90 s (waypoint {})", nav.next + 1));
            }
            let Some(pos) = self.arrow_at(h, Some(me)) else {
                lost += 1;
                if lost > 10 {
                    return Err("lost the arrow on the minimap".into());
                }
                self.sleep(200)?;
                continue;
            };
            lost = 0;
            me = pos;
            match nav.step(started.elapsed().as_millis() as u64, pos) {
                Cmd::Arrived => return Ok(()),
                Cmd::GiveUp(why) => return Err(why),
                Cmd::Moves(moves) => {
                    for (keys, ms) in moves {
                        self.hold_keys(h, &keys, ms)?;
                    }
                }
            }
        }
    }

    /// Every client votes Start (alts first), then back on the main.
    pub fn stage_vote(&self) -> R<()> {
        let all: Vec<Hwnd> = self.clients.iter().map(|c| c.0).collect();
        let names = Self::names(&["vote_start"]);
        for h in self.main_last(all) {
            let scr = self.screen(h);
            // the vote box's "Yes" button by OCR first (a vote_start picture
            // may be the box's title, not the button), then the picture
            // the green Yes by colour (fixed spot, every map), then OCR "Yes"
            let mut at = scr.find_vote_yes().or_else(|| {
                scr.has_ocr()
                    .then(|| scr.find_words_now(&["~yes"]))
                    .flatten()
            });
            if at.is_none() {
                at = self.wait_wide(h, "vote_start", if h == self.main { 3000 } else { 1500 })?;
            }
            let mut tries = 0;
            while at.is_none() && tries < 4 {
                at = scr
                    .find_vote_yes()
                    .or_else(|| scr.find_words_now(&["~yes", "vote start"]))
                    .or_else(|| scr.find_label_now(&names));
                if at.is_none() {
                    self.sleep(700)?;
                }
                tries += 1;
            }
            match at {
                Some(p) => {
                    self.click(h, p, "vote start")?;
                    // no re-click: Yes stays on screen after voting (0/2 -> 1/2)
                }
                None => self.log("Vote Start not seen (picture or OCR) - carrying on"),
            }
        }
        self.to_main()?;
        Ok(())
    }

    /// Settings > Teleport to Spawn > close (skipped until tp_spawn is captured).
    fn tp_spawn(&self, h: Hwnd) -> R<()> {
        let scr = self.screen(h);
        if !scr.has("tp_spawn") {
            return Ok(());
        }
        let routes = self.routes.get();
        let Some(r) = routes.lobby.get("tp_spawn") else {
            return Ok(());
        };
        let b = r
            .to_batch(h, scr, &vars(&self.settings))
            .map_err(|e| e.to_string())?;
        if let Err(e) = self.arb.run(b) {
            self.log(format!("teleport to spawn: {e}"));
            // the kept settings spots may be off: search for them next time
            super::buttonmem::forget(Some("settings_button"));
            super::buttonmem::forget(Some("settings_close"));
            // don't leave the settings open
            if let Some(p) = scr.find(&Self::names(&["settings_close"]), None) {
                let _ = self.click(h, p, "close settings");
            }
        }
        Ok(())
    }

    /// Place on every placing client at once. Returns placed (client, slot, spot).
    pub fn place_all(&self, map: &str) -> R<Vec<(Hwnd, usize, Point)>> {
        let s = &self.settings;
        super::placing::PLACE_SETTLE_MS.store(
            s.place_settle_ms.clamp(100, 5000),
            std::sync::atomic::Ordering::Relaxed,
        );
        super::placing::PLACE_CHECK_MS.store(
            s.place_check_ms.clamp(500, 10000),
            std::sync::atomic::Ordering::Relaxed,
        );
        let keys = Keys::from(&s.keybinds)?;
        let map = route_map(map);
        let mut work = vec![];
        for n in s.placers.iter().copied() {
            let Some((h, scr)) = self.clients.get(n.saturating_sub(1)).cloned() else {
                continue;
            };
            let key = spots_key(&map, n);
            // each placing client can use its own team
            let units = s.units_for(n);
            let mut jobs = vec![];
            for (i, u) in units
                .iter()
                .enumerate()
                .filter(|(_, u)| u.enabled && u.place > 0)
            {
                let slot = i + 1;
                let mut spots: Vec<Point> = self
                    .placements
                    .for_slot(&key, slot as u8)
                    .iter()
                    .map(|p| Point::new(p.x, p.y))
                    .collect();
                if s.spiral_fallback {
                    let nth = s.placers.iter().position(|x| *x == n).unwrap_or(0);
                    spots.extend(super::placing::shape_spots_for(
                        s.place_shape,
                        // plenty of spots: the outer rings are only tried when
                        // the inner ones are taken / fail
                        160,
                        n as u64 * 31 + slot as u64,
                        s.shape_scale,
                        nth,
                        s.placers.len(),
                    ));
                }
                // hill unit: select it, read the glowing high ground, try those first
                if u.hill {
                    let key_ = keys.units[(slot.clamp(1, 6)) - 1];
                    let _ = self.arb.run(
                        Batch::new(h, "hill: show high ground")
                            .then(Action::KeyTap {
                                key: key_,
                                hold_ms: 40,
                            })
                            .wait(500),
                    );
                    let mut hs = scr
                        .frame()
                        .map(|f| super::placing::hill_spots(&f, 60))
                        .unwrap_or_default();
                    if hs.is_empty() {
                        // a banner / slow glow: one more look
                        std::thread::sleep(Duration::from_millis(500));
                        hs = scr
                            .frame()
                            .map(|f| super::placing::hill_spots(&f, 60))
                            .unwrap_or_default();
                    }
                    let _ = self.arb.run(super::placing::cancel_batch(h, &keys));
                    if hs.is_empty() {
                        self.log(format!(
                            "client {n}: slot {slot} hill - no green high ground seen, placing it on the ground"
                        ));
                    } else {
                        self.log(format!(
                            "client {n}: slot {slot} hill - {} high-ground spot(s) seen (ground after those)",
                            hs.len()
                        ));
                    }
                    let mut v = hs;
                    v.extend(spots.into_iter());
                    spots = v;
                }
                // skip spots that failed before on this map (spot memory)
                if s.spot_memory {
                    // hill units learn their own spots (high ground only)
                    let mkey = if u.hill {
                        format!("{key} hill")
                    } else {
                        key.clone()
                    };
                    let mem = self.memory.lock().unwrap();
                    // hill units: try spots that worked for hill units first
                    if u.hill {
                        let good = mem.good_spots(&mkey);
                        spots.sort_by_key(|p| {
                            !good
                                .iter()
                                .any(|g| (g.x - p.x).abs() <= 10 && (g.y - p.y).abs() <= 10)
                        });
                    }
                    let before = spots.len();
                    spots.retain(|p| !mem.is_bad(&mkey, *p));
                    if spots.len() < before {
                        self.log(format!(
                            "{mkey}: skipping {} spot(s) that failed before",
                            mem.bad_count(&mkey)
                        ));
                    }
                }
                jobs.push((slot, u.place as u32, spots));
            }
            // farm units first
            jobs.sort_by_key(|(slot, _, _)| !units[slot - 1].farm);
            work.push((n, h, scr, jobs, units));
        }
        let delay = (s.place_delay_ms as f32 * self.speed) as u64;
        let placing: Vec<Hwnd> = work.iter().map(|w| w.1).collect();
        // clients that don't place: the first placer watches their card pop-ups
        let watchers: Vec<Hwnd> = self
            .clients
            .iter()
            .map(|c| c.0)
            .filter(|h| !placing.contains(h))
            .collect();
        // set when VICTORY / DEFEAT shows up while placing
        let over = std::sync::atomic::AtomicBool::new(false);
        let over = &over;
        // spots taken by an account (being tried or placed): the others skip
        // anything close, so two accounts never fight over one spot
        let claimed: std::sync::Mutex<Vec<(Point, usize)>> = std::sync::Mutex::new(vec![]);
        let claimed = &claimed;
        let out = std::thread::scope(|sc| {
            let mut handles = vec![];
            for (wi, (n, h, scr, jobs, units)) in work.into_iter().enumerate() {
                let keys = keys.clone();
                let map = map.clone();
                let watchers = if wi == 0 { watchers.clone() } else { vec![] };
                handles.push(sc.spawn(move || {
                    // "UPG [0]" badge = the unit really went down
                    // the unit panel opened = it went down: your unit_placed
                    // picture, and/or "UPG [0]" read with OCR anywhere on screen
                    let badge = {
                        // unit_placed or the panel's red X (unit_close) picture
                        let pic = scr.has("unit_placed") || scr.has("unit_close");
                        let ocr = scr.has_ocr();
                        (pic || ocr).then(|| {
                            let me = scr.clone();
                            crate::input::Probe::new("unit placed (UPG)", move || {
                                if pic {
                                    if let Some(p) = me.find(
                                        &["unit_placed".to_string(), "unit_close".to_string()],
                                        None,
                                    ) {
                                        return Some(p);
                                    }
                                }
                                if ocr {
                                    return crate::platform::urgent_ocr(|| {
                                        me.frame().and_then(|f| me.find_upg(&f))
                                    });
                                }
                                None
                            })
                        })
                    };
                    // before every placement: stage over? card pop-up?
                    let cards = || {
                        use std::sync::atomic::Ordering;
                        if over.load(Ordering::Relaxed) {
                            return false;
                        }
                        if matches!(
                            self.end_screen(&Self::names(&["victory", "defeat"])),
                            Ok(Some(_))
                        ) {
                            over.store(true, Ordering::Relaxed);
                            self.log("stage ended while placing");
                            return false;
                        }
                        let _ = self.handle_cards(h);
                        for w in &watchers {
                            let _ = self.handle_cards(*w);
                        }
                        true
                    };
                    let mut used: Vec<Point> = vec![];
                    let mut out = vec![];
                    for (slot, count, spots) in jobs {
                        if over.load(std::sync::atomic::Ordering::Relaxed) {
                            break; // VICTORY / DEFEAT: no more placing
                        }
                        // anti-AFK waits until this unit is down
                        let _seq = seq_lock(h, Duration::from_secs(10));
                        let free: Vec<Point> =
                            spots.into_iter().filter(|p| !used.contains(p)).collect();
                        // BUFF units: turn Auto ability on while the panel is open
                        let buff = units[slot - 1].buff;
                        let auto = |_at: Point| {
                            if !buff || !scr.has("auto_ability") {
                                return;
                            }
                            match self.wait_any(h, &["auto_ability"], 1500) {
                                Ok(Some((_, p))) => {
                                    let _ = self.click(h, p, "auto ability");
                                    self.log(format!("client {n}: slot {slot} auto ability on"));
                                }
                                _ => self
                                    .log(format!("client {n}: slot {slot} auto ability not seen")),
                            }
                        };
                        let set = if units[slot - 1].hill {
                            format!("{} hill", spots_key(&map, n))
                        } else {
                            spots_key(&map, n)
                        };
                        let remember = s.spot_memory && badge.is_some();
                        let good = |p: Point| {
                            auto(p);
                            if remember {
                                self.memory.lock().unwrap().placed(&set, p);
                            }
                        };
                        let bad = |p: Point| {
                            if remember {
                                self.memory.lock().unwrap().failed(&set, p);
                            }
                            // free it again (nothing stands there)
                            claimed
                                .lock()
                                .unwrap()
                                .retain(|(q, who)| !(*who == n && *q == p));
                        };
                        let claim = |p: Point| -> bool {
                            let mut c = claimed.lock().unwrap();
                            if c.iter().any(|(q, who)| {
                                *who != n && (q.x - p.x).abs() <= 28 && (q.y - p.y).abs() <= 28
                            }) {
                                return false;
                            }
                            c.push((p, n));
                            true
                        };
                        // second opinion on each placement: the hotbar
                        // counter ("1/3") and the "Unique units may not be
                        // placed" message (AA Spiral v0.17)
                        use super::placing::Check;
                        use crate::config::PlacementDetect;
                        let detect = s.placement_detect;
                        // Panel (default) = the placing used before: the
                        // counter is only read in the other modes
                        let use_counter = detect != PlacementDetect::Panel;
                        let read_counter = || -> Option<(u32, u32)> {
                            let k = keys.units[slot.clamp(1, 6) - 1];
                            self.arb
                                .run(
                                    Batch::new(h, "read unit counter")
                                        .then(Action::KeyTap { key: k, hold_ms: 40 })
                                        .wait(350),
                                )
                                .ok()?;
                            let mut c = crate::platform::urgent_ocr(|| {
                                scr.frame().and_then(|f| scr.ocr_counter(&f))
                            });
                            if c.is_none() {
                                std::thread::sleep(Duration::from_millis(250));
                                let f = scr.frame();
                                c = crate::platform::urgent_ocr(|| {
                                    f.as_ref().and_then(|f| scr.ocr_counter(f))
                                });
                                // keep one picture of what it saw, to check where
                                // the counter sits
                                static SAVED: std::sync::atomic::AtomicBool =
                                    std::sync::atomic::AtomicBool::new(false);
                                if c.is_none() && !SAVED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                                    if let Some(f) = f {
                                        let path = super::data_dir().join("counter-debug.png");
                                        if f.save_png(&path).is_ok() {
                                            self.log(format!(
                                                "counter (1/3) not found on screen - picture saved: {}",
                                                path.display()
                                            ));
                                        }
                                    }
                                }
                            }
                            let _ = self.arb.run(super::placing::cancel_batch(h, &keys));
                            c
                        };
                        // how many of this unit stand (that we know of). A
                        // stage starts at 0, so no first read is needed; one
                        // is taken anyway in case some are down already
                        let known = std::cell::Cell::new(0u32);
                        let limit = std::cell::Cell::new(None::<u32>);
                        if use_counter {
                            let first = read_counter().or_else(|| {
                                std::thread::sleep(Duration::from_millis(300));
                                read_counter()
                            });
                            match first {
                                Some((c, m)) => {
                                    self.log(format!("client {n}: slot {slot} counter {c}/{m}"));
                                    known.set(c);
                                    limit.set(Some(m));
                                }
                                None => self.log(format!(
                                    "client {n}: slot {slot} counter not readable yet - starting from 0"
                                )),
                            }
                        }
                        let unique_or_failed = || -> Check {
                            // didn't go down: one of a unique unit already stands?
                            if ocr_due_for(h, 6) {
                                let blocked = scr
                                    .frame()
                                    .and_then(|f| scr.ocr_all(&f))
                                    .is_some_and(|t| super::hotbar::unique_blocked(&t));
                                if blocked {
                                    self.log(format!(
                                        "client {n}: slot {slot} is a unique unit and one is already down - next unit"
                                    ));
                                    return Check::Stop;
                                }
                            }
                            Check::Failed
                        };
                        let check_inner = |seen: bool| -> Check {
                            let panel_says = badge.is_some() && seen;
                            if panel_says && detect != PlacementDetect::Hotbar {
                                known.set(known.get() + 1);
                                return Check::Placed;
                            }
                            if use_counter {
                                if let Some((c, m)) = read_counter() {
                                    limit.set(Some(m));
                                    let before = known.get();
                                    if c > before {
                                        known.set(c);
                                        self.log(format!(
                                            "client {n}: slot {slot} counter {before} -> {c}/{m} - placed"
                                        ));
                                        return if c >= m {
                                            Check::PlacedLast
                                        } else {
                                            Check::Placed
                                        };
                                    }
                                    if c >= m {
                                        self.log(format!(
                                            "client {n}: slot {slot} is at its limit ({c}/{m}) - next unit"
                                        ));
                                        return Check::Stop;
                                    }
                                    // counter read and unchanged: really didn't go down
                                    return unique_or_failed();
                                }
                                // counter unreadable: fall back on the panel
                                if detect == PlacementDetect::Hotbar && badge.is_none() {
                                    self.log(format!(
                                        "client {n}: slot {slot} counter not readable - counting the click as placed"
                                    ));
                                    known.set(known.get() + 1);
                                    return Check::Placed;
                                }
                            }
                            if panel_says || badge.is_none() {
                                known.set(known.get() + 1);
                                return Check::Placed;
                            }
                            unique_or_failed()
                        };
                        // misses in a row: after a few, ask the counter
                        // whether this unit is simply at its limit already
                        let misses = std::cell::Cell::new(0u32);
                        let check = |seen: bool, _at: Point| -> Check {
                            let r = check_inner(seen);
                            if r == Check::Failed {
                                misses.set(misses.get() + 1);
                                if !use_counter && misses.get() % 3 == 0 {
                                    if let Some((c, m)) = read_counter() {
                                        if c >= m {
                                            self.log(format!(
                                                "client {n}: slot {slot} is at its limit ({c}/{m}) - next unit"
                                            ));
                                            return Check::Stop;
                                        }
                                    }
                                }
                            } else {
                                misses.set(0);
                            }
                            r
                        };
                        match super::placing::place_slot_checked(
                            &self.arb,
                            h,
                            &keys,
                            slot,
                            count,
                            &free,
                            badge.clone(),
                            buff,
                            delay,
                            &cards,
                            &claim,
                            &check,
                            &good,
                            &bad,
                        ) {
                            Ok(done) => {
                                self.log(format!(
                                    "client {n}: slot {slot} placed {}/{count}",
                                    done.len()
                                ));
                                used.extend(done.iter().copied());
                                out.extend(done.into_iter().map(|p| (h, slot, p)));
                            }
                            Err(e) => {
                                self.log(format!("client {n}: placing stopped - {e}"));
                                break;
                            }
                        }
                    }
                    out
                }));
            }
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap_or_default())
                .collect::<Vec<_>>()
        });
        if !watchers.is_empty() {
            self.to_main()?;
        }
        Ok(out)
    }

    /// Upgrade by priority until VICTORY / DEFEAT shows. Returns true on a win.
    /// Hold upgrade clicks on `h` while its card / weather vote screen is up
    /// (seen in the last 12 s and still on screen).
    fn wait_cards_gone(&self, h: Hwnd) -> R<()> {
        let mut said = false;
        loop {
            let until = CARDS_UP
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .find(|(c, _)| *c == h)
                .map(|(_, t)| *t);
            match until {
                Some(t) if Instant::now() < t => {
                    if !said {
                        said = true;
                        self.log("card vote on screen - upgrades wait for it");
                    }
                    self.sleep(500)?;
                }
                _ => return Ok(()),
            }
        }
    }

    /// Place > Upgrade hand-over: placement mode closed on every client that
    /// placed (Cancel twice: an open ghost / unit panel), a moment for the
    /// game's UI to settle, then a log line per client with what's on screen
    /// (unit panel open? card pick? end screen?). A panel still open gets
    /// another Cancel, so the first unit click selects the unit.
    pub(crate) fn place_to_upgrade(&self, placed: &[(Hwnd, usize, Point)]) -> R<()> {
        let keys = Keys::from(&self.settings.keybinds)?;
        let mut hs: Vec<Hwnd> = vec![];
        for (h, _, _) in placed {
            if !hs.contains(h) {
                hs.push(*h);
            }
        }
        let num = |h: Hwnd| self.clients.iter().position(|c| c.0 == h).unwrap_or(0) + 1;
        self.log(format!(
            "place > upgrade: {} unit(s) on {} client(s) - closing placement mode, then {} ms to settle",
            placed.len(),
            hs.len(),
            PLACE_TO_UPGRADE_MS
        ));
        for h in self.main_last(hs.clone()) {
            let _seq = seq_lock(h, Duration::from_secs(10));
            self.arb
                .run(
                    super::placing::cancel_batch(h, &keys)
                        .wait(150)
                        .then(Action::KeyTap {
                            key: keys.cancel,
                            hold_ms: 40,
                        })
                        .wait(150),
                )
                .map_err(|e| format!("close placement: {e}"))?;
        }
        self.sleep(PLACE_TO_UPGRADE_MS)?;
        let end = Self::names(&["victory", "defeat"]);
        for h in &hs {
            let scr = self.screen(*h);
            let open = self.panel_open(scr, 300)?;
            let cards = scr.has("choose_card")
                && scr
                    .find_picture_wide(&Self::names(&["choose_card"]))
                    .is_some();
            let over = scr.find_picture_wide(&end).is_some();
            let units: Vec<String> = placed
                .iter()
                .filter(|(ph, _, _)| ph == h)
                .map(|(_, slot, at)| format!("slot {slot} at {},{}", at.x, at.y))
                .collect();
            self.log(format!(
                "place > upgrade: client {}: unit panel {}, card pick {}, end screen {} - units: {}",
                num(*h),
                if open { "OPEN" } else { "closed" },
                if cards { "on screen" } else { "no" },
                if over { "on screen" } else { "no" },
                if units.is_empty() { "none".into() } else { units.join(", ") }
            ));
            if open {
                self.log(format!(
                    "place > upgrade: client {}: panel still open - closing it",
                    num(*h)
                ));
                let _seq = seq_lock(*h, Duration::from_secs(10));
                self.arb
                    .run(super::placing::cancel_batch(*h, &keys).wait(200))
                    .map_err(|e| format!("close placement: {e}"))?;
            }
        }
        Ok(())
    }

    pub fn upgrade_until_end(&self, placed: &[(Hwnd, usize, Point)]) -> R<bool> {
        let s = &self.settings;
        let keys = Keys::from(&s.keybinds)?;
        // unit settings per client (each placing client may have its own team)
        let per: std::collections::HashMap<Hwnd, Vec<crate::config::UnitSlot>> = placed
            .iter()
            .map(|(h, _, _)| {
                let n = self.clients.iter().position(|c| c.0 == *h).unwrap_or(0) + 1;
                (*h, s.units_for(n))
            })
            .collect();
        let unit = |h: &Hwnd, slot: usize| per[h][slot - 1].clone();
        let mut order: Vec<&(Hwnd, usize, Point)> = placed
            .iter()
            .filter(|(h, slot, _)| unit(h, *slot).upgrade != Upgrade::None)
            .collect();
        // your Priority decides (1 first); farm units only go first among equals
        order.sort_by_key(|(h, slot, _)| (unit(h, *slot).priority, !unit(h, *slot).farm));
        if !order.is_empty() {
            let list: Vec<String> = order
                .iter()
                .map(|(h, slot, _)| {
                    let n = self.clients.iter().position(|c| c.0 == *h).unwrap_or(0) + 1;
                    format!("c{n} slot {slot} (prio {})", unit(h, *slot).priority)
                })
                .collect();
            self.log(format!("upgrade order: {}", list.join(" > ")));
        }
        // placed, but its Upgrade setting is None: never upgraded
        for (h, slot, _) in placed {
            if unit(h, *slot).upgrade == Upgrade::None {
                let n = self.clients.iter().position(|c| c.0 == *h).unwrap_or(0) + 1;
                self.log(format!(
                    "c{n} slot {slot}: Upgrade is set to None on the units page - not upgraded"
                ));
            }
        }
        if order.is_empty() && !placed.is_empty() {
            self.log("WARN nothing to upgrade: every placed unit has Upgrade = None");
        }
        // a unit that should be upgraded but never registered as placed
        for (h, _) in &per {
            let n = self.clients.iter().position(|c| c.0 == *h).unwrap_or(0) + 1;
            for (i, u) in per[h].iter().enumerate() {
                if u.enabled
                    && u.place > 0
                    && u.upgrade != Upgrade::None
                    && !placed.iter().any(|(ph, sl, _)| ph == h && *sl == i + 1)
                {
                    self.log(format!(
                        "c{n} slot {} wasn't registered as placed - it can't be upgraded (placement check missed it)",
                        i + 1
                    ));
                }
            }
        }
        // one upgrader per account, all at the same time (each one's input
        // turns still take the window in turn); this thread watches for the
        // end screen and cards meanwhile
        let mut clients: Vec<Hwnd> = vec![];
        for (h, _, _) in &order {
            if !clients.contains(h) {
                clients.push(*h);
            }
        }
        let over = std::sync::atomic::AtomicBool::new(false);
        let failed: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
        let end = Self::names(&["victory", "defeat"]);
        let result = std::thread::scope(|sc| -> R<bool> {
            for h in &clients {
                let mine: Vec<&(Hwnd, usize, Point)> =
                    order.iter().copied().filter(|(c, _, _)| c == h).collect();
                let (over, failed, keys, unit) = (&over, &failed, &keys, &unit);
                sc.spawn(move || {
                    if let Err(e) = self.upgrade_account(&mine, keys, unit, over) {
                        if e != "stage over" {
                            *failed.lock().unwrap() = Some(e);
                        }
                        over.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                });
            }
            let mut last_idle = Instant::now();
            let mut dl = self.control.deadline(self.secs(60.0 * 40.0));
            let r = loop {
                if let Some(e) = failed.lock().unwrap().take() {
                    break Err(e);
                }
                if let Err(e) = self.check() {
                    break Err(e);
                }
                match self.end_screen(&end) {
                    Ok(Some(won)) => break Ok(won),
                    Err(e) => break Err(e),
                    Ok(None) => {}
                }
                if let Err(e) = self.cards_all() {
                    break Err(e);
                }
                if dl.expired() {
                    break Err("no end screen after 40 min".into());
                }
                // nothing left to upgrade anywhere: keep the PC / Roblox busy
                if UPGRADING.load(std::sync::atomic::Ordering::Relaxed) == 0 {
                    let _ = self.idle_click(&mut last_idle, keys_ref(&keys));
                }
                if self.sleep(700).is_err() {
                    break Err("stopped".into());
                }
            };
            over.store(true, std::sync::atomic::Ordering::Relaxed);
            r
        });
        result
    }

    /// One account's upgrades, by priority, until `over`. MAX units get the
    /// upgrade key 3x per visit.
    fn upgrade_account(
        &self,
        order: &[&(Hwnd, usize, Point)],
        keys: &Keys,
        unit: &dyn Fn(&Hwnd, usize) -> crate::config::UnitSlot,
        over: &std::sync::atomic::AtomicBool,
    ) -> R<()> {
        use std::sync::atomic::Ordering;
        let s = &self.settings;
        let mut done: Vec<u32> = vec![0; order.len()];
        let mut maxed: Vec<bool> = vec![false; order.len()];
        // visits in a row where the presses changed nothing; after 3 the
        // unit is left alone for a minute (no money yet, or MAX not read)
        let mut stuck: Vec<u32> = vec![0; order.len()];
        // safety: 60 upgrade presses on one unit and still not seen as done
        // (MAX / its level not read) = treat it as done, go on to the others
        let mut pressed: Vec<u32> = vec![0; order.len()];
        let mut rest: Vec<Option<Instant>> = vec![None; order.len()];
        // per unit: the spot that opened its panel last time (the ground
        // point it was placed at, or a bit above it - the unit's body)
        let mut aim: Vec<Option<Point>> = vec![None; order.len()];
        // per unit: since when its level hasn't moved (no money yet) while
        // Priority upgrade keeps on it, and when that was last logged
        let mut cash_since: Vec<Option<Instant>> = vec![None; order.len()];
        let mut cash_said: Vec<Option<Instant>> = vec![None; order.len()];
        let target = |want: Upgrade| match want {
            Upgrade::Level(n) => n.to_string(),
            _ => "MAX".to_string(),
        };
        let resting = |k: usize, rest: &[Option<Instant>]| {
            rest[k].is_some_and(|t| t.elapsed() < Duration::from_secs(60))
        };
        let finished = |k: usize, done: &[u32], maxed: &[bool]| {
            let (h, slot, _) = order[k];
            maxed[k]
                || match unit(h, *slot).upgrade {
                    Upgrade::Level(n) => done[k] >= n as u32,
                    _ => false,
                }
        };
        let mut busy = true;
        UPGRADING.fetch_add(1, Ordering::Relaxed);
        let mut i = 0usize;
        let r = (|| -> R<()> {
            loop {
                if over.load(Ordering::Relaxed) {
                    return Err("stage over".into());
                }
                self.check()?;
                if order.is_empty()
                    || (0..order.len()).all(|k| finished(k, &done, &maxed) || resting(k, &rest))
                {
                    if busy {
                        busy = false;
                        UPGRADING.fetch_sub(1, Ordering::Relaxed);
                    }
                    self.sleep(1000)?;
                    continue;
                }
                while finished(i % order.len(), &done, &maxed) || resting(i % order.len(), &rest) {
                    i += 1;
                }
                let k = i % order.len();
                let (h, slot, at) = order[k];
                let scr = self.screen(*h);
                let want = unit(h, *slot).upgrade;
                // a card / weather vote is up: unit clicks would land on the
                // cards (and vote!) - wait until it's gone
                self.wait_cards_gone(*h)?;
                // this unit's sequence: anti-AFK waits until it's done
                let seq = seq_lock(*h, Duration::from_secs(10));
                // open the unit, and make sure the panel IS open before
                // reading anything (a closed panel = nothing at the level box)
                let mut open = false;
                for attempt in 0..=PANEL_RETRIES {
                    self.check()?;
                    crate::status::attempt(*h, attempt + 1, PANEL_RETRIES + 1);
                    // where to click: what worked last time, else the placed
                    // spot, then a bit above / beside it (the unit stands on
                    // the spot; its body / nameplate is higher up)
                    let p = match (attempt, aim[k]) {
                        (0, Some(p)) => p,
                        (n, _) => unit_click_point(*at, n),
                    };
                    let right = attempt == 0 && s.upgrade_right_click;
                    if attempt > 0 {
                        self.log(format!(
                            "slot {slot} at {},{}: panel didn't open, clicking unit again at {},{} ({attempt}/{PANEL_RETRIES})",
                            at.x, at.y, p.x, p.y
                        ));
                    }
                    let b = if right {
                        Batch::new(*h, "open unit (right)").then(Action::Click {
                            at: p,
                            button: crate::platform::MouseButton::Right,
                        })
                    } else {
                        Batch::new(*h, "open unit").click(p.x, p.y)
                    };
                    self.arb
                        .run(b.wait(300))
                        .map_err(|e| format!("upgrade: {e}"))?;
                    // let the panel start drawing, then look for it
                    self.sleep(s.upgrade_read_ms / 2)?;
                    if self.panel_open(scr, s.upgrade_read_ms + 700)? {
                        open = true;
                        if aim[k] != Some(p) && !right {
                            if attempt > 0 {
                                self.log(format!(
                                    "slot {slot}: panel opened at {},{} - using that spot from now on",
                                    p.x, p.y
                                ));
                            }
                            aim[k] = Some(p);
                        }
                        crate::status::clear_attempt(*h);
                        break;
                    }
                    if attempt == 0 {
                        aim[k] = None; // what worked before doesn't now
                    }
                }
                if !open {
                    self.log(format!(
                        "slot {slot} at {},{}: panel didn't open after {PANEL_RETRIES} retries - unit not selectable? next unit, back in a minute",
                        at.x, at.y
                    ));
                    rest[k] = Some(Instant::now());
                    self.arb
                        .run(super::placing::cancel_batch(*h, keys))
                        .map_err(|e| format!("upgrade: {e}"))?;
                    drop(seq);
                    i += 1;
                    self.sleep(PANEL_CLOSE_MS)?;
                    continue;
                }
                let mut waiting_cash = false;
                let is_max = scr.has("upgrade_max")
                    && scr.find(&Self::names(&["upgrade_max"]), None).is_some();
                let before = self.read_level_stable(scr, s.upgrade_read_ms + 1200, None)?;
                let reached = match (want, before) {
                    (Upgrade::Level(n), Some(l)) => l >= n as u32,
                    _ => false,
                };
                if is_max || before == Some(u32::MAX) {
                    maxed[k] = true;
                    self.log(format!("slot {slot} at {},{} is MAX", at.x, at.y));
                } else if reached {
                    done[k] = before.unwrap_or(0);
                    cash_since[k] = None;
                    self.log(format!(
                        "Upgrade: slot {slot} reached Target Level {} - moving to next priority.",
                        target(want)
                    ));
                } else {
                    // MAX: 3 presses a visit; a set level: only what's left (max 3)
                    let presses = match (want, before) {
                        (Upgrade::Level(n), Some(l)) => (n as u32).saturating_sub(l).clamp(1, 3),
                        (Upgrade::Level(_), None) => 1,
                        _ => 3,
                    };
                    let mut b = Batch::new(*h, "upgrade");
                    for _ in 0..presses {
                        b = b
                            .then(crate::input::Action::KeyTap {
                                key: keys.upgrade,
                                hold_ms: 40,
                            })
                            .wait(180);
                    }
                    self.arb.run(b).map_err(|e| format!("upgrade: {e}"))?;
                    pressed[k] += presses;
                    let cap = s.upgrade_press_cap;
                    if cap > 0 && pressed[k] >= cap {
                        maxed[k] = true;
                        self.log(format!(
                            "slot {slot} at {},{}: {cap} upgrade presses and still not MAX / its level - counting it as done, on to the next unit",
                            at.x, at.y
                        ));
                    }
                    self.sleep(s.upgrade_read_ms.max(500))?;
                    // panel closed under us (a click, a pop-up): this visit
                    // tells nothing - no MAX / no-money verdict, try it again
                    let still_open = self.panel_open(scr, s.upgrade_read_ms + 700)?;
                    let after = if still_open {
                        // poll until the level settles on a new value (or time's up)
                        self.read_level_stable(scr, s.upgrade_read_ms + 1500, before)?
                    } else {
                        None
                    };
                    if !still_open {
                        pressed[k] = pressed[k].saturating_sub(presses);
                        self.log(format!(
                            "slot {slot} at {},{}: panel closed during the upgrade - not counted, trying again",
                            at.x, at.y
                        ));
                    } else {
                        match (before, after) {
                            (_, Some(u32::MAX)) => {
                                maxed[k] = true;
                                self.log(format!("slot {slot} at {},{} is MAX", at.x, at.y));
                            }
                            (Some(b), Some(a)) if a > b => {
                                stuck[k] = 0;
                                cash_since[k] = None;
                                done[k] = a;
                                self.log(format!(
                                    "slot {slot} at {},{} upgraded to {a}",
                                    at.x, at.y
                                ));
                                if let Upgrade::Level(n) = want {
                                    if a >= n as u32 {
                                        self.log(format!(
                                            "Upgrade: slot {slot} reached Target Level {n} - moving to next priority."
                                        ));
                                    }
                                }
                            }
                            // same level, panel open: not enough money yet.
                            // Priority upgrade: stay on this unit, wait for
                            // cash, try it again (no swapping to the next)
                            (Some(b), Some(_)) if s.run.priority_upgrade => {
                                let since = *cash_since[k].get_or_insert_with(Instant::now);
                                if since.elapsed() > CASH_WAIT_LIMIT {
                                    // 10 min without a level: most likely MAX
                                    // that isn't shown / read - don't stall on it
                                    cash_since[k] = None;
                                    maxed[k] = true;
                                    self.log(format!(
                                        "Upgrade: slot {slot} at {},{} (Level {b}) - no change in {} min, counting it as done (MAX not seen?) - next priority",
                                        at.x, at.y, CASH_WAIT_LIMIT.as_secs() / 60
                                    ));
                                } else {
                                    if cash_said[k]
                                        .is_none_or(|t| t.elapsed() > Duration::from_secs(30))
                                    {
                                        cash_said[k] = Some(Instant::now());
                                        self.log(format!(
                                            "Upgrade: slot {slot} at {},{} (Level {b}) - waiting for cash to reach Target Level {}...",
                                            at.x, at.y, target(want)
                                        ));
                                    }
                                    waiting_cash = true;
                                }
                            }
                            // same level = not enough money yet (or MAX not read)
                            (Some(_), Some(_)) => {
                                stuck[k] += 1;
                                if stuck[k] >= 3 {
                                    stuck[k] = 0;
                                    rest[k] = Some(Instant::now());
                                    self.log(format!(
                                    "slot {slot} at {},{}: no change after 3 tries (MAX or no money) - next unit, back in a minute",
                                    at.x, at.y
                                ));
                                }
                            }
                            (None, Some(a)) => done[k] = a,
                            // level text not readable: count the presses
                            _ => {
                                done[k] += presses;
                                stuck[k] += 1;
                                if stuck[k] >= 3 {
                                    stuck[k] = 0;
                                    rest[k] = Some(Instant::now());
                                    self.log(format!(
                                    "slot {slot} at {},{}: level not readable 3x - next unit, back in a minute",
                                    at.x, at.y
                                ));
                                }
                            }
                        }
                    }
                }
                self.arb
                    .run(super::placing::cancel_batch(*h, keys))
                    .map_err(|e| format!("upgrade: {e}"))?;
                drop(seq);
                if !(s.run.priority_upgrade && !finished(k, &done, &maxed) && !resting(k, &rest)) {
                    i += 1; // priority upgrade keeps on this one until it's done
                }
                // the panel's closing: a unit clicked during it doesn't open
                self.sleep(PANEL_CLOSE_MS)?;
                if waiting_cash {
                    // let the match pay out before trying the same unit again
                    self.sleep(CASH_WAIT_MS)?;
                }
            }
        })();
        if busy {
            UPGRADING.fetch_sub(1, Ordering::Relaxed);
        }
        r
    }

    /// Every 5 s once there's nothing left to upgrade: a click on an empty
    /// spot (then Cancel, in case it opened a unit) on every client, so the PC
    /// doesn't idle into sleep and Roblox doesn't kick for AFK.
    fn idle_click(&self, last: &mut Instant, keys: &Keys) -> R<()> {
        if !self.settings.idle_click || last.elapsed() < Duration::from_secs_f64(self.secs(5.0)) {
            return Ok(());
        }
        *last = Instant::now();
        let (x, y) = self.settings.idle_click_at;
        let all: Vec<Hwnd> = self.clients.iter().map(|c| c.0).collect();
        for h in self.main_last(all) {
            self.arb
                .run(
                    Batch::new(h, "idle click")
                        .click(x, y)
                        .wait(150)
                        .then(Action::KeyTap {
                            key: keys.cancel,
                            hold_ms: 40,
                        }),
                )
                .map_err(|e| format!("idle click: {e}"))?;
        }
        Ok(())
    }

    /// Is the unit panel open on this client? Polls up to `ms`: your
    /// unit_close / unit_placed / upgrade_max pictures, else OCR of the
    /// panel's own words (UPG / Sell / targeting, as `upg_line`).
    fn panel_open(&self, scr: &super::LiveScreen, ms: u64) -> R<bool> {
        let pics: Vec<String> = ["unit_close", "unit_placed", "upgrade_max"]
            .iter()
            .filter(|n| scr.has(n))
            .map(|n| n.to_string())
            .collect();
        let ocr = scr.has_ocr();
        if pics.is_empty() && !ocr {
            return Ok(true); // nothing to tell by: assume it opened
        }
        let mut dl = self
            .control
            .deadline(self.secs(ms as f64 / 1000.0).max(0.05));
        loop {
            self.check()?;
            if !pics.is_empty() && scr.find(&pics, None).is_some() {
                return Ok(true);
            }
            if ocr
                && crate::platform::urgent_ocr(|| scr.frame().and_then(|f| scr.find_upg(&f)))
                    .is_some()
            {
                return Ok(true);
            }
            if dl.expired() {
                return Ok(false);
            }
            self.sleep(200)?;
        }
    }

    /// The unit panel's level ("UPG [n]"): Windows OCR first (nothing to
    /// capture), else your upg_1 .. upg_10 patterns (unit_placed = 0).
    /// Some(u32::MAX) = MAX.
    /// Poll the panel's level until it's trustworthy, up to `ms`:
    /// - MAX: once is enough (it's never a misread of a number)
    /// - a number: the same value twice in a row (a half-drawn panel or an
    ///   OCR slip rarely repeats itself)
    /// - with `changed_from`: stop early once two reads agree on a value
    ///   other than that one (the upgrade went through)
    /// Gives the last value read when time runs out (None = never read).
    fn read_level_stable(
        &self,
        scr: &super::LiveScreen,
        ms: u64,
        changed_from: Option<u32>,
    ) -> R<Option<u32>> {
        let mut dl = self.control.deadline(self.secs(ms as f64 / 1000.0));
        let mut last: Option<u32> = None;
        let mut agreed: Option<u32> = None;
        let mut gen = crate::input::generation(scr.hwnd());
        loop {
            // input since the last read: earlier reads don't count any more
            let now = crate::input::generation(scr.hwnd());
            if now != gen {
                gen = now;
                last = None;
            }
            let v = Self::read_level(scr);
            if v == Some(u32::MAX) {
                return Ok(v);
            }
            if v.is_some() && v == last {
                agreed = v;
                // reading the level before an upgrade: settled
                if changed_from.is_none() || v != changed_from {
                    return Ok(v);
                }
            }
            if v.is_some() {
                last = v;
            }
            if dl.expired() {
                return Ok(agreed.or(last));
            }
            self.sleep(250)?;
        }
    }

    fn read_level(scr: &super::LiveScreen) -> Option<u32> {
        // upgrade reads of different accounts may run at the same time.
        // Input sent to this client while reading = the read may show the old
        // panel: throw it away and read again (up to 3x)
        for _ in 0..3 {
            let started = crate::input::generation(scr.hwnd());
            let v = crate::platform::urgent_ocr(|| Self::read_level_now(scr));
            if crate::input::generation(scr.hwnd()) == started {
                return v;
            }
        }
        None
    }

    fn read_level_now(scr: &super::LiveScreen) -> Option<u32> {
        let f = scr.frame()?;
        // MAX first: the panel says "UPG [6] (MAX)" and "Max Upgrade", but a
        // small read of the UPG box may only catch "UPG [6]"
        if scr.has_ocr() {
            let max = crate::platform::urgent_ocr(|| scr.ocr_lines(&f)).is_some_and(|lines| {
                // "Max" and "Upgrade" sit on two lines in the panel: look at
                // the whole text as well as line by line
                let all: String = lines
                    .iter()
                    .flat_map(|l| l.iter().map(|(w, _)| w.to_lowercase()))
                    .collect::<Vec<_>>()
                    .join(" ");
                let all_flat: String = all.chars().filter(|c| c.is_alphanumeric()).collect();
                all_flat.contains("maxupgrade")
                    || all.contains("(max)")
                    || all.contains("[max]")
                    || lines.iter().any(|l| {
                        let flat: String = l
                            .iter()
                            .flat_map(|(w, _)| w.chars())
                            .filter(|c| c.is_alphanumeric())
                            .flat_map(|c| c.to_lowercase())
                            .collect();
                        flat.contains("upg") && flat.contains("max")
                    })
            });
            if max {
                return Some(u32::MAX);
            }
        }
        match scr.ocr_level(&f) {
            Some(Ok(n)) => return Some(n),
            Some(Err(())) => return Some(u32::MAX),
            None => {}
        }
        let mut names: Vec<(u32, String)> = (1..=10).map(|n| (n, format!("upg_{n}"))).collect();
        names.push((0, "unit_placed".into()));
        // highest first so "UPG [10]" isn't read as "UPG [1]"
        names.sort_by(|a, b| b.0.cmp(&a.0));
        names
            .into_iter()
            .filter(|(_, n)| scr.has(n))
            .find(|(_, n)| scr.find_in_frame(&f, n))
            .map(|(l, _)| l)
    }

    /// VICTORY / DEFEAT on any client (main first) - an alt can see the
    /// end first, then everyone goes to the end screen handling.
    fn end_screen(&self, end: &[String]) -> R<Option<bool>> {
        let mut order: Vec<Hwnd> = vec![self.main];
        order.extend(self.clients.iter().map(|c| c.0).filter(|h| *h != self.main));
        for h in order {
            let scr = self.screen(h);
            for n in end {
                if scr.find(std::slice::from_ref(n), None).is_some() {
                    return Ok(Some(n == "victory"));
                }
            }
            // OCR reads VICTORY / DEFEAT too (no capture needed)
            if ocr_due_for(h, 1) {
                if let Some(r) = scr.frame().and_then(|f| scr.ocr_end(&f)) {
                    return Ok(Some(r));
                }
            }
        }
        Ok(None)
    }

    /// Next -> Replay or Return to lobby. Returns true when back in the lobby.
    pub fn after_end(&self) -> R<bool> {
        self.after_end_to(self.settings.run.return_to_lobby)
    }

    /// `after_end`, choosing Return to Lobby (`lobby`) or Replay.
    pub fn after_end_to(&self, lobby: bool) -> R<bool> {
        self.after_end_with(lobby, &mut |_| Ok(()))
    }

    /// `after_end_to`, calling `on_rewards` once on the screen after Next
    /// (where Replay / Return shows), before either is clicked.
    pub fn after_end_with(
        &self,
        lobby: bool,
        on_rewards: &mut dyn FnMut(&Self) -> R<()>,
    ) -> R<bool> {
        let mut rewards_done = false;
        // straight to the main: spam Next until Replay shows, then Replay
        self.to_main()?;
        let main = self.screen(self.main);
        let want = if lobby { "return_to_lobby" } else { "replay" };
        let title = Self::names(&["victory", "defeat"]);
        // your Next pictures (no OCR: the white-on-green font misreads)
        let next = Self::names(NEXT_PICS);
        let end = Self::names(&[
            "victory",
            "defeat",
            "next_button",
            "defeat_next_btn",
            "victory_next_btn",
            "replay",
            "return_to_lobby",
        ]);
        let mut dl = self.control.deadline(self.secs(90.0));
        let waited = Instant::now();
        let mut clicked = false;
        let mut blind = 0u32;
        loop {
            self.check()?;
            // the button we want: click it, then make sure the end screen goes
            if let Some(p) = main.find_wide(&Self::names(&[want])) {
                if !rewards_done {
                    rewards_done = true;
                    on_rewards(self)?;
                    // the item's second click may have moved things: look again
                    continue;
                }
                self.click(self.main, p, want)?;
                clicked = true;
                let mut gone = self.control.deadline(self.secs(8.0));
                while main.find_wide(&end).is_some() && !gone.expired() {
                    self.sleep(400)?;
                }
                if main.find_wide(&end).is_none() {
                    break;
                }
                self.log(format!("{want} didn't take - clicking again"));
                continue;
            }
            // Next, wherever it is on this end screen: your picture, else
            // the big green button by its colour
            let green = || main.frame().and_then(|f| super::next_green(&f));
            if let Some((p, how)) = main
                .find_picture_wide(&next)
                .map(|p| (p, "next"))
                .or_else(|| green().map(|p| (p, "next (green)")))
            {
                self.arb
                    .run(Batch::new(self.main, how).click(p.x, p.y))
                    .map_err(|e| format!("next: {e}"))?;
                self.sleep(600)?;
                continue;
            }
            // VICTORY / DEFEAT but no Next seen: click where Next sits under
            // the title (spam it - one of them lands)
            if let Some(t) = main.find_wide(&title) {
                let p = Point::new(t.x, (t.y + self.settings.next_below_title).min(590));
                if blind % 5 == 0 {
                    self.log(format!(
                        "Next not recognised - clicking under the title at {},{}",
                        p.x, p.y
                    ));
                }
                blind += 1;
                self.arb
                    .run(Batch::new(self.main, "next (blind)").click(p.x, p.y))
                    .map_err(|e| format!("next: {e}"))?;
                self.sleep(800)?;
                continue;
            }
            // only OCR sees VICTORY / DEFEAT: Next's usual spot
            if let Some(f) = main.frame() {
                if main.ocr_end(&f).is_some() {
                    let p = super::NEXT_SPOT;
                    if blind % 5 == 0 {
                        self.log("end screen read by OCR - clicking Next's usual spot");
                    }
                    blind += 1;
                    self.arb
                        .run(Batch::new(self.main, "next (ocr)").click(p.x, p.y))
                        .map_err(|e| format!("next: {e}"))?;
                    self.sleep(800)?;
                    continue;
                }
            }
            // already left the end screen (e.g. the click took on the first try)
            if clicked && main.find_wide(&end).is_none() {
                break;
            }
            // no Replay on this end screen (some modes only offer the lobby)
            if !lobby && waited.elapsed() > Duration::from_secs_f64(self.secs(15.0)) {
                if let Some(p) = main.find_wide(&Self::names(&["return_to_lobby"])) {
                    if !rewards_done {
                        on_rewards(self)?;
                    }
                    self.log("no Replay here - going back to the lobby instead");
                    self.click(self.main, p, "return to lobby")?;
                    self.sleep(8000)?;
                    return Ok(true);
                }
            }
            if dl.expired() {
                return Err(format!("end screen: '{want}' never showed up"));
            }
            self.sleep(400)?;
        }
        self.log(if lobby {
            "back to the lobby"
        } else {
            "replaying"
        });
        if lobby {
            self.sleep(8000)?;
        }
        Ok(lobby)
    }

    /// The pearls this run ("+7291" next to the pearl icon on the VICTORY /
    /// DEFEAT panel). Your `pearl_icon` picture, else the icon by its pink,
    /// else the fixed spot; OCR of just that box. A few looks while the
    /// panel animates in. None = not read.
    pub(crate) fn read_pearls(&self) -> R<Option<u64>> {
        use super::loot;
        let main = self.screen(self.main);
        if !main.has_ocr() {
            return Ok(None);
        }
        for _ in 0..4 {
            if let Some(f) = main.frame() {
                let icon = main
                    .has("pearl_icon")
                    .then(|| {
                        main.find(
                            &Self::names(&["pearl_icon"]),
                            Some(crate::geom::Rect::new(250, 320, 310, 70)),
                        )
                    })
                    .flatten()
                    .map(|p| crate::geom::Rect::new(p.x - 12, p.y - 12, 24, 24))
                    .or_else(|| loot::pearl_icon(&f));
                let area = loot::pearl_amount_rect(icon);
                let text = crate::platform::urgent_ocr(|| main.ocr_rect(&f, area));
                if let Some(n) = text.as_deref().and_then(loot::parse_amount) {
                    self.log(format!(
                        "pearls this run: {n} (read '{}' {})",
                        text.unwrap_or_default().trim(),
                        if icon.is_some() {
                            "next to the icon"
                        } else {
                            "at the fixed spot"
                        }
                    ));
                    return Ok(Some(n));
                }
            }
            self.sleep(400)?;
        }
        self.log("WARN pearls not read on the end panel");
        Ok(None)
    }

    /// End step, reward screen (after Next): read Pearls / event currency,
    /// look for a charm / portal (picture of it), record the run (history +
    /// session totals), send the Discord loot report and wait for it, then
    /// click the item again (the game wants a second click to go on).
    pub(crate) fn end_rewards(
        &self,
        won: bool,
        map: &str,
        end_drops: &[(String, u64)],
        pearls: Option<u64>,
    ) -> R<()> {
        use super::loot;
        let main = self.screen(self.main);
        self.sleep(500)?; // let the reward screen finish drawing
        let frame = main.frame();
        let lines = frame
            .as_ref()
            .and_then(|f| main.ocr_lines(f))
            .unwrap_or_default();
        let texts: Vec<String> = lines
            .iter()
            .map(|l| {
                l.iter()
                    .map(|(w, _)| w.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        let drops = loot::merge(end_drops, &super::drops::parse(&texts));
        // the charm / portal: your drop_* pictures first, else OCR of its name
        let pics: Vec<String> = ["drop_charm", "drop_portal", "charm", "portal"]
            .iter()
            .filter(|n| main.has(n))
            .map(|n| n.to_string())
            .collect();
        let charm: Option<(String, crate::geom::Rect)> = (!pics.is_empty())
            .then(|| main.find_wide(&pics))
            .flatten()
            .map(|p| {
                (
                    "item".to_string(),
                    crate::geom::Rect::new(p.x - 20, p.y - 20, 40, 40),
                )
            })
            .or_else(|| loot::find_charm(&lines));
        let mut stats = loot::RunStats {
            run: 0, // numbered by the session below
            at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            won,
            mode: self.settings.run.mode.label().to_string(),
            map: map.to_string(),
            // pearls = the Summer Event currency: the end panel's "+7291"
            // first, else whatever the reward lines said
            event_currency: pearls.unwrap_or_else(|| {
                loot::amount(&drops, loot::PEARLS).max(loot::amount(&drops, loot::EVENT_CURRENCY))
            }),
            pearls: pearls.unwrap_or_else(|| loot::amount(&drops, loot::PEARLS)),
            charm: charm.as_ref().map(|c| c.0.clone()),
            drops,
            image: None,
        };
        // the item's picture
        let mut picture = None;
        if let (Some((_, at)), Some(f)) = (&charm, &frame) {
            if let Some(r) = loot::crop_around(*at, f.width, f.height) {
                let crop = f.crop(r.x as u32, r.y as u32, r.w as u32, r.h as u32);
                let dir = loot::image_dir();
                let name = format!("{}.png", chrono::Local::now().format("%Y%m%d_%H%M%S"));
                if std::fs::create_dir_all(&dir).is_ok() && crop.save_png(&dir.join(&name)).is_ok()
                {
                    stats.image = Some(dir.join(name));
                }
                picture = Some(crop);
            }
        }
        let session = {
            let mut s = self.session.lock().unwrap_or_else(|e| e.into_inner());
            s.add(&mut stats);
            s.clone()
        };
        if let Err(e) = loot::append_history(&stats) {
            self.log(format!("run history not saved: {e}"));
        }
        self.log(format!(
            "run #{} {}: event currency {}, pearls {}, charm {}",
            stats.run,
            stats.result(),
            stats.event_currency,
            stats.pearls,
            stats.charm.as_deref().unwrap_or("no")
        ));
        // Discord: wait for the upload (the picture) before going on, but
        // never longer than WEBHOOK_WAIT_SECS
        let reported = if let Some((url, ping)) = &self.loot_hook {
            let m = loot::message(&stats, &session, picture, false);
            match crate::webhook::send_and_wait(
                url,
                ping,
                m,
                Duration::from_secs(loot::WEBHOOK_WAIT_SECS),
            ) {
                Ok(None) => self.log(format!("Discord: run #{} sent", stats.run)),
                Ok(Some(note)) => self.log(format!("Discord: run #{} {note}", stats.run)),
                Err(e) => self.log(format!("WARN Discord: run #{} - {e}", stats.run)),
            }
            true
        } else {
            false
        };
        let _ = self.tx.send(Event::Run(stats, reported));
        // the game wants the item clicked again to go on
        if let Some((_, at)) = &charm {
            let p = super::loot::click_point(*at);
            self.click(self.main, p, "dropped item")?;
            self.sleep(800)?;
        }
        Ok(())
    }

    /// What's on the main's screen right now, to get back on track after a
    /// hiccup: "end", "lobby", "stage" or "".
    pub(crate) fn where_am_i(&self) -> &'static str {
        let main = self.screen(self.main);
        if main
            .find(
                &Self::names(&[
                    "victory",
                    "defeat",
                    "next_button",
                    "defeat_next_btn",
                    "victory_next_btn",
                    "replay",
                ]),
                None,
            )
            .is_some()
        {
            "end"
        } else if main.frame().and_then(|f| main.ocr_end(&f)).is_some() {
            "end"
        } else if main
            .find(&Self::names(&["play_button", "areas_button"]), None)
            .is_some()
        {
            "lobby"
        } else if main.find(&Self::names(&["vote_start"]), None).is_some() {
            "stage"
        } else {
            ""
        }
    }

    /// Every few minutes: a few clicks (+ Cancel) on each alt, so Roblox's
    /// 20-minute idle kick never hits an alt that's just standing there.
    fn anti_afk(&self, finished: &std::sync::atomic::AtomicBool) {
        use std::sync::atomic::Ordering;
        let every = Duration::from_secs_f64(self.secs(self.settings.afk_every_min as f64 * 60.0));
        if self.settings.afk_every_min == 0 {
            return;
        }
        let Ok(keys) = Keys::from(&self.settings.keybinds) else {
            return;
        };
        let mut last = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(200));
            if finished.load(Ordering::Relaxed) || self.control.is_stopped() {
                return;
            }
            if self.control.is_paused() || last.elapsed() < every {
                continue;
            }
            last = Instant::now();
            let (x, y) = self.settings.idle_click_at;
            let alts: Vec<Hwnd> = self
                .clients
                .iter()
                .map(|c| c.0)
                .filter(|h| *h != self.main)
                .collect();
            for h in alts {
                // never in the middle of placing / upgrading a unit: wait
                // for that sequence to finish (it's input anyway, no AFK)
                let Some(_seq) = seq_lock(h, Duration::from_secs(30)) else {
                    self.log("anti-AFK: alt busy placing/upgrading - skipped this time");
                    continue;
                };
                if self.control.is_stopped() || finished.load(Ordering::Relaxed) {
                    return;
                }
                let mut b = Batch::new(h, "anti-AFK");
                for _ in 0..5 {
                    b = b.click(x, y).wait(150);
                }
                b = b.then(Action::KeyTap {
                    key: keys.cancel,
                    hold_ms: 40,
                });
                match self.arb.run(b) {
                    Ok(_) => self.log("anti-AFK: clicked on an alt"),
                    Err(e) => self.log(format!("anti-AFK: {e}")),
                }
            }
        }
    }

    /// One stage: (lobby ->) setup -> place -> upgrade -> end -> Replay / lobby.
    fn one_round(
        &self,
        in_lobby: &mut bool,
        map: &mut String,
        last: &dyn Fn() -> bool,
        resume: &mut Option<super::resume::Saved>,
    ) -> R<()> {
        let mode = self.settings.run.mode.label();
        // a saved step to pick up at (restart / hiccup mid-stage)
        if let Some(r) = resume.take() {
            if self.where_am_i() == "stage" {
                let m = r.step_params["map"].as_str().unwrap_or("").to_string();
                match r.step.as_str() {
                    "upgrade" => {
                        let placed: Vec<(Hwnd, usize, Point)> = super::resume::placed(&r)
                            .into_iter()
                            .filter_map(|(n, slot, x, y)| {
                                self.clients
                                    .get(n.checked_sub(1)?)
                                    .map(|c| (c.0, slot, Point::new(x, y)))
                            })
                            .collect();
                        if !placed.is_empty() {
                            self.log(format!(
                                "resuming: upgrading the {} unit(s) placed before ({} s ago)",
                                placed.len(),
                                r.age_secs()
                            ));
                            *map = m;
                            return self.round_from_upgrade(in_lobby, map, last, placed);
                        }
                    }
                    "place" | "vote" => {
                        let main = self.screen(self.main);
                        let vote_up = main.find_vote_yes().is_some();
                        if super::resume::resume_at(&r.step, vote_up)
                            == Some(super::steps::Step::Vote)
                        {
                            self.log("resuming: voting, then placing");
                            for h in self.main_last(self.clients.iter().map(|c| c.0).collect()) {
                                if let Some(p) = self.screen(h).find_vote_yes() {
                                    self.click(h, p, "vote start")?;
                                }
                            }
                        } else {
                            self.log(format!("resuming: placing on {m} ({} s ago)", r.age_secs()));
                        }
                        *map = m;
                        return self.round_from_place(in_lobby, map, last);
                    }
                    _ => {}
                }
            } else {
                self.log("saved step not resumed: not in a stage any more");
            }
        }
        if *in_lobby {
            super::trace::step("lobby", serde_json::json!({ "mode": mode }));
            self.enter_stage()?;
            map.clear();
        }
        super::trace::step(
            "stage_setup",
            serde_json::json!({ "mode": mode, "map": map.clone() }),
        );
        *map = self.stage_setup(map)?;
        self.round_from_place(in_lobby, map, last)
    }

    /// The rest of a round from placing on.
    fn round_from_place(
        &self,
        in_lobby: &mut bool,
        map: &mut String,
        last: &dyn Fn() -> bool,
    ) -> R<()> {
        let mode = self.settings.run.mode.label();
        self.cards_all()?;
        let started = Instant::now();
        super::trace::step(
            "place",
            serde_json::json!({ "mode": mode, "map": map.clone() }),
        );
        let placed = self.place_all(map)?;
        self.log(format!(
            "placed {} unit(s) in {:.0}s",
            placed.len(),
            started.elapsed().as_secs_f32()
        ));
        self.round_from_upgrade(in_lobby, map, last, placed)
    }

    /// The rest of a round from upgrading on.
    fn round_from_upgrade(
        &self,
        in_lobby: &mut bool,
        map: &mut String,
        last: &dyn Fn() -> bool,
        placed: Vec<(Hwnd, usize, Point)>,
    ) -> R<()> {
        let mode = self.settings.run.mode.label();
        self.place_to_upgrade(&placed)?;
        super::trace::step(
            "upgrade",
            serde_json::json!({
                "mode": mode,
                "map": map.clone(),
                "placed": placed_json(&self.clients, &placed),
            }),
        );
        let won = self.upgrade_until_end(&placed)?;
        super::trace::step(
            "end",
            serde_json::json!({ "mode": mode, "map": map.clone(), "won": won }),
        );
        *in_lobby = self.end_round(won, map, last(), &EndOnce::default())?;
        Ok(())
    }

    /// End step: end-screen drops + pearls, Win / Loss, then Next > reward
    /// screen (loot report, item click) > Replay / Return. `rewarded` makes
    /// the loot part happen once even when this is run again (a retry).
    /// Ok(true) = back in the lobby.
    pub(crate) fn end_round(&self, won: bool, map: &str, last: bool, once: &EndOnce) -> R<bool> {
        use std::sync::atomic::Ordering;
        let first = !once.reported.load(Ordering::Relaxed);
        let (end_drops, pearls) = if first {
            // rewards on the end screen (Gems, Summer Coins, Remnants, ...)
            let end_drops = {
                let main = self.screen(self.main);
                let mut drops = vec![];
                // quick: one look (two if nothing read) - Next comes first
                for _ in 0..2 {
                    self.sleep(600)?;
                    if let Some(f) = main.frame() {
                        drops = super::drops::parse(&main.ocr_text_lines(&f));
                    }
                    if !drops.is_empty() {
                        break;
                    }
                }
                if !drops.is_empty() {
                    let list: Vec<String> = drops.iter().map(|(n, a)| format!("{a} {n}")).collect();
                    self.log(format!("drops: {}", list.join(", ")));
                    let _ = self.tx.send(Event::Drops(drops.clone()));
                }
                drops
            };
            // the pearls (Summer Event currency) on the VICTORY / DEFEAT panel
            let pearls = self.read_pearls()?;
            let _ = self.tx.send(if won { Event::Win } else { Event::Loss });
            once.reported.store(true, Ordering::Relaxed);
            *once.read.lock().unwrap_or_else(|e| e.into_inner()) = (end_drops.clone(), pearls);
            (end_drops, pearls)
        } else {
            once.read.lock().unwrap_or_else(|e| e.into_inner()).clone()
        };
        let stop = !won && self.settings.run.stop_after_defeat;
        // Next > reward screen: loot scan, Discord report, item click;
        // then (last run of a schedule step: back to the lobby for the next one)
        let mut on_rewards = |c: &Self| {
            if once.rewarded.swap(true, Ordering::Relaxed) {
                return Ok(());
            }
            c.end_rewards(won, map, &end_drops, pearls)
        };
        if stop {
            // still record the run, but don't leave the end screen
            let _ = on_rewards(self);
            return Err("stopped after a defeat".into());
        }
        self.after_end_with(self.settings.run.return_to_lobby || last, &mut on_rewards)
    }

    /// The whole loop until Stop / an error.
    pub fn run(&self) {
        crate::debugbus::set_clients(&self.clients.iter().map(|c| c.0).collect::<Vec<_>>());
        super::resume::install();
        let finished = std::sync::atomic::AtomicBool::new(false);
        let res = std::thread::scope(|sc| {
            // alts that sit idle get kicked after 20 min: poke them every few minutes
            sc.spawn(|| self.anti_afk(&finished));
            // disconnect box / captcha on any client
            sc.spawn(|| self.watchdog(&finished));
            // Challenge runs from the next :00 / :30 on
            super::challenge::reset();
            let r = (|| -> R<()> {
                if self.settings.run.own_team
                    && self.settings.run.party_invite
                    && self.clients.len() > 1
                {
                    self.party_up()?;
                }
                let sched = &self.settings.schedule;
                let steps: Vec<(usize, &crate::config::ScheduleStep)> = sched
                    .steps
                    .iter()
                    .enumerate()
                    .filter(|(_, st)| st.enabled)
                    .collect();
                if !sched.enabled || steps.is_empty() {
                    return self.run_loop(None, true);
                }
                // schedule: one step after another, each with its own settings
                let mut in_lobby = true;
                let mut resume_once = self.resume.clone();
                loop {
                    for (i, st) in &steps {
                        let mut ctx = self.clone();
                        ctx.settings = st.apply(&self.settings);
                        // a saved step only applies to the first step run
                        ctx.resume = resume_once.take();
                        self.log(format!(
                            "schedule: step {} of {} - {}",
                            i + 1,
                            sched.steps.len(),
                            st.label()
                        ));
                        let limit = StepLimit {
                            runs: st.runs,
                            until: (st.minutes > 0).then(|| {
                                Instant::now() + Duration::from_secs(st.minutes as u64 * 60)
                            }),
                        };
                        ctx.run_loop(Some(limit), in_lobby)?;
                        in_lobby = true;
                    }
                    if !sched.repeat {
                        self.log("schedule: all steps done");
                        return Ok(());
                    }
                    self.log("schedule: starting again from the first step");
                }
            })();
            finished.store(true, std::sync::atomic::Ordering::Relaxed);
            r
        });
        let _ = self.tx.send(Event::Done(match res {
            Ok(()) => "finished".into(),
            Err(e) => e,
        }));
    }

    /// Rounds until Stop / an error, or until `limit` is reached (then
    /// back in the lobby, Ok).
    fn run_loop(&self, limit: Option<StepLimit>, start_in_lobby: bool) -> R<()> {
        if self.settings.step_machine {
            return super::machine::run_rounds(self, limit, start_in_lobby);
        }
        {
            {
                let mut in_lobby = start_in_lobby;
                let mut map = String::new();
                let mut rounds = 0u32;
                let reached = |rounds: u32| {
                    limit.as_ref().is_some_and(|l| {
                        (l.runs > 0 && rounds >= l.runs)
                            || l.until.is_some_and(|t| Instant::now() >= t)
                    })
                };
                let mut fails = 0;
                let mut blocked_told = false;
                let mut blocked_n = 0u32;
                let mut resume = self.resume.clone();
                if resume.is_some() {
                    in_lobby = false;
                }
                // a hiccup mid-stage: pick up at the step it was in
                let again = super::resume::from_trace;
                loop {
                    let done_after = rounds + 1;
                    // Challenge reset passed: one Challenge run from the lobby
                    if in_lobby && resume.is_none() && self.challenge_owed() {
                        self.run_challenge_now()?;
                        in_lobby = true;
                        map.clear();
                    }
                    match self.one_round(
                        &mut in_lobby,
                        &mut map,
                        &|| reached(done_after) || self.challenge_owed(),
                        &mut resume,
                    ) {
                        Ok(()) => {
                            fails = 0;
                            blocked_told = false;
                            blocked_n = 0;
                            rounds += 1;
                            if reached(rounds) {
                                // make sure we're in the lobby for the next step
                                if !in_lobby {
                                    let _ = self.return_to_lobby_now();
                                }
                                return Ok(());
                            }
                        }
                        Err(e) if e == "stopped" || e.starts_with("stopped after") => {
                            return Err(e)
                        }
                        Err(e) if e.contains("input blocked") => {
                            // locked PC / screen off: not the game's fault - wait, don't give up.
                            // Only ping once it keeps happening (a blip is retried quietly)
                            blocked_n += 1;
                            self.log(format!(
                                "input refused by Windows ({blocked_n}x) - retrying"
                            ));
                            if blocked_n >= 3 && !blocked_told {
                                let why = if e.contains("Remote Desktop") {
                                    "Input is blocked: the Remote Desktop window is minimized or disconnected (Windows stops all input then) - restore it and the macro carries on"
                                } else {
                                    "Input is blocked (PC locked, screen off or a UAC prompt) - the macro waits and carries on once it's unlocked"
                                };
                                let _ = self.tx.send(Event::Attention(why.into()));
                                blocked_told = true;
                            }
                            self.sleep(if blocked_n >= 3 { 10_000 } else { 2_000 })?;
                            let at = self.where_am_i();
                            match at {
                                "end" => in_lobby = self.after_end().unwrap_or(in_lobby),
                                "lobby" => in_lobby = true,
                                _ => {
                                    in_lobby = false;
                                    resume = again();
                                }
                            }
                        }
                        Err(e) => {
                            if e != "stopped" {
                                self.dump_failure(&super::trace::current_name(), &e, None, vec![]);
                            }
                            // a client dropped out (Roblox "Disconnected" / error
                            // code box): reconnect, else relaunch everything
                            if e.starts_with(DISCONNECTED) || self.disconnected().is_some() {
                                match self.recover_disconnect() {
                                    Ok(()) => {
                                        in_lobby = true;
                                        resume = None;
                                        fails = 0;
                                        self.sleep(500)?;
                                        continue;
                                    }
                                    Err(r) => {
                                        return Err(format!(
                                            "disconnected from the game - reconnect failed: {r}"
                                        ))
                                    }
                                }
                            }
                            fails += 1;
                            if fails > 3 {
                                return Err(format!("{e} (3 tries in a row)"));
                            }
                            // find out where we are and carry on from there
                            let at = self.where_am_i();
                            self.log(format!(
                                "hiccup: {e} - on screen: {} - carrying on",
                                if at.is_empty() { "unknown" } else { at }
                            ));
                            match at {
                                "end" => in_lobby = self.after_end().unwrap_or(in_lobby),
                                "lobby" => in_lobby = true,
                                _ => {
                                    in_lobby = false;
                                    resume = again();
                                    self.sleep(3000)?;
                                }
                            }
                        }
                    }
                    self.sleep(500)?;
                }
            }
        }
    }

    /// Recovery watchdog: every few seconds, every client - disconnect box
    /// (your `disconnect_box` picture, or its words), captcha (`captcha`
    /// picture, or its words). Captcha: pause + ping until it's gone.
    fn watchdog(&self, finished: &std::sync::atomic::AtomicBool) {
        use super::watchdog::{EVERY, OCR_EVERY_NTH};
        use std::sync::atomic::Ordering;
        let mut n: u32 = 0;
        while !finished.load(Ordering::Relaxed) && !self.control.is_stopped() {
            n += 1;
            self.watch_once(n.is_multiple_of(OCR_EVERY_NTH));
            // the watchdog's own wait ignores pause (it has to see the
            // captcha go away while paused)
            let until = Instant::now() + Duration::from_secs_f64(self.secs(EVERY.as_secs_f64()));
            while Instant::now() < until {
                if finished.load(Ordering::Relaxed) || self.control.is_stopped() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    /// One watchdog look at every client (`ocr`: also read the screen when
    /// no picture matched). Captcha: pause + ping; gone: carry on.
    pub(crate) fn watch_once(&self, ocr: bool) {
        use super::watchdog::{classify, Kind};
        use std::sync::atomic::Ordering;
        for (i, (h, scr)) in self.clients.iter().enumerate() {
            let pic = |name: &str| {
                scr.has(name) && scr.find_picture_wide(&Self::names(&[name])).is_some()
            };
            let (dis, cap) = (pic("disconnect_box"), pic("captcha"));
            let text = (ocr && !dis && !cap)
                .then(|| scr.frame().and_then(|f| scr.ocr_all(&f)))
                .flatten();
            let kind = classify(dis, cap, text.as_deref());
            if self.watch.set(*h, i + 1, kind) {
                let what = kind.map(Kind::label).unwrap_or("");
                self.log(format!("watchdog: client {} - {what}", i + 1));
            }
            crate::status::set_incident(*h, kind.map(Kind::label));
        }
        // captcha: hold everything until it's typed
        let captcha = self.watch.first(Kind::Captcha);
        let ours = self.watch.captcha_paused.load(Ordering::Relaxed);
        match (captcha, ours) {
            (Some(c), false) if self.settings.recovery.captcha_pause => {
                self.watch.captcha_paused.store(true, Ordering::Relaxed);
                self.control.set_paused(true);
                let why = format!(
                    "Captcha on client {} - type it in Roblox, the macro is paused until it's gone",
                    c.client
                );
                self.log(format!("WARN {why}"));
                let _ = self.tx.send(Event::Captcha(why));
            }
            (None, true) => {
                self.watch.captcha_paused.store(false, Ordering::Relaxed);
                self.control.set_paused(false);
                self.log("captcha gone - carrying on");
                let _ = self.tx.send(Event::CaptchaGone);
            }
            _ => {}
        }
    }

    /// Dungeon floor (skeleton): until the end screen, every few seconds -
    /// shop open: buy from your list; shrine: switch it per your rules;
    /// doors (your door_* pictures): take the one highest on your room
    /// order. Ok(won).
    pub(crate) fn dungeon_floor(&self) -> R<bool> {
        use super::dungeon::{choose_buys, choose_door, shop_items, shrine_action, RoomKind};
        let d = &self.settings.dungeon;
        let end = Self::names(&["victory", "defeat"]);
        let main = self.screen(self.main);
        let mut told = false;
        loop {
            self.check()?;
            if let Some(won) = self.end_screen(&end)? {
                return Ok(won);
            }
            let f = main.frame();
            let lines = f
                .as_ref()
                .and_then(|f| main.ocr_lines(f))
                .unwrap_or_default();
            let text: String = lines
                .iter()
                .flat_map(|l| l.iter().map(|(w, _)| w.to_lowercase()))
                .collect::<Vec<_>>()
                .join(" ");
            // the shop
            if d.shop_enabled && text.contains("shop") && !d.shop_priority.is_empty() {
                let items = shop_items(&lines);
                let money = main
                    .frame()
                    .and_then(|f| main.ocr_rect(&f, crate::geom::Rect::new(300, 470, 200, 30)))
                    .and_then(|t| super::loot::parse_amount(&t))
                    .unwrap_or(0);
                for i in choose_buys(&items, &d.shop_priority, money, d.shop_keep) {
                    self.log(format!(
                        "dungeon shop: buying {} ({})",
                        items[i].name, items[i].price
                    ));
                    self.click(self.main, items[i].at, "dungeon buy")?;
                    self.sleep(600)?;
                }
            }
            // a shrine: "<name> ... on/off"
            if text.contains("shrine") {
                for l in &lines {
                    let t: String = l
                        .iter()
                        .map(|(w, _)| w.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    if !t.to_lowercase().contains("shrine") {
                        continue;
                    }
                    let on = t.to_lowercase().contains(" on");
                    let name = t.split([':', '(']).next().unwrap_or(&t).trim().to_string();
                    if let Some(want) = shrine_action(&name, on, &d.shrines) {
                        self.log(format!(
                            "dungeon shrine: {name} -> {}",
                            if want { "on" } else { "off" }
                        ));
                        let r = l[0].1;
                        self.click(
                            self.main,
                            Point::new(r.x + r.w / 2, r.y + r.h / 2),
                            "shrine",
                        )?;
                        self.sleep(600)?;
                    }
                }
            }
            // doors
            let doors: Vec<(RoomKind, Point)> = RoomKind::ALL
                .into_iter()
                .filter(|k| main.has(k.door_pattern()))
                .filter_map(|k| {
                    main.find_picture_wide(&Self::names(&[k.door_pattern()]))
                        .map(|p| (k, p))
                })
                .collect();
            if doors.is_empty() && !told {
                told = true;
                self.log("dungeon: no door pictures found (capture door_item / door_gold / door_shrine / door_shop / door_combat / door_boss with F6)");
            }
            if let Some(i) = choose_door(&doors, &d.room_priority) {
                let (k, p) = doors[i];
                self.log(format!(
                    "dungeon: {} door (of {}) - your room order",
                    k.label(),
                    doors
                        .iter()
                        .map(|d| d.0.label())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                self.click(self.main, p, "dungeon door")?;
                self.sleep(2500)?;
                continue;
            }
            self.sleep(2000)?;
        }
    }

    /// Disconnect recovery: Reconnect on every disconnected client, wait
    /// for the lobby, party up again. The stage is lost (an aborted run, no
    /// Discord report); the loop goes on from the lobby. Err = couldn't
    /// (then the full relaunch takes over, if it's on).
    pub(crate) fn recover_disconnect(&self) -> R<()> {
        use super::watchdog::{Kind, Recovering, RECONNECT_WAIT_SECS};
        let rec = &self.settings.recovery;
        if !rec.reconnect {
            return Err("reconnect is off (Settings > Recovery)".into());
        }
        if !self.watch.allow_reconnect(rec.max_reconnects_per_hour) {
            return Err(format!(
                "{} reconnects already this hour",
                rec.max_reconnects_per_hour
            ));
        }
        let _r = Recovering::new(&self.watch);
        // which clients: what the watchdog saw, else a fresh look
        let mut who: Vec<(Hwnd, usize)> = self
            .watch
            .incidents()
            .into_iter()
            .filter(|i| i.kind == Kind::Disconnected)
            .map(|i| (i.hwnd, i.client))
            .collect();
        if who.is_empty() {
            for (i, (h, scr)) in self.clients.iter().enumerate() {
                let dis = (scr.has("disconnect_box")
                    && scr
                        .find_picture_wide(&Self::names(&["disconnect_box"]))
                        .is_some())
                    || scr
                        .frame()
                        .and_then(|f| scr.ocr_all(&f))
                        .is_some_and(|t| disconnect_text(&t));
                if dis {
                    who.push((*h, i + 1));
                }
            }
        }
        let list: Vec<String> = who.iter().map(|(_, n)| format!("client {n}")).collect();
        self.log(format!(
            "disconnected: {} - run aborted, reconnecting",
            if list.is_empty() {
                "?".into()
            } else {
                list.join(", ")
            }
        ));
        let _ = self.tx.send(Event::Reconnect(format!(
            "Disconnected ({}) - reconnecting",
            list.join(", ")
        )));
        for (h, n) in &who {
            let scr = self.screen(*h);
            let Some(p) = scr.find_wide(&Self::names(&["reconnect_button"])) else {
                return Err(format!(
                    "client {n}: no Reconnect button on its disconnect box"
                ));
            };
            self.click(*h, p, "reconnect")?;
            // back in the lobby: the box gone and the lobby's buttons there
            let mut dl = self.control.deadline(self.secs(RECONNECT_WAIT_SECS));
            loop {
                self.sleep(2000)?;
                let lobby = scr
                    .find(&Self::names(&["play_button", "areas_button"]), None)
                    .is_some();
                if lobby {
                    self.watch.set(*h, *n, None);
                    crate::status::set_incident(*h, None);
                    self.log(format!("client {n}: reconnected - in the lobby"));
                    break;
                }
                if dl.expired() {
                    return Err(format!(
                        "client {n}: not back in the lobby {RECONNECT_WAIT_SECS:.0} s after Reconnect"
                    ));
                }
            }
        }
        self.watch.clear();
        if self.settings.run.own_team && self.settings.run.party_invite && self.clients.len() > 1 {
            self.party_up()?;
        }
        let _ = self.tx.send(Event::Reconnect(
            "Reconnected - next round from the lobby".into(),
        ));
        self.to_main()?;
        Ok(())
    }

    /// A client showing Roblox's "Disconnected" / "Error Code" box.
    pub(crate) fn disconnected(&self) -> Option<String> {
        for (i, (_, scr)) in self.clients.iter().enumerate() {
            let Some(f) = scr.frame() else { continue };
            let Some(t) = scr.ocr_all(&f) else { continue };
            if disconnect_text(&t) {
                return Some(format!("client {}", i + 1));
            }
        }
        None
    }

    /// Leave the stage for the lobby (end screen or mid-stage).
    pub(crate) fn return_to_lobby_now(&self) -> R<()> {
        if self.where_am_i() == "end" {
            self.after_end_to(true)?;
        }
        Ok(())
    }
}

/// Roblox's disconnect box ("Disconnected", "Error Code: 277", "lost
/// connection", "You were kicked").
pub fn disconnect_text(t: &str) -> bool {
    let t = t.to_lowercase();
    t.contains("error code")
        || (t.contains("disconnected") && (t.contains("reconnect") || t.contains("leave")))
        || t.contains("lost connection")
        || t.contains("you were kicked")
}

/// When a schedule step is done.
#[derive(Clone, Copy)]
pub(crate) struct StepLimit {
    pub(crate) runs: u32,
    pub(crate) until: Option<Instant>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn disconnect_box() {
        assert!(disconnect_text(
            "Disconnected\nLost connection to the game server (Error Code: 277)\nLeave Reconnect"
        ));
        assert!(!disconnect_text("Wave 12 Vote Start"));
    }

    #[test]
    fn map_by_ocr_text() {
        let known: Vec<String> = [
            "Future City",
            "Future City (Tyrant)",
            "Ruined City",
            "Contracts",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            match_map("LOADING... FUTURE CITY (TYRANT) Tip: ...", &known).as_deref(),
            Some("Future City (Tyrant)")
        );
        assert_eq!(
            match_map("Ruined  City", &known).as_deref(),
            Some("Ruined City")
        );
        assert_eq!(match_map("Wave 1", &known), None);
        let k2: Vec<String> = ["Puppet Island", "Magic Hills", "Planet Greenie"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            match_map("Puppet Island (Summer) Summer Hunt 0.0/68250", &k2).as_deref(),
            Some("Puppet Island")
        );
        assert_eq!(
            match_map("Magic Hills Summer Hunt", &k2).as_deref(),
            Some("Magic Hills")
        );
    }

    #[test]
    fn plan_per_mode() {
        let routes = Routes::load(Path::new("/nonexistent"));
        let mut s = Settings::default();
        s.units[0].enabled = true;
        s.run.mode = Mode::SummerHunt;
        let p = plan(&s, &routes);
        let all: Vec<String> = p.iter().flat_map(|x| x.needs.clone()).collect();
        for n in [
            "play_button",
            "join_matchmaking",
            "play_here",
            "open_portal",
            "open_button",
            "start_button",
            "victory",
            "replay",
        ] {
            assert!(all.iter().any(|a| a == n), "summer needs {n}");
        }
        s.run.own_team = true;
        s.run.party_invite = false; // no party: alts walk into the portal
        let all: Vec<String> = plan(&s, &routes)
            .iter()
            .flat_map(|x| x.needs.clone())
            .collect();
        assert!(
            all.iter().any(|a| a == "open_friends_only")
                && all.iter().any(|a| a == "portal_join_e")
        );

        s.run.mode = Mode::Legend;
        s.run.map = "Ruined City (Midnight)".into();
        s.run.act = "Legend Stage 3".into();
        s.run.return_to_lobby = true;
        let p = plan(&s, &routes);
        let all: Vec<String> = p.iter().flat_map(|x| x.needs.clone()).collect();
        for n in [
            "areas_button",
            "play_area_button",
            "select_world",
            "world_ruined_city_midnight",
            "act_legend_stage_3",
            "select_button",
            "return_to_lobby",
        ] {
            assert!(all.iter().any(|a| a == n), "legend needs {n}");
        }
        assert!(p.iter().any(|x| x.label == "Map walk: Ruined City"));
        assert!(!all.iter().any(|a| a == "open_portal"));
    }

    #[test]
    fn route_map_strips_variant() {
        assert_eq!(route_map("Ruined City (Midnight)"), "Ruined City");
        assert_eq!(route_map("Space Center"), "Space Center");
    }
}

#[cfg(test)]
mod seq_tests {
    use super::*;

    #[test]
    fn seq_lock_waits_for_the_holder() {
        let h = Hwnd(0xBEEF01);
        let g = seq_lock(h, Duration::from_millis(10)).expect("free");
        // busy: a second taker gives up after its wait
        assert!(seq_lock(h, Duration::from_millis(50)).is_none());
        // another client is unaffected
        assert!(seq_lock(Hwnd(0xBEEF02), Duration::from_millis(10)).is_some());
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            drop(g);
        });
        // freed while waiting = taken
        assert!(seq_lock(h, Duration::from_secs(2)).is_some());
        t.join().unwrap();
    }
}

#[cfg(test)]
mod unit_click_tests {
    use super::*;

    #[test]
    fn retries_look_a_bit_above_the_placed_spot() {
        let at = Point::new(370, 414);
        assert_eq!(unit_click_point(at, 0), at);
        assert_eq!(unit_click_point(at, 1), Point::new(370, 400));
        assert_eq!(unit_click_point(at, 2), Point::new(358, 390));
        assert_eq!(unit_click_point(at, 3), Point::new(382, 390));
        // never off the client
        assert_eq!(unit_click_point(Point::new(5, 10), 2), Point::new(1, 1));
    }
}
