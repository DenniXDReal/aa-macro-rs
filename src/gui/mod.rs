//! Anime Adventure Macro By DenniXD - the app window.
//!
//! Shell (Command Center): top bar (title, Start/Pause/Stop, status),
//! left sidebar, collapsible log dock at the bottom, pages.
//! Pages: Dashboard (2x2 client cards, Roblox docked in), Patterns, Routes
//! (Movement), Settings, Logs; setup: Run setup (Home), Accounts, Placement.

mod command;
mod debug_tp; // TEMPORARY - remove with tp_dbg_click and its 2 calls
mod tools;

use crate::accounts::{self, AccountRole, AccountSlot, Ram};
use crate::capture::Frame;
use crate::config::{Mode, PlayChoice, Settings, Upgrade};
use crate::control::Control;
use crate::geom::{Point, Rect};
use crate::input::Arbiter;
use crate::movement::{Route, Routes, Stages};
use crate::placement::Placements;
use crate::platform::{Platform, WinInfo};
use crate::secrets::Secrets;
use crate::update::{self, Available};
use crate::vision::{Hit, Library};
use eframe::egui::{
    self, Align, Color32, ColorImage, ComboBox, CornerRadius, Layout, RichText, Sense, Stroke,
    TextureHandle, TextureOptions, Vec2,
};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const APP_NAME: &str = "Anime Adventure Macro";
pub const APP_BY: &str = "By DenniXD";

// ---- theme: minimal - black, dark blue, dark purple, purple, white ----
const BG: Color32 = Color32::from_rgb(6, 6, 12); // near black
const PANEL: Color32 = Color32::from_rgb(10, 12, 26); // dark blue
const CARD: Color32 = Color32::from_rgb(18, 14, 34); // dark purple
const CARD_HI: Color32 = Color32::from_rgb(28, 20, 52);
const LINE: Color32 = Color32::from_rgb(34, 30, 62);
const PURPLE: Color32 = Color32::from_rgb(150, 110, 255);
const PURPLE_DIM: Color32 = Color32::from_rgb(76, 52, 150);
const TEXT: Color32 = Color32::from_rgb(245, 244, 250); // white
const MUTED: Color32 = Color32::from_rgb(138, 136, 170);
const ACCENT: Color32 = PURPLE;
const GOOD: Color32 = Color32::from_rgb(74, 222, 128);
const WARN: Color32 = Color32::from_rgb(251, 191, 36);
const BAD: Color32 = Color32::from_rgb(248, 113, 113);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Dashboard,
    Logs,
    Home,
    Accounts,
    Movement,
    Placement,
    Patterns,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    General,
    Input,
    Game,
    Discord,
    Weather,
    Recovery,
    Updates,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HomeTab {
    Control,
    Schedule,
    Sequence,
    Activity,
}

#[derive(Clone)]
enum Upd {
    Idle,
    Checking,
    UpToDate,
    Available(Available),
    Installing,
    Installed(PathBuf),
    Failed(String),
}

pub struct App {
    settings: Settings,
    settings_path: PathBuf,
    dirty_at: Option<Instant>,
    lib: Library,
    placements: Placements,
    placements_path: PathBuf,
    platform: Arc<dyn Platform>,
    control: Control,
    arbiter: Option<Arbiter>,
    clients: Vec<WinInfo>,
    sel: usize,
    /// Which client is the main (the rest are alts).
    main: usize,
    page: Page,
    settings_tab: SettingsTab,
    /// Saved step waiting for 'resume or start fresh?'.
    resume_prompt: Option<crate::game::resume::Saved>,
    resume_answer: Option<bool>,
    home_tab: HomeTab,

    // live picture of the selected client
    frame: Option<Frame>,
    tex: Option<TextureHandle>,
    live: bool,
    last_cap: Instant,
    // alt thumbnails
    alt_tex: Vec<Option<TextureHandle>>,
    last_alt_cap: Instant,

    // pattern tool
    drag_from: Option<Point>,
    crop: Option<Rect>,
    pat_name: String,
    pat_image: bool,
    pat_auto: bool,
    pat_thr: u8,
    pat_margin: i32,
    pat_preview: Option<TextureHandle>,
    detect_map: String,
    /// Summer minimaps waiting for a name: file -> typed name / thumbnail.
    mm_names: std::collections::HashMap<String, String>,
    mm_tex: std::collections::HashMap<String, TextureHandle>,
    /// TEMPORARY (debug_tp): where the TP click would land.
    tp_dbg_click: Option<Point>,
    hits: Vec<(String, Rect, Option<Hit>)>,

    // placement
    map: String,
    new_map: String,
    place_slot: u8,
    show_spiral: bool,

    // movement
    routes: Routes,
    stages: Stages,
    route_sel: String,
    route_lobby: bool,
    route_text: String,
    route_err: Option<String>,

    // accounts (Roblox Account Manager)
    ram_state: String,
    ram_ok: Option<bool>,
    ram_list: Vec<String>,
    ram_pw_input: String,
    ps_input: String,
    cookie_input: String,
    ram_rx: Receiver<RamMsg>,
    ram_tx: Sender<RamMsg>,

    // docking
    dock_rect: Option<egui::Rect>,
    /// Command Center dashboard (cards, multi-client dock).
    dash: command::Dash,
    /// Settings > Weather editor text (None = not loaded yet).
    weather_text: Option<String>,
    /// The game loop sends the Discord loot report (no plain win / loss).
    loot_reports: bool,
    /// Another copy of the macro was already running at launch.
    other_instance: bool,
    own_hwnd: Option<crate::geom::Hwnd>,
    last_dock: Instant,
    docked_ok: bool,
    /// Window we last made topmost (so we only touch z-order on change).
    top_hwnd: Option<crate::geom::Hwnd>,
    dock_note: String,

    // multi-Roblox + usernames
    multi: Option<crate::multi::MultiLock>,
    /// Roblox pid -> RAM username (learned when we launch an account).
    names: std::collections::HashMap<u32, String>,
    last_refresh: Instant,
    /// When we last tried to work out a client's username (by pid).
    name_tries: std::collections::HashMap<u32, Instant>,
    /// Roblox log file already matched to each client (so two clients
    /// never get the same name).
    name_logs: std::collections::HashMap<u32, std::path::PathBuf>,
    /// Roblox runs as admin and we don't: Windows blocks docking/capture/input.
    admin_needed: bool,

    // game loop
    run_rx: Option<Receiver<crate::game::run::Event>>,
    /// Auto-relaunch after the loop got stuck: when it began / when the
    /// launch finished, and how many times this session.
    relaunch: Option<Instant>,
    relaunch_launched: Option<Instant>,
    relaunches: Vec<Instant>,
    /// Spots where units didn't go down (skipped next time).
    memory: Arc<std::sync::Mutex<crate::spotmem::SpotMemory>>,

    // webhook
    notifier: crate::webhook::Notifier,
    hook_input: String,

    // run state (game loop arrives with your patterns)
    running: bool,
    paused: bool,
    started: Option<Instant>,
    wins: u32,
    losses: u32,
    /// Rewards this session (end screen OCR) and the last round's.
    drops: Vec<(String, u64)>,
    last_drops: Vec<(String, u64)>,
    hotkeys: Receiver<u8>,

    log: Vec<String>,
    log_tx: Sender<String>,
    log_rx: Receiver<String>,

    // updates
    secrets: Secrets,
    token_input: String,
    upd: Upd,
    upd_tx: Sender<Upd>,
    upd_rx: Receiver<Upd>,
}

enum RamMsg {
    Status(bool, String),
    Accounts(Vec<String>),
    Log(String),
    /// A launched account's Roblox window showed up.
    Pid(u32, String),
    /// Name read from a client's Roblox log (that log is now taken).
    PidLog(u32, String, std::path::PathBuf),
    Lock(crate::multi::MultiLock),
    /// A login checked out: (username, user id, cookie) - save it.
    Added(String, u64, String),
    LaunchDone,
}

impl App {
    pub fn new(platform: Arc<dyn Platform>, settings: Settings, settings_path: PathBuf) -> Self {
        // failure dumps: keep the newest ones only
        let _ = crate::game::dump::prune(
            &crate::game::dump::root(),
            crate::game::dump::FAILURE_DUMP_RETENTION,
        );
        let (lib, defaults) = Library::load_with_defaults(&Library::default_dir())
            .unwrap_or_else(|_| (Library::default(), 0));
        let placements_path = Placements::default_path();
        let placements = Placements::load(&placements_path).unwrap_or_default();
        let map = placements
            .maps
            .keys()
            .next()
            .cloned()
            .unwrap_or_else(|| "Space Center".into());
        let routes = Routes::load(&Routes::user_dir_default());
        // running macros pick up pattern / route edits without a restart
        crate::hotreload::watch(Library::default_dir(), routes.user_dir.clone());
        let route_sel = "Space Center".to_string();
        let (log_tx, log_rx) = mpsc::channel();
        let (upd_tx, upd_rx) = mpsc::channel();
        let (ram_tx, ram_rx) = mpsc::channel();
        let (hk_tx, hotkeys) = mpsc::channel();
        #[cfg(windows)]
        crate::platform::win32::spawn_hotkeys(hk_tx, settings.keybinds.overlay_fkey());
        #[cfg(not(windows))]
        drop(hk_tx);
        let mut app = Self {
            settings,
            settings_path,
            dirty_at: None,
            lib,
            placements,
            placements_path,
            platform,
            control: Control::new(),
            arbiter: None,
            clients: vec![],
            sel: 0,
            main: 0,
            page: Page::Dashboard,
            settings_tab: SettingsTab::General,
            resume_prompt: None,
            resume_answer: None,
            home_tab: HomeTab::Control,
            frame: None,
            tex: None,
            live: true,
            last_cap: Instant::now() - Duration::from_secs(10),
            alt_tex: vec![],
            last_alt_cap: Instant::now() - Duration::from_secs(10),
            drag_from: None,
            crop: None,
            pat_name: String::new(),
            pat_image: false,
            pat_auto: true,
            pat_thr: 128,
            pat_margin: 40,
            pat_preview: None,
            detect_map: String::new(),
            mm_names: Default::default(),
            mm_tex: Default::default(),
            tp_dbg_click: None,
            hits: vec![],
            map,
            new_map: String::new(),
            place_slot: 0,
            show_spiral: true,
            route_text: String::new(),
            routes,
            stages: Stages::builtin(),
            route_sel,
            route_lobby: false,
            route_err: None,
            ram_state: "not checked".into(),
            ram_ok: None,
            ram_list: vec![],
            ram_pw_input: String::new(),
            ps_input: String::new(),
            cookie_input: String::new(),
            ram_rx,
            ram_tx,
            dock_rect: None,
            dash: command::Dash::default(),
            other_instance: false,
            loot_reports: false,
            weather_text: None,
            own_hwnd: None,
            last_dock: Instant::now() - Duration::from_secs(10),
            docked_ok: false,
            top_hwnd: None,
            dock_note: String::new(),
            multi: None,
            names: Default::default(),
            last_refresh: Instant::now(),
            name_tries: Default::default(),
            name_logs: Default::default(),
            admin_needed: false,
            run_rx: None,
            relaunch: None,
            relaunch_launched: None,
            relaunches: vec![],
            memory: Arc::new(std::sync::Mutex::new(crate::spotmem::SpotMemory::load(
                &crate::spotmem::SpotMemory::default_path(),
            ))),
            notifier: crate::webhook::Notifier::start(Some(log_tx.clone())),
            hook_input: String::new(),
            running: false,
            paused: false,
            started: None,
            wins: 0,
            losses: 0,
            drops: vec![],
            last_drops: vec![],
            hotkeys,
            log: vec![],
            log_tx,
            log_rx,
            secrets: Secrets::load(&Secrets::default_path()),
            token_input: String::new(),
            upd: Upd::Idle,
            upd_tx,
            upd_rx,
        };
        app.settings.ensure(0);
        app.settings.fix_teams();
        if defaults > 0 {
            app.say(format!(
                "{defaults} built-in pattern(s) loaded (your own captures win)"
            ));
        }
        app.refresh_clients();
        if app.settings.multi_roblox && app.clients.is_empty() {
            // nothing open yet: take Roblox's lock now so every client can open
            match crate::multi::MultiLock::acquire(&app.settings.singleton_names) {
                Ok(l) => {
                    app.say("multi-Roblox on - open as many clients as you like");
                    app.multi = Some(l);
                }
                Err(e) => log::warn!("multi-Roblox: {e}"),
            }
        }
        app.load_route_text();
        app.page = match std::env::var("AA_START_PAGE").as_deref() {
            Ok("accounts") => Page::Accounts,
            Ok("movement") => Page::Movement,
            Ok("placement") => Page::Placement,
            Ok("patterns") => Page::Patterns,
            Ok("settings") => Page::Settings,
            Ok("home") => Page::Home,
            Ok("logs") => Page::Logs,
            _ => Page::Dashboard,
        };
        app.other_instance = app.platform.another_instance();
        if app.other_instance {
            app.say("WARN another copy of the macro is already running - close it (Task Manager) or both will drive Roblox");
        }
        app.say("Ready - nothing runs until you press START ALL (F2)");
        if app.settings.auto_update_check && app.secrets.has(update::TOKEN_KEY) {
            app.check_update();
        }
        app
    }

    // ---------------- run controls ----------------

    fn start(&mut self) {
        // a stage still running from before a restart (last 5 min)?
        let resume = match (self.resume_answer.take(), self.settings.resume_mode) {
            (Some(true), _) => crate::game::resume::recent(),
            (Some(false), _) | (None, crate::config::ResumeMode::Off) => None,
            (None, crate::config::ResumeMode::Auto) => crate::game::resume::recent(),
            (None, crate::config::ResumeMode::Ask) => {
                if let Some(r) = crate::game::resume::recent() {
                    if self.run_rx.is_none() {
                        self.resume_prompt = Some(r);
                        return;
                    }
                }
                None
            }
        };
        if resume.is_none() {
            crate::game::resume::clear();
        }
        if self.run_rx.is_some() {
            self.say("the game loop is still running (or still stopping) - press Stop and give it a second");
            return;
        }
        self.refresh_clients();
        if self.clients.is_empty() {
            self.say("no Roblox clients - open Roblox first");
            return;
        }
        let missing: Vec<String> = crate::game::run::plan(&self.settings, &self.routes)
            .into_iter()
            .filter(|st| !st.optional)
            .flat_map(|st| st.needs)
            .filter(|n| self.lib.get(n).is_none())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        if !missing.is_empty() {
            self.say(format!(
                "can't start yet - capture these on the Patterns page first (F6 over each): {}",
                missing.join(", ")
            ));
            self.home_tab = HomeTab::Activity;
            return;
        }
        self.control.reset();
        let arb = self.arbiter();
        let main = self.clients[self.main.min(self.clients.len() - 1)].hwnd;
        let clients = self
            .clients
            .iter()
            .map(|c| {
                (
                    c.hwnd,
                    crate::game::LiveScreen::new(self.platform.clone(), c.hwnd, &self.lib),
                )
            })
            .collect();
        let (tx, rx) = mpsc::channel();
        // the game loop sends the loot report itself (and waits for it)
        let w = &self.settings.webhook;
        let loot_hook = (w.enabled && w.on_result && w.loot_report)
            .then(|| self.secrets.get(crate::webhook::URL_KEY))
            .flatten()
            .filter(|u| crate::webhook::valid_url(u))
            .map(|u| (u, w.ping_user.clone()));
        let ctx = crate::game::run::RunCtx {
            arb,
            control: self.control.clone(),
            main,
            clients,
            settings: self.settings.clone(),
            routes: self.routes.clone().into(),
            placements: self.placements.clone(),
            tx,
            speed: 1.0,
            memory: self.memory.clone(),
            resume,
            loot_hook: loot_hook.clone(),
            session: Default::default(),
            step_deadline: Default::default(),
            watch: Default::default(),
        };
        self.loot_reports = loot_hook.is_some();
        crate::status::reset();
        std::thread::Builder::new()
            .name("game-loop".into())
            .spawn(move || ctx.run())
            .expect("spawn game loop");
        self.run_rx = Some(rx);
        self.platform.keep_awake(true);
        self.running = true;
        self.paused = false;
        self.started.get_or_insert(Instant::now());
        self.hook(
            crate::webhook::Kind::Info,
            "Started",
            &format!(
                "{} on {}",
                self.settings.run.mode.label(),
                self.client_label(self.main)
            ),
            false,
        );
        self.say(format!(
            "Start: {} on {}",
            self.settings.run.mode.label(),
            self.client_label(self.main)
        ));
    }

    /// "3W / 1L" plus this round's and the session's drops (Discord).
    fn result_text(&mut self) -> String {
        let fmt = |v: &[(String, u64)]| {
            v.iter()
                .map(|(n, a)| format!("{a} {n}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut t = format!("{}W / {}L", self.wins, self.losses);
        let last = std::mem::take(&mut self.last_drops);
        if !last.is_empty() {
            t += &format!("\nDrops: {}", fmt(&last));
        }
        if !self.drops.is_empty() {
            t += &format!("\nSession: {}", fmt(&self.drops));
        }
        t
    }

    /// Events from the game loop thread.
    fn run_events(&mut self) {
        use crate::game::run::Event;
        let Some(rx) = &self.run_rx else { return };
        let evs: Vec<Event> = rx.try_iter().collect();
        for e in evs {
            match e {
                Event::Log(s) => self.say(s),
                Event::Drops(d) => {
                    for (n, a) in &d {
                        match self.drops.iter_mut().find(|(m, _)| m == n) {
                            Some(e) => e.1 += a,
                            None => self.drops.push((n.clone(), *a)),
                        }
                    }
                    self.last_drops = d;
                }
                Event::Win => {
                    self.wins += 1;
                    let text = self.result_text();
                    self.say("VICTORY");
                    if !self.loot_reports {
                        self.hook(crate::webhook::Kind::Win, "Victory", &text, false);
                    }
                }
                Event::Loss => {
                    self.losses += 1;
                    let text = self.result_text();
                    self.say("DEFEAT");
                    if !self.loot_reports {
                        self.hook(crate::webhook::Kind::Loss, "Defeat", &text, false);
                    }
                }
                Event::Run(r, _reported) => {
                    if let Some(p) = &r.image {
                        self.say(format!(
                            "run #{}: item picture saved: {}",
                            r.run,
                            p.display()
                        ));
                    }
                }
                Event::Captcha(s) => {
                    self.paused = true;
                    self.say(format!("WARN {s}"));
                    self.hook_with(
                        crate::webhook::Kind::Problem,
                        "Captcha detected - manual input needed",
                        &s,
                        true,
                        self.settings.recovery.captcha_here,
                    );
                }
                Event::CaptchaGone => {
                    self.paused = false;
                    self.say("captcha gone - carrying on");
                }
                Event::Reconnect(s) => {
                    self.say(s.clone());
                    self.hook(crate::webhook::Kind::Problem, "Reconnect", &s, false);
                }
                Event::Fatal(s) => {
                    self.paused = true;
                    self.say(format!("FATAL {s} - paused, press F4 / Resume to carry on"));
                    self.hook(
                        crate::webhook::Kind::Problem,
                        "Paused (step failed)",
                        &s,
                        true,
                    );
                }
                Event::Attention(s) => {
                    self.say(format!("needs you: {s}"));
                    self.hook(crate::webhook::Kind::Problem, "Needs you", &s, true);
                }
                Event::Done(why) => {
                    self.say(format!("game loop ended: {why}"));
                    self.run_rx = None;
                    self.platform.keep_awake(false);
                    self.running = false;
                    self.paused = false;
                    let by_user = why == "stopped" || why.starts_with("stopped after");
                    if !by_user && self.settings.auto_relaunch && self.begin_relaunch(&why) {
                        continue;
                    }
                    if !by_user {
                        self.hook(crate::webhook::Kind::Problem, "Macro stopped", &why, true);
                    }
                }
            }
        }
    }

    /// Stuck for good: close every Roblox, launch the accounts again (main
    /// first, alts after it loaded), then Start again. Max 5 an hour.
    fn begin_relaunch(&mut self, why: &str) -> bool {
        self.relaunches
            .retain(|t| t.elapsed() < Duration::from_secs(3600));
        if self.relaunches.len() >= 5 {
            self.say("auto-relaunch: 5 already this hour - stopping instead");
            return false;
        }
        let order: Vec<String> = accounts::launch_order(&self.settings.accounts)
            .iter()
            .map(|a| a.username.clone())
            .collect();
        if order.is_empty() {
            self.say("auto-relaunch: no accounts on the Accounts page to launch");
            return false;
        }
        self.relaunches.push(Instant::now());
        self.relaunch = Some(Instant::now());
        self.relaunch_launched = None;
        self.say(format!(
            "stuck ({why}) - closing Roblox and relaunching {} account(s)",
            order.len()
        ));
        self.hook(
            crate::webhook::Kind::Problem,
            "Relaunching",
            &format!("{why} - closing Roblox and relaunching everything"),
            false,
        );
        let _ = self.platform.kill_process(&self.settings.process_name);
        // the lock is ours already; launch() waits for main then alts
        self.ram_launch(order, false);
        true
    }

    fn relaunch_tick(&mut self) {
        let Some(started) = self.relaunch else { return };
        let ready = self
            .relaunch_launched
            .is_some_and(|t| t.elapsed() > Duration::from_secs(45));
        if ready || started.elapsed() > Duration::from_secs(600) {
            self.relaunch = None;
            self.relaunch_launched = None;
            self.refresh_clients();
            if self.clients.is_empty() {
                self.say("auto-relaunch: no Roblox came up - stopped");
                self.hook(
                    crate::webhook::Kind::Problem,
                    "Macro stopped",
                    "relaunch failed: no Roblox clients",
                    true,
                );
                return;
            }
            self.say(format!(
                "relaunched {} client(s) - starting again",
                self.clients.len()
            ));
            self.start();
        }
    }

    fn stop(&mut self) {
        self.relaunch = None;
        self.relaunch_launched = None;
        self.control.stop();
        if self.running {
            self.say("Stopped");
            let (w, l) = (self.wins, self.losses);
            self.hook(
                crate::webhook::Kind::Info,
                "Stopped",
                &format!("{w} win(s), {l} loss(es) this session"),
                false,
            );
        }
        self.running = false;
        self.paused = false;
    }

    fn toggle_pause(&mut self) {
        if !self.running {
            return;
        }
        self.paused = self.control.toggle_pause();
        self.say(if self.paused {
            "Paused (F4 to resume)"
        } else {
            "Resumed"
        });
    }

    fn hotkey(&mut self, n: u8) {
        match n {
            1 => {
                if self.settings.dock {
                    self.last_dock = Instant::now() - Duration::from_secs(10);
                } else {
                    self.tile();
                }
            }
            2 => self.start(),
            3 => self.stop(),
            4 => self.toggle_pause(),
            6 => self.hover_capture(),
            k if k as u32 == self.settings.keybinds.overlay_fkey() => self.toggle_overlay(),
            _ => {}
        }
    }

    /// Debug overlay on / off (F9): boxes and text over the game showing
    /// what the macro sees.
    fn toggle_overlay(&mut self) {
        let on = !crate::debugbus::enabled();
        crate::debugbus::set_show_misses(self.settings.overlay_misses);
        crate::debugbus::set_enabled(on);
        #[cfg(windows)]
        crate::platform::overlay::spawn();
        self.say(if on {
            "debug overlay on"
        } else {
            "debug overlay off"
        });
    }

    /// Queue a Discord message if the webhook is set up.
    fn hook(&self, kind: crate::webhook::Kind, title: &str, body: &str, ping: bool) {
        self.hook_with(kind, title, body, ping, false)
    }

    /// `hook`, optionally pinging @here too.
    fn hook_with(
        &self,
        kind: crate::webhook::Kind,
        title: &str,
        body: &str,
        ping: bool,
        here: bool,
    ) {
        let w = &self.settings.webhook;
        if !w.enabled || (kind == crate::webhook::Kind::Info && !w.on_start_stop) {
            return;
        }
        let Some(url) = self.secrets.get(crate::webhook::URL_KEY) else {
            return;
        };
        let shot = (w.screenshot && kind != crate::webhook::Kind::Info)
            .then(|| {
                self.main_hwnd()
                    .and_then(|c| self.platform.capture_client(c.hwnd).ok())
            })
            .flatten();
        self.notifier.notify(
            &url,
            &w.ping_user,
            crate::webhook::Message {
                kind,
                title: title.into(),
                body: body.into(),
                screenshot: shot,
                ping,
                fields: vec![],
                here,
            },
        );
    }

    /// Keep the main client exactly over the dock box (left column, every
    /// page), alts stacked behind it. While the app or a client is in front,
    /// the client on show is kept topmost so clicking the app never covers it.
    fn dock_tick(&mut self, ppp: f32) {
        if !self.settings.dock || self.last_dock.elapsed() < Duration::from_millis(250) {
            return;
        }
        self.last_dock = Instant::now();
        let p = self.platform.clone();
        if self.own_hwnd.is_none() {
            self.own_hwnd = p.own_window();
        }
        let (Some(own), Some(main)) = (self.own_hwnd, self.main_hwnd()) else {
            self.docked_ok = false;
            return;
        };
        if self.admin_needed {
            self.docked_ok = false;
            self.dock_note = "can't dock: Roblox runs as administrator (see the red banner)".into();
            return;
        }
        let Some(app) = p.geometry(own) else { return };
        let fg = p.foreground();
        let fg_client = self
            .clients
            .iter()
            .find(|c| Some(c.hwnd) == fg)
            .map(|c| c.hwnd);
        let ours = fg == Some(own) || fg_client.is_some();
        if p.is_minimized(own) {
            self.set_top(None);
            return;
        }
        let Some(rect) = self.dock_rect else { return };
        let mut target = crate::layout::ui_to_screen(
            Point::new(app.client.x, app.client.y),
            rect.min.x,
            rect.min.y,
            ppp,
        );
        target.x += self.settings.dock_dx;
        target.y += self.settings.dock_dy;
        let (cw, ch) = (self.settings.client_w, self.settings.client_h);
        for c in self.clients.clone() {
            let Some(g) = p.geometry(c.hwnd) else {
                continue;
            };
            if (g.client.x, g.client.y, g.client.w, g.client.h) != (target.x, target.y, cw, ch) {
                let _ = p.move_resize(
                    c.hwnd,
                    crate::layout::outer_for_client_at(&g, target, cw, ch),
                );
            }
        }
        // Which client is on show: the one the macro is driving right now
        // (it focuses alts to click them), else the main.
        let show = fg_client.unwrap_or(main.hwnd);
        if ours {
            self.set_top(Some(show));
            for c in &self.clients {
                if c.hwnd != show {
                    let _ = p.place_below(c.hwnd, show);
                }
            }
        } else {
            self.set_top(None);
            if let Some(f) = fg {
                // another app is in front: tuck Roblox behind it
                let _ = p.place_below(main.hwnd, f);
            }
        }
        let now = p.geometry(main.hwnd);
        self.docked_ok = now
            .as_ref()
            .is_some_and(|g| (g.client.x, g.client.y) == (target.x, target.y));
        let note = format!(
            "app client at {},{} · box at {:.0},{:.0} pt × {ppp:.2} · target {},{} · main client at {}",
            app.client.x,
            app.client.y,
            rect.min.x,
            rect.min.y,
            target.x,
            target.y,
            now.map(|g| format!("{},{} ({}×{})", g.client.x, g.client.y, g.client.w, g.client.h))
                .unwrap_or_else(|| "?".into())
        );
        self.dock_note = note;
    }

    fn admin_banner(&mut self, ui: &mut egui::Ui) {
        if !self.admin_needed {
            return;
        }
        egui::Frame::new()
            .fill(Color32::from_rgb(70, 22, 30))
            .stroke(Stroke::new(1.0, BAD))
            .corner_radius(10)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Roblox is running as administrator, this macro isn't - Windows blocks moving, docking, capturing and clicking it.")
                            .color(TEXT),
                    );
                    if ui
                        .add(egui::Button::new(RichText::new("Restart macro as admin").color(Color32::WHITE)).fill(BAD))
                        .clicked()
                    {
                        self.restart_as_admin();
                    }
                });
                ui.label(
                    RichText::new("Or: close Roblox and Roblox Account Manager, open RAM normally (not 'Run as administrator'), launch again.")
                        .size(11.0)
                        .color(MUTED),
                );
            });
        ui.add_space(8.0);
    }

    fn restart_as_admin(&mut self) {
        #[cfg(windows)]
        {
            if crate::platform::win32::relaunch_as_admin() {
                let _ = self.settings.save(&self.settings_path);
                std::process::exit(0);
            }
            self.say("restart as admin was cancelled");
        }
        #[cfg(not(windows))]
        self.say("needs Windows");
    }

    /// Make `hwnd` the one topmost client (None = none of them).
    fn set_top(&mut self, hwnd: Option<crate::geom::Hwnd>) {
        if self.top_hwnd == hwnd {
            if let Some(h) = hwnd {
                // re-assert cheaply: something may have bumped it
                let _ = self.platform.set_topmost(h, true);
            }
            return;
        }
        if let Some(old) = self.top_hwnd.take() {
            let _ = self.platform.set_topmost(old, false);
        }
        if let Some(h) = hwnd {
            let _ = self.platform.set_topmost(h, true);
        }
        self.top_hwnd = hwnd;
    }

    /// Work out usernames of clients we didn't launch (or missed): their
    /// Roblox log has the user id once they're in a game.
    fn lookup_names(&mut self) {
        let todo: Vec<u32> = self
            .clients
            .iter()
            .map(|c| c.pid)
            .filter(|pid| !self.names.contains_key(pid))
            .filter(|pid| {
                self.name_tries
                    .get(pid)
                    .is_none_or(|t| t.elapsed() > Duration::from_secs(10))
            })
            .collect();
        let known: Vec<(String, u64)> = self
            .settings
            .accounts
            .iter()
            .filter(|a| a.user_id > 0)
            .map(|a| (a.username.clone(), a.user_id))
            .collect();
        let use_ram = self.settings.account_source == accounts::AccountSource::Ram;
        for pid in todo {
            self.name_tries.insert(pid, Instant::now());
            let taken: Vec<std::path::PathBuf> = self
                .name_logs
                .iter()
                .filter(|(p, _)| **p != pid)
                .map(|(_, l)| l.clone())
                .collect();
            let p = self.platform.clone();
            let ram = self.ram();
            let tx = self.ram_tx.clone();
            let known = known.clone();
            std::thread::spawn(move || {
                let Some(started) = p.process_started(pid) else {
                    return;
                };
                let Some((log, id)) = crate::robloxlog::user_id_for(started, &taken) else {
                    return;
                };
                let name = known
                    .iter()
                    .find(|(_, i)| *i == id)
                    .map(|(n, _)| n.clone())
                    .or_else(|| {
                        use_ram
                            .then(|| ram.accounts_ids().ok())
                            .flatten()
                            .and_then(|l| l.into_iter().find(|(_, i)| *i == id).map(|(n, _)| n))
                    })
                    .or_else(|| crate::robloxlog::username_online(id));
                if let Some(n) = name {
                    let _ = tx.send(RamMsg::PidLog(pid, n, log));
                }
            });
        }
    }

    /// Client name for the UI: the RAM username when we know it.
    fn client_label(&self, i: usize) -> String {
        let Some(c) = self.clients.get(i) else {
            return client_name(i, &self.settings);
        };
        if let Some(n) = self.names.get(&c.pid) {
            return n.clone();
        }
        let l = client_name(i, &self.settings);
        if l == format!("Client {}", i + 1) {
            format!("Roblox {} (finding name…)", i + 1)
        } else {
            l
        }
    }

    /// Next checklist pattern nobody captured yet (your stage first).
    fn next_missing(&self) -> Option<String> {
        let r = &self.settings.run;
        crate::flows::stage_needs(r.mode, &r.map, &r.act)
            .into_iter()
            .map(|(n, _)| n)
            .chain(
                crate::flows::FLOWS
                    .iter()
                    .flat_map(|f| f.needs.iter().map(|n| n.name.to_string())),
            )
            .find(|n| self.lib.get(n).is_none())
    }

    /// F6: save the label under the mouse (in any Roblox client) as a pattern.
    fn hover_capture(&mut self) {
        let p = self.platform.clone();
        let Some(cur) = p.cursor() else {
            self.say("hover capture needs Windows");
            return;
        };
        let hit = self.clients.iter().find_map(|c| {
            p.geometry(c.hwnd)
                .filter(|g| g.client.contains(cur))
                .map(|g| (c.clone(), g))
        });
        let Some((client, geo)) = hit else {
            self.say("F6: put the mouse on a label inside a Roblox client first");
            return;
        };
        let frame = match p.capture_client(client.hwnd) {
            Ok(f) => f,
            Err(e) => {
                self.say(format!("F6: capture failed: {e}"));
                return;
            }
        };
        let at = geo.screen_to_client(cur);
        let Some(bx) = crate::vision::box_around(&frame, at) else {
            self.say(format!(
                "F6: no label found at ({}, {}) - point at the middle of the text",
                at.x, at.y
            ));
            return;
        };
        let name = if self.pat_name.trim().is_empty() {
            self.next_missing()
        } else {
            Some(self.pat_name.trim().to_string())
        };
        let Some(name) = name else {
            self.say("F6: checklist complete - type a name on the Patterns page to capture something else");
            return;
        };
        match crate::vision::Pattern::text_from(&frame, &name, bx, self.pat_margin, None)
            .and_then(|pt| self.lib.save(pt))
        {
            Ok(_) => {
                self.pat_name.clear();
                let next = self
                    .next_missing()
                    .map(|n| format!(" - next: {n}"))
                    .unwrap_or_else(|| " - checklist complete".into());
                self.say(format!(
                    "F6: saved '{name}' ({}x{} at {}, {}){next}",
                    bx.w, bx.h, bx.x, bx.y
                ));
            }
            Err(e) => self.say(format!("F6: {e}")),
        }
    }

    fn main_hwnd(&self) -> Option<WinInfo> {
        self.clients.get(self.main).cloned()
    }

    // ---------------- shell ----------------

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            // ◆ brand mark (no font has the glyph: drawn)
            let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
            let c = r.center();
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(c.x, c.y - 9.0),
                    egui::pos2(c.x + 9.0, c.y),
                    egui::pos2(c.x, c.y + 9.0),
                    egui::pos2(c.x - 9.0, c.y),
                ],
                PURPLE,
                Stroke::NONE,
            ));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new("AA-MACRO").size(16.0).strong().color(TEXT));
                ui.label(
                    RichText::new(format!("{APP_BY} · {}", update::version_label()))
                        .size(10.0)
                        .color(MUTED),
                );
            });
            ui.add_space(18.0);
            let start =
                egui::Button::new(RichText::new("▶ START ALL").strong().color(Color32::WHITE))
                    .fill(if self.running { PURPLE_DIM } else { PURPLE })
                    .corner_radius(8)
                    .min_size(Vec2::new(116.0, 32.0));
            if ui
                .add_enabled(!self.running, start)
                .on_hover_text("F2")
                .clicked()
            {
                self.start();
            }
            let pause = egui::Button::new(if self.paused {
                "▶ RESUME"
            } else {
                "⏸ PAUSE"
            })
            .corner_radius(8)
            .min_size(Vec2::new(92.0, 32.0));
            if ui
                .add_enabled(self.running, pause)
                .on_hover_text("F4")
                .clicked()
            {
                self.toggle_pause();
            }
            let stop = egui::Button::new(RichText::new("⏹ STOP").color(if self.running {
                BAD
            } else {
                MUTED
            }))
            .corner_radius(8)
            .min_size(Vec2::new(80.0, 32.0));
            if ui.add(stop).on_hover_text("F3").clicked() {
                self.stop();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // status: dot + word
                let (txt, c) = if self.running && self.paused {
                    ("Paused", WARN)
                } else if self.running {
                    ("Running", GOOD)
                } else {
                    ("Idle", MUTED)
                };
                ui.label(RichText::new(txt).strong().color(c));
                let (r, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 5.0, c);
                if self.running {
                    ui.painter().circle_stroke(
                        r.center(),
                        7.5,
                        Stroke::new(1.5, c.gamma_multiply(0.35)),
                    );
                }
                ui.label(RichText::new("Status:").color(MUTED));
                ui.add_space(10.0);
                let n = self.clients.len();
                chip(
                    ui,
                    &format!(
                        "{n} client{} · {}",
                        if n == 1 { "" } else { "s" },
                        self.settings.run.mode.label()
                    ),
                    TEXT,
                    Some(if n > 0 { GOOD } else { BAD }),
                );
                ui.add_space(6.0);
                // window tools tucked into one menu
                ui.menu_button(RichText::new("Window").color(MUTED), |ui| {
                    if ui.button("Refresh clients").clicked() {
                        self.refresh_clients();
                        ui.close();
                    }
                    if ui
                        .button("Tile clients (F1)")
                        .on_hover_text("every client to an 800×599 viewport, laid out on screen")
                        .clicked()
                    {
                        self.tile();
                        ui.close();
                    }
                    if ui
                        .button("Doctor")
                        .on_hover_text("check scaling, size and capture of every client")
                        .clicked()
                    {
                        self.doctor();
                        ui.close();
                    }
                    ui.separator();
                    if self.settings.dock {
                        if ui.button("Undock Roblox").clicked() {
                            self.settings.dock = false;
                            self.docked_ok = false;
                            self.set_top(None);
                            self.dirty();
                            self.tile();
                            ui.close();
                        }
                    } else if ui
                        .button("Dock Roblox into the app")
                        .on_hover_text(
                            "the main client sits in the left column, in front of the app",
                        )
                        .clicked()
                    {
                        self.settings.dock = true;
                        self.dirty();
                        ui.close();
                    }
                });
            });
        });
    }

    /// Left sidebar: the Command Center pages, then the setup pages.
    fn rail(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        let mut item = |ui: &mut egui::Ui, p: Page, name: &str| {
            let selected = self.page == p;
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
            if selected {
                ui.painter().rect_filled(rect, 8.0, PURPLE_DIM);
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())),
                    2.0,
                    PURPLE,
                );
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, 8.0, CARD_HI);
            }
            ui.painter().text(
                egui::pos2(rect.left() + 12.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                name,
                egui::FontId::proportional(13.5),
                if selected { Color32::WHITE } else { MUTED },
            );
            if resp.clicked() {
                self.page = p;
            }
            ui.add_space(2.0);
        };
        for (p, name) in command::NAV {
            item(ui, p, name);
        }
        ui.add_space(14.0);
        ui.label(RichText::new("  SETUP").size(10.0).color(MUTED));
        ui.add_space(4.0);
        for (p, name) in command::NAV_SETUP {
            item(ui, p, name);
        }
    }

    // ---------------- Home ----------------

    fn page_home(&mut self, ui: &mut egui::Ui) {
        if self.settings.dock {
            // Roblox + session live in the dock column; this side is controls.
            self.home_controls(ui);
            return;
        }
        let avail = ui.available_width();
        let left_w = (avail * 0.58).max(420.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(left_w, 0.0),
                Layout::top_down(Align::Min),
                |ui| {
                    self.home_preview(ui);
                    ui.add_space(8.0);
                    self.session_row(ui);
                },
            );
            ui.add_space(8.0);
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), 0.0),
                Layout::top_down(Align::Min),
                |ui| self.home_controls(ui),
            );
        });
    }

    fn session_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_top(|ui| {
            let w = (ui.available_width() - 8.0) / 2.0;
            ui.allocate_ui_with_layout(Vec2::new(w, 0.0), Layout::top_down(Align::Min), |ui| {
                self.session_card(ui)
            });
            ui.allocate_ui_with_layout(Vec2::new(w, 0.0), Layout::top_down(Align::Min), |ui| {
                self.quick_card(ui)
            });
        });
    }

    fn home_controls(&mut self, ui: &mut egui::Ui) {
        self.mode_card(ui);
        ui.add_space(8.0);
        if self.settings.run.own_team {
            self.alts_card(ui);
            ui.add_space(8.0);
        }
        segmented(
            ui,
            &mut self.home_tab,
            &[
                (HomeTab::Control, "Control"),
                (HomeTab::Schedule, "Schedule"),
                (HomeTab::Sequence, "Sequence check"),
                (HomeTab::Activity, "Activity"),
            ],
        );
        ui.add_space(6.0);
        match self.home_tab {
            HomeTab::Control => self.units_grid(ui),
            HomeTab::Schedule => self.schedule_card(ui),
            HomeTab::Sequence => self.sequence_card(ui),
            HomeTab::Activity => self.activity(ui),
        }
    }

    /// Farm list: modes one after another, each for some runs / minutes.
    fn schedule_card(&mut self, ui: &mut egui::Ui) {
        use crate::config::{Mode, PlayChoice, ScheduleStep};
        let mut changed = false;
        let teams: Vec<String> = self.settings.teams.iter().map(|t| t.name.clone()).collect();
        let now = ScheduleStep::from_run(&self.settings);
        card(ui, |ui| {
            let sc = &mut self.settings.schedule;
            ui.horizontal(|ui| {
                ui.label(RichText::new("SCHEDULE").size(11.0).color(MUTED));
                changed |= toggle(ui, &mut sc.enabled, "Use the schedule");
                changed |= toggle(ui, &mut sc.repeat, "Repeat");
            });
            ui.label(
                RichText::new(
                    "Start runs the steps in order. A step ends after its runs or minutes (whichever first, the current run is finished), then it goes back to the lobby and starts the next.",
                )
                .size(11.0)
                .color(MUTED),
            );
            let mut remove = None;
            let mut swap = None;
            let len = sc.steps.len();
            for (i, st) in sc.steps.iter_mut().enumerate() {
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("{}.", i + 1)).strong());
                    changed |= ui.checkbox(&mut st.enabled, "").changed();
                    ComboBox::from_id_salt(("sched_mode", i))
                        .selected_text(st.mode.label())
                        .show_ui(ui, |ui| {
                            for m in Mode::ALL {
                                changed |=
                                    ui.selectable_value(&mut st.mode, m, m.label()).changed();
                            }
                        });
                    if matches!(st.mode, Mode::Story | Mode::Legend | Mode::Raid) {
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut st.map)
                                    .hint_text("map")
                                    .desired_width(110.0),
                            )
                            .changed();
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut st.act)
                                    .hint_text("act")
                                    .desired_width(60.0),
                            )
                            .changed();
                    }
                    if st.mode == Mode::SummerHunt {
                        changed |= segmented(
                            ui,
                            &mut st.play,
                            &[
                                (PlayChoice::PlayHere, "Play here"),
                                (PlayChoice::FindMatch, "Find match"),
                            ],
                        );
                    }
                    if st.mode == Mode::InfinityCastle {
                        changed |= ui.checkbox(&mut st.castle_hard, "Hard").changed();
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Team");
                    let shown = st
                        .team
                        .and_then(|t| teams.get(t).cloned())
                        .unwrap_or_else(|| "mode's team".into());
                    ComboBox::from_id_salt(("sched_team", i))
                        .selected_text(shown)
                        .show_ui(ui, |ui| {
                            changed |= ui
                                .selectable_value(&mut st.team, None, "mode's team")
                                .changed();
                            for (t, name) in teams.iter().enumerate() {
                                changed |=
                                    ui.selectable_value(&mut st.team, Some(t), name).changed();
                            }
                        });
                    ui.label("Runs");
                    changed |= ui
                        .add(egui::DragValue::new(&mut st.runs).range(0..=9999))
                        .changed();
                    ui.label("Minutes");
                    changed |= ui
                        .add(egui::DragValue::new(&mut st.minutes).range(0..=1440))
                        .changed();
                    ui.label(RichText::new("(0 = no limit)").size(11.0).color(MUTED));
                    if ui
                        .add_enabled(i > 0, egui::Button::new("Up").small())
                        .clicked()
                    {
                        swap = Some((i, i - 1));
                    }
                    if ui
                        .add_enabled(i + 1 < len, egui::Button::new("Down").small())
                        .clicked()
                    {
                        swap = Some((i, i + 1));
                    }
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some((a, b)) = swap {
                sc.steps.swap(a, b);
                changed = true;
            }
            if let Some(i) = remove {
                sc.steps.remove(i);
                changed = true;
            }
            ui.separator();
            if ui.button("+ Add step (what's set up now)").clicked() {
                sc.steps.push(now.clone());
                changed = true;
            }
        });
        if changed {
            self.dirty();
        }
    }

    /// Every step the game loop will do for the chosen mode, and whether the
    /// patterns it looks for are captured.
    fn sequence_card(&mut self, ui: &mut egui::Ui) {
        let steps = crate::game::run::plan(&self.settings, &self.routes);
        card(ui, |ui| {
            ui.label(
                RichText::new(format!(
                    "{} · what Start will do, in order",
                    self.settings.run.mode.label()
                ))
                .size(11.0)
                .color(MUTED),
            );
            let mut blocking = 0;
            for (i, st) in steps.iter().enumerate() {
                let missing: Vec<&String> = st
                    .needs
                    .iter()
                    .filter(|n| self.lib.get(n).is_none())
                    .collect();
                let c = if missing.is_empty() {
                    GOOD
                } else if st.optional {
                    WARN
                } else {
                    blocking += 1;
                    BAD
                };
                ui.horizontal_wrapped(|ui| {
                    dot(ui, c);
                    ui.label(RichText::new(format!("{}. {}", i + 1, st.label)).color(TEXT));
                    if !missing.is_empty() {
                        let names: Vec<&str> = missing.iter().map(|s| s.as_str()).collect();
                        ui.label(
                            RichText::new(format!(
                                "{} {}",
                                if st.optional { "optional:" } else { "missing:" },
                                names.join(", ")
                            ))
                            .size(11.0)
                            .color(c),
                        );
                    }
                });
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new(if blocking == 0 {
                    "Everything the loop needs is captured - Start will run this.".to_string()
                } else {
                    format!("{blocking} step(s) still need patterns - F6 over each label in game (Patterns page shows the list).")
                })
                .size(11.0)
                .color(if blocking == 0 { GOOD } else { WARN }),
            );
        });
    }

    /// Left column when docked: the box Roblox sits on, then the session.
    fn dock_column(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("MAIN").size(11.0).color(MUTED));
            let labels: Vec<String> = (0..self.clients.len())
                .map(|i| self.client_label(i))
                .collect();
            if !labels.is_empty() {
                let cur = labels[self.main.min(labels.len() - 1)].clone();
                ComboBox::from_id_salt("mainsel")
                    .selected_text(cur)
                    .show_ui(ui, |ui| {
                        for (i, l) in labels.iter().enumerate() {
                            if ui.selectable_value(&mut self.main, i, l).changed() {
                                self.sel = i;
                            }
                        }
                    });
            }
            ui.label(
                RichText::new(if self.docked_ok {
                    "docked"
                } else {
                    "docking…"
                })
                .size(11.0)
                .color(if self.docked_ok { GOOD } else { WARN }),
            );
        });
        let ppp = ui.ctx().pixels_per_point();
        let size = Vec2::new(
            self.settings.client_w as f32 / ppp,
            self.settings.client_h as f32 / ppp,
        );
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        ui.painter().rect_filled(rect, 0.0, BG);
        let msg = if self.clients.is_empty() {
            "No Roblox client - open Roblox (windowed) or launch from Accounts"
        } else {
            "Roblox sits here"
        };
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            msg,
            egui::FontId::proportional(14.0),
            MUTED,
        );
        self.dock_rect = Some(rect);
        ui.add_space(8.0);
        self.session_row(ui);
    }

    fn home_preview(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("MAIN").size(11.0).color(MUTED));
                let labels: Vec<String> = (0..self.clients.len())
                    .map(|i| self.client_label(i))
                    .collect();
                if !labels.is_empty() {
                    let cur = labels[self.main.min(labels.len() - 1)].clone();
                    ComboBox::from_id_salt("mainsel")
                        .selected_text(cur)
                        .show_ui(ui, |ui| {
                            for (i, l) in labels.iter().enumerate() {
                                if ui.selectable_value(&mut self.main, i, l).changed() {
                                    self.sel = i;
                                }
                            }
                        });
                }
                toggle(ui, &mut self.live, "live");
            });
            self.sel = self.main;
            if let Some(tex) = &self.tex {
                let size = tex.size_vec2();
                let scale = (ui.available_width() / size.x).min(1.0);
                let (rect, _) = ui.allocate_exact_size(size * scale, Sense::hover());
                ui.painter().image(
                    tex.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), ui.available_width() * 0.75),
                    Sense::hover(),
                );
                ui.painter().rect_filled(rect, 8.0, BG);
                let msg = if self.clients.is_empty() {
                    "No Roblox client - open Roblox (windowed) and press Refresh"
                } else {
                    "Waiting for a picture…"
                };
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    msg,
                    egui::FontId::proportional(14.0),
                    MUTED,
                );
            }
        });
    }

    fn session_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("SESSION").size(11.0).color(MUTED));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("Reset").clicked() {
                        self.wins = 0;
                        self.losses = 0;
                        self.drops.clear();
                        self.started = if self.running {
                            Some(Instant::now())
                        } else {
                            None
                        };
                    }
                });
            });
            let runs = self.wins + self.losses;
            let rate = if runs > 0 { self.wins * 100 / runs } else { 0 };
            let t = self.started.map(|s| s.elapsed().as_secs()).unwrap_or(0);
            ui.horizontal(|ui| {
                stat(ui, &self.wins.to_string(), "Wins", GOOD);
                stat(ui, &self.losses.to_string(), "Losses", BAD);
                stat(ui, &format!("{rate}%"), "Win rate", TEXT);
                stat(ui, &runs.to_string(), "Runs", TEXT);
                stat(
                    ui,
                    &format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60),
                    "Runtime",
                    TEXT,
                );
            });
            if !self.drops.is_empty() {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Drops").size(11.0).color(MUTED));
                    for (n, a) in &self.drops {
                        ui.label(RichText::new(format!("{a} {n}")).color(TEXT));
                        ui.label(RichText::new("·").color(MUTED));
                    }
                });
            }
        });
    }

    fn quick_card(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        card(ui, |ui| {
            ui.label(RichText::new("QUICK").size(11.0).color(MUTED));
            let r = &mut self.settings.run;
            changed |= toggle(ui, &mut r.priority_upgrade, "Priority upgrade");
            changed |= toggle(ui, &mut r.one_at_a_time, "One unit at a time");
            changed |= toggle(ui, &mut r.auto_ability, "Auto ability");
            changed |= toggle(ui, &mut r.challenge_every_30, "Challenge every 30 min");
        });
        if changed {
            self.dirty();
        }
    }

    fn mode_card(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        let mode_before = self.settings.run.mode;
        card(ui, |ui| {
            let r = &mut self.settings.run;
            ui.horizontal(|ui| {
                ui.label(RichText::new("Mode").color(MUTED));
                ComboBox::from_id_salt("mode")
                    .width(ui.available_width() - 8.0)
                    .selected_text(r.mode.label())
                    .show_ui(ui, |ui| {
                        for m in Mode::ALL {
                            changed |= ui.selectable_value(&mut r.mode, m, m.label()).changed();
                        }
                    });
            });
            ui.add_space(6.0);
            if matches!(r.mode, Mode::SummerHunt | Mode::Challenge) {
                ui.horizontal(|ui| {
                    changed |= segmented(
                        ui,
                        &mut r.play,
                        &[
                            (PlayChoice::PlayHere, "Play Here"),
                            (PlayChoice::FindMatch, "Find Match"),
                        ],
                    );
                    changed |= toggle(ui, &mut r.own_team, "Own team (alts)");
                });
                if r.play == PlayChoice::FindMatch {
                    ui.label(
                        RichText::new(
                            "Find Match shows a captcha: type it yourself, the macro waits.",
                        )
                        .size(11.0)
                        .color(WARN),
                    );
                }
            }
            if r.mode == Mode::InfinityCastle {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Difficulty").color(MUTED));
                    changed |=
                        segmented(ui, &mut r.castle_hard, &[(false, "Normal"), (true, "Hard")]);
                });
            }
            if r.mode == Mode::Dungeon {
                ui.label(
                    RichText::new("Dungeon isn't supported yet.")
                        .size(11.0)
                        .color(WARN),
                );
            }
            if let Some(key) = stage_key(r.mode) {
                if let Some(ms) = self.stages.modes.get(key) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Map").color(MUTED));
                        ComboBox::from_id_salt("stage-map")
                            .width(170.0)
                            .selected_text(r.map.clone())
                            .show_ui(ui, |ui| {
                                for m in &ms.maps {
                                    changed |= ui
                                        .selectable_value(&mut r.map, m.name.clone(), &m.name)
                                        .changed();
                                }
                            });
                        ui.label(RichText::new("Act").color(MUTED));
                        ComboBox::from_id_salt("stage-act")
                            .width(90.0)
                            .selected_text(r.act.clone())
                            .show_ui(ui, |ui| {
                                for a in &ms.acts {
                                    changed |= ui
                                        .selectable_value(&mut r.act, a.name.clone(), &a.name)
                                        .changed();
                                }
                            });
                    });
                    // Switching modes: keep map/act valid for the new list.
                    if !ms.maps.iter().any(|m| m.name == r.map) {
                        if let Some(m) = ms.maps.first() {
                            r.map = m.name.clone();
                            changed = true;
                        }
                    }
                    if !ms.acts.iter().any(|a| a.name == r.act) {
                        if let Some(a) = ms
                            .acts
                            .iter()
                            .find(|a| a.name != "Infinite")
                            .or(ms.acts.first())
                        {
                            r.act = a.name.clone();
                            changed = true;
                        }
                    }
                    if r.mode == Mode::Story {
                        changed |= toggle(ui, &mut r.next_level, "Next level after a win");
                    }
                }
                changed |= toggle(ui, &mut r.own_team, "Own team (alts)");
            }
            ui.horizontal(|ui| {
                changed |= toggle(ui, &mut r.return_to_lobby, "Return to lobby");
                changed |= toggle(ui, &mut r.stop_after_defeat, "Stop after a defeat");
            });
        });
        if self.settings.run.mode != mode_before {
            let before = self.settings.team;
            self.settings.team_for_mode(self.settings.run.mode);
            if self.settings.team != before {
                let n = self.settings.teams[self.settings.team].name.clone();
                self.say(format!(
                    "team '{n}' loaded for {}",
                    self.settings.run.mode.label()
                ));
            }
        }
        if changed {
            self.dirty();
        }
    }

    fn alts_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("YOUR ALTS").size(11.0).color(MUTED));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("Refresh").clicked() {
                        self.refresh_clients();
                        self.last_alt_cap = Instant::now() - Duration::from_secs(10);
                    }
                });
            });
            let alts: Vec<usize> = (0..self.clients.len())
                .filter(|i| *i != self.main)
                .collect();
            if alts.is_empty() {
                ui.label(
                    RichText::new(
                        "No alts open. Launch them from Accounts, or open more Roblox clients.",
                    )
                    .color(MUTED),
                );
                return;
            }
            let mut make_main = None;
            ui.horizontal_wrapped(|ui| {
                for i in &alts {
                    ui.vertical(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(148.0, 111.0), Sense::hover());
                        match self.alt_tex.get(*i).and_then(|t| t.as_ref()) {
                            Some(t) => {
                                ui.painter().image(
                                    t.id(),
                                    rect,
                                    egui::Rect::from_min_max(
                                        egui::pos2(0.0, 0.0),
                                        egui::pos2(1.0, 1.0),
                                    ),
                                    Color32::WHITE,
                                );
                            }
                            None => {
                                ui.painter().rect_filled(rect, 6.0, BG);
                            }
                        }
                        ui.label(
                            RichText::new(format!("{} · 800×599", self.client_label(*i)))
                                .size(11.0)
                                .color(TEXT),
                        );
                        if ui.small_button("Make main").clicked() {
                            make_main = Some(*i);
                        }
                    });
                }
            });
            if let Some(i) = make_main {
                self.main = i;
                self.sel = i;
                self.say(format!("{} is now the main", self.client_label(i)));
            }
            ui.label(
                RichText::new(format!(
                    "{} alt(s). Alts join the main's party / portal and press Play Here / Replay before the main; only the main places units.",
                    alts.len()
                ))
                .size(11.0)
                .color(MUTED),
            );
        });
    }

    /// Team loadouts: pick, rename, add, delete, link to the current mode.
    fn team_bar(&mut self, ui: &mut egui::Ui) {
        self.settings.fix_teams();
        let mut switch = None;
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("TEAM").size(11.0).color(MUTED));
            let cur = self.settings.team;
            ComboBox::from_id_salt("team")
                .selected_text(self.settings.teams[cur].name.clone())
                .show_ui(ui, |ui| {
                    for (i, t) in self.settings.teams.iter().enumerate() {
                        if ui.selectable_label(i == cur, &t.name).clicked() {
                            switch = Some(i);
                        }
                    }
                });
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut self.settings.teams[cur].name)
                        .desired_width(120.0)
                        .hint_text("team name"),
                )
                .on_hover_text("rename this team")
                .changed();
            if ui
                .small_button("+ New")
                .on_hover_text("copy of this team")
                .clicked()
            {
                let n = format!("Team {}", self.settings.teams.len() + 1);
                self.settings.add_team(&n);
                changed = true;
            }
            if self.settings.teams.len() > 1 && ui.small_button("Delete").clicked() {
                self.settings.remove_team(cur);
                changed = true;
            }
            let mode = self.settings.run.mode.label().to_string();
            let mut linked = self.settings.mode_teams.get(&mode) == Some(&self.settings.team);
            if toggle(ui, &mut linked, &format!("use for {mode}")) {
                if linked {
                    self.settings.mode_teams.insert(mode, self.settings.team);
                } else {
                    self.settings.mode_teams.remove(&mode);
                }
                changed = true;
            }
        });
        if let Some(i) = switch {
            self.settings.switch_team(i);
            changed = true;
        }
        if changed {
            self.dirty();
        }
        ui.add_space(4.0);
    }

    fn units_grid(&mut self, ui: &mut egui::Ui) {
        self.team_bar(ui);
        if self.clients.len() > 1 {
            egui::CollapsingHeader::new(
                RichText::new(format!(
                    "Placing clients ({}) - 2 accounts with 2 teams",
                    self.settings.placers.len()
                ))
                .color(MUTED),
            )
            .id_salt("placers-home")
            .show(ui, |ui| self.placers_ui(ui));
        }
        let mut changed = false;
        let keys = self.settings.keybinds.units.clone();
        // one compact row per hotbar slot
        card(ui, |ui| {
            egui::Grid::new("units_table")
                .num_columns(8)
                .spacing(Vec2::new(10.0, 8.0))
                .min_row_height(26.0)
                .show(ui, |ui| {
                    for h in ["", "Key", "Unit", "Role", "Place", "Prio", "Upgrade", "Now"] {
                        ui.label(RichText::new(h).size(10.5).color(MUTED));
                    }
                    ui.end_row();
                    for i in 0..6usize {
                        let key = keys.get(i).cloned().unwrap_or_default();
                        let u = &mut self.settings.units[i];
                        changed |= toggle(ui, &mut u.enabled, "");
                        ui.label(
                            RichText::new(key.to_uppercase())
                                .strong()
                                .color(if u.enabled { PURPLE } else { MUTED }),
                        )
                        .on_hover_text(format!("hotbar slot {} (key {key})", i + 1));
                        changed |= ui
                            .add_sized(
                                [104.0, 24.0],
                                egui::TextEdit::singleline(&mut u.name)
                                    .hint_text(format!("Unit {}", i + 1)),
                            )
                            .changed();
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            changed |= tag_tip(ui, &mut u.farm, "F", "Farm: placed first");
                            changed |= tag_tip(ui, &mut u.buff, "B", "Buff: turns Auto ability on");
                            changed |= tag_tip(
                                ui,
                                &mut u.hill,
                                "H",
                                "Hill: high ground first, ground if there's none",
                            );
                        });
                        changed |= ui
                            .add(egui::DragValue::new(&mut u.place).range(0..=10).speed(0.1))
                            .on_hover_text("how many to place")
                            .changed();
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut u.priority)
                                    .range(1..=6)
                                    .speed(0.1),
                            )
                            .on_hover_text("upgrade order (1 first)")
                            .changed();
                        let label = |u: Upgrade| match u {
                            Upgrade::None => "none".to_string(),
                            Upgrade::Max => "MAX".to_string(),
                            Upgrade::Level(n) => format!("{n}x"),
                        };
                        ComboBox::from_id_salt(("upg", i))
                            .width(62.0)
                            .selected_text(label(u.upgrade))
                            .show_ui(ui, |ui| {
                                let mut opts = vec![Upgrade::None];
                                opts.extend((1..=8).map(Upgrade::Level));
                                opts.push(Upgrade::Max);
                                for o in opts {
                                    changed |=
                                        ui.selectable_value(&mut u.upgrade, o, label(o)).changed();
                                }
                            });
                        changed |= ui
                            .checkbox(&mut u.upg_immediately, "")
                            .on_hover_text("upgrade right after placing")
                            .changed();
                        ui.end_row();
                    }
                });
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "Hotbar keys {} · upgrade {}",
                keys.join(" "),
                self.settings.keybinds.upgrade.to_uppercase()
            ))
            .size(11.0)
            .color(MUTED),
        );
        if changed {
            self.dirty();
        }
    }

    fn activity(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(RichText::new("ACTIVITY").size(11.0).color(MUTED));
            egui::ScrollArea::vertical()
                .id_salt("activity")
                .max_height(360.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for l in &self.log {
                        ui.label(RichText::new(l).monospace().size(12.0).color(TEXT));
                    }
                });
        });
    }

    // ---------------- Accounts ----------------

    fn ram(&self) -> Ram {
        Ram::new(
            self.settings.ram_port,
            &self.secrets.get(accounts::PASSWORD_KEY).unwrap_or_default(),
        )
    }

    fn page_accounts(&mut self, ui: &mut egui::Ui) {
        heading(
            ui,
            "Accounts",
            "launch your main + alts into your private server - built in, no other app needed",
        );
        ui.horizontal(|ui| {
            ui.label(RichText::new("Launch accounts with").color(MUTED));
            if segmented(
                ui,
                &mut self.settings.account_source,
                &[
                    (accounts::AccountSource::BuiltIn, "Built-in (this app)"),
                    (accounts::AccountSource::Ram, "Roblox Account Manager"),
                ],
            ) {
                self.dirty();
            }
        });
        ui.add_space(6.0);
        if self.settings.account_source == accounts::AccountSource::BuiltIn {
            self.builtin_card(ui);
        } else {
            card(ui, |ui| {
                ui.label(
                    RichText::new("ROBLOX ACCOUNT MANAGER")
                        .size(11.0)
                        .color(MUTED),
                );
                ui.horizontal(|ui| {
                    let (c, t) = match self.ram_ok {
                        Some(true) => (GOOD, "connected"),
                        Some(false) => (BAD, "not connected"),
                        None => (MUTED, "not checked"),
                    };
                    dot(ui, c);
                    ui.colored_label(c, t);
                    ui.label(RichText::new(&self.ram_state).color(MUTED).size(12.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Port");
                    if ui
                        .add(egui::DragValue::new(&mut self.settings.ram_port).range(1..=65535))
                        .changed()
                    {
                        self.dirty();
                    }
                    ui.label("Password");
                    let has = self.secrets.has(accounts::PASSWORD_KEY);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.ram_pw_input)
                            .password(true)
                            .desired_width(150.0)
                            .hint_text(if has {
                                "saved (encrypted)"
                            } else {
                                "6+ characters"
                            }),
                    );
                    if ui.button("Save").clicked() {
                        let pw = self.ram_pw_input.clone();
                        match self.secrets.set(accounts::PASSWORD_KEY, &pw) {
                            Ok(()) => {
                                self.ram_pw_input.clear();
                                self.say("RAM password saved (encrypted)");
                            }
                            Err(e) => self.say(format!("saving password failed: {e}")),
                        }
                    }
                    if ui.button("Test & load accounts").clicked() {
                        self.ram_load();
                    }
                });
                ui.label(
                RichText::new("In RAM: Settings > Developer > Enable Web Server · Web Server > Allow LaunchAccount + Allow GetAccounts + a password · General > Multi Roblox. Cookies stay inside RAM; the macro only asks it to launch.")
                    .size(11.0)
                    .color(MUTED),
            );
            });
        }
        ui.add_space(8.0);
        card(ui, |ui| {
            ui.label(RichText::new("PRIVATE SERVER").size(11.0).color(MUTED));
            ui.horizontal(|ui| {
                let has = self.secrets.has(accounts::PRIVATE_SERVER_KEY);
                ui.add(
                    egui::TextEdit::singleline(&mut self.ps_input)
                        .desired_width(420.0)
                        .hint_text(if has { "saved (encrypted)" } else { "https://www.roblox.com/share?code=…&type=Server" }),
                );
                if ui.button("Save").clicked() {
                    let link = self.ps_input.trim().to_string();
                    if !link.is_empty() && !(link.starts_with("https://www.roblox.com/") || link.starts_with("roblox://")) {
                        self.say("private server link must start with https://www.roblox.com/ or roblox://");
                    } else if self.secrets.set(accounts::PRIVATE_SERVER_KEY, &link).is_ok() {
                        self.ps_input.clear();
                        self.say(if link.is_empty() { "private server link removed" } else { "private server link saved (encrypted)" });
                    }
                }
            });
        });
        ui.add_space(8.0);
        let mut changed = false;
        let mut launch: Option<Vec<String>> = None;
        let mut follow: Option<String> = None;
        let mut check: Option<String> = None;
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("ACCOUNTS").size(11.0).color(MUTED));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let order: Vec<String> = accounts::launch_order(&self.settings.accounts)
                        .iter()
                        .map(|a| a.username.clone())
                        .collect();
                    if ui
                        .add_enabled(
                            !order.is_empty(),
                            egui::Button::new(RichText::new("Launch all").color(Color32::WHITE))
                                .fill(PURPLE),
                        )
                        .clicked()
                    {
                        launch = Some(order);
                    }
                    if self.settings.account_source == accounts::AccountSource::Ram
                        && ui.button("+ Add").clicked()
                    {
                        let role = if self
                            .settings
                            .accounts
                            .iter()
                            .any(|a| a.role == AccountRole::Main)
                        {
                            AccountRole::Alt
                        } else {
                            AccountRole::Main
                        };
                        self.settings.accounts.push(AccountSlot {
                            username: String::new(),
                            role,
                            enabled: true,
                            user_id: 0,
                        });
                        changed = true;
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Alts join").color(MUTED));
                changed |= segmented(
                    ui,
                    &mut self.settings.alt_join,
                    &[
                        (accounts::AltJoin::PrivateServer, "Private server link"),
                        (accounts::AltJoin::FollowMain, "Follow the main"),
                    ],
                );
                if self.settings.alt_join == accounts::AltJoin::FollowMain {
                    ui.label("after");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.settings.follow_delay_s)
                                .range(5..=180)
                                .suffix(" s"),
                        )
                        .changed();
                }
            });
            if self.settings.alt_join == accounts::AltJoin::FollowMain {
                ui.label(
                    RichText::new("The main launches first (private server link if set), then each alt joins the main's server through RAM's Follow. In RAM tick Web Server > Allow LaunchAccount. In Roblox the main's privacy 'Who can join me' must allow the alts (friends), and alts must be allowed into that private server.")
                        .size(11.0)
                        .color(MUTED),
                );
            }
            let names = self.ram_list.clone();
            let builtin = self.settings.account_source == accounts::AccountSource::BuiltIn;
            let saved: Vec<bool> = self
                .settings
                .accounts
                .iter()
                .map(|a| self.secrets.has(&crate::rbx::cookie_key(&a.username)))
                .collect();
            let mut remove = None;
            egui::Grid::new("accounts")
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    for h in ["On", "Account", "Role", ""] {
                        ui.label(RichText::new(h).color(MUTED));
                    }
                    ui.end_row();
                    for (i, a) in self.settings.accounts.iter_mut().enumerate() {
                        changed |= ui.checkbox(&mut a.enabled, "").changed();
                        if builtin {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&a.username).color(TEXT));
                                if saved.get(i).copied().unwrap_or(false) {
                                    ui.label(RichText::new("login saved").size(11.0).color(GOOD));
                                    if ui.small_button("Check").on_hover_text("asks Roblox if this login still works (doesn't start Roblox)").clicked() {
                                        check = Some(a.username.clone());
                                    }
                                } else {
                                    ui.label(RichText::new("no login - add it above").size(11.0).color(BAD));
                                }
                            });
                        } else {
                        ComboBox::from_id_salt(("acc", i))
                            .width(180.0)
                            .selected_text(if a.username.is_empty() {
                                "pick…".to_string()
                            } else {
                                a.username.clone()
                            })
                            .show_ui(ui, |ui| {
                                for n in &names {
                                    changed |= ui
                                        .selectable_value(&mut a.username, n.clone(), n)
                                        .changed();
                                }
                                if names.is_empty() {
                                    ui.label("press Test & load accounts first");
                                }
                            });
                        }
                        ComboBox::from_id_salt(("accrole", i))
                            .width(80.0)
                            .selected_text(if a.role == AccountRole::Main {
                                "Main"
                            } else {
                                "Alt"
                            })
                            .show_ui(ui, |ui| {
                                changed |= ui
                                    .selectable_value(&mut a.role, AccountRole::Main, "Main")
                                    .changed();
                                changed |= ui
                                    .selectable_value(&mut a.role, AccountRole::Alt, "Alt")
                                    .changed();
                            });
                        ui.horizontal(|ui| {
                            if ui.small_button("Launch").clicked() && !a.username.is_empty() {
                                launch = Some(vec![a.username.clone()]);
                            }
                            if a.role == AccountRole::Alt
                                && !a.username.is_empty()
                                && ui
                                    .small_button("Join main")
                                    .on_hover_text("open this alt straight into the main's current server (RAM Follow)")
                                    .clicked()
                            {
                                follow = Some(a.username.clone());
                            }
                            if ui.small_button("Remove").clicked() {
                                remove = Some(i);
                            }
                        });
                        ui.end_row();
                    }
                });
            if let Some(i) = remove {
                let gone = self.settings.accounts.remove(i);
                let _ = self
                    .secrets
                    .set(&crate::rbx::cookie_key(&gone.username), "");
                changed = true;
            }
            if self.settings.accounts.is_empty() {
                ui.label(
                    RichText::new("No accounts yet - press + Add (first one becomes the main).")
                        .color(MUTED),
                );
            }
        });
        ui.add_space(8.0);
        card(ui, |ui| {
            ui.label(RichText::new("PARTY").size(11.0).color(MUTED));
            changed |= toggle(
                ui,
                &mut self.settings.run.party_invite,
                "Main invites alts into its party",
            );
            ui.label(
                RichText::new("Flow: everyone joins the private server > main clicks the Party + (top right) > Invite on each alt > each alt presses Accept. Needs the party patterns from the Patterns page checklist.")
                    .size(11.0)
                    .color(MUTED),
            );
        });
        if changed {
            self.dirty();
        }
        if let Some(list) = launch {
            self.ram_launch(list, false);
        }
        if let Some(alt) = follow {
            self.ram_launch(vec![alt], true);
        }
        if let Some(n) = check {
            match self.secrets.get(&crate::rbx::cookie_key(&n)) {
                Some(c) => {
                    let tx = self.ram_tx.clone();
                    self.say(format!("{n}: checking the login…"));
                    std::thread::spawn(move || {
                        let msg = match crate::rbx::check(&c) {
                            Ok(m) => m,
                            Err(e) => format!("{n}: login check failed - {e:#}"),
                        };
                        let _ = tx.send(RamMsg::Log(msg));
                    });
                }
                None => self.say(format!("{n}: no saved login")),
            }
        }
    }

    /// Built-in account manager: add a login (cookie), or move accounts
    /// over from Roblox Account Manager once.
    fn builtin_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(
                RichText::new("BUILT-IN ACCOUNT MANAGER")
                    .size(11.0)
                    .color(MUTED),
            );
            ui.label(
                RichText::new("Add each account's login once. It's saved encrypted on this PC (Windows DPAPI - only your Windows user can read it) and only ever sent to roblox.com to start Roblox. Never share it: it logs straight into the account.")
                    .size(11.0)
                    .color(MUTED),
            );
            ui.horizontal(|ui| {
                ui.label("Login cookie");
                ui.add(
                    egui::TextEdit::singleline(&mut self.cookie_input)
                        .password(true)
                        .desired_width(320.0)
                        .hint_text(".ROBLOSECURITY value (_|WARNING:-DO-NOT-SHARE…)"),
                );
                if ui.button("Add account").clicked() {
                    let c = crate::rbx::clean_cookie(&self.cookie_input);
                    self.cookie_input.clear();
                    if c.len() < 50 {
                        self.say("that doesn't look like a login cookie");
                    } else {
                        let tx = self.ram_tx.clone();
                        self.say("checking the login with Roblox…");
                        std::thread::spawn(move || match crate::rbx::whoami(&c) {
                            Ok((id, name)) => {
                                let _ = tx.send(RamMsg::Added(name, id, c));
                            }
                            Err(e) => {
                                let _ = tx.send(RamMsg::Log(format!("add account: {e}")));
                            }
                        });
                    }
                }
            });
            ui.label(
                RichText::new("Where to get it: open a private / incognito browser window, log into the account at roblox.com, press F12 > Application > Cookies > https://www.roblox.com > .ROBLOSECURITY > copy the Value. Then just close the window - don't press Log out, that ends the saved login too.")
                    .size(11.0)
                    .color(MUTED),
            );
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button("Import all from Roblox Account Manager")
                    .on_hover_text("one time: RAM open, Web Server on, Allow GetAccounts + Allow GetCookie ticked, password saved in the RAM mode of this page")
                    .clicked()
                {
                    self.import_from_ram();
                }
                ui.label(
                    RichText::new("one time - then untick Allow GetCookie in RAM and you can close RAM for good")
                        .size(11.0)
                        .color(MUTED),
                );
            });
        });
    }

    fn import_from_ram(&mut self) {
        let ram = self.ram();
        let tx = self.ram_tx.clone();
        self.say("importing accounts from Roblox Account Manager…");
        std::thread::spawn(move || {
            let log = |s: String| {
                let _ = tx.send(RamMsg::Log(s));
            };
            let list = match ram.accounts() {
                Ok(l) => l,
                Err(e) => return log(format!("import: {e}")),
            };
            let mut n = 0;
            for a in list {
                match ram.cookie(&a).and_then(|c| {
                    let c = crate::rbx::clean_cookie(&c);
                    crate::rbx::whoami(&c).map(|(id, name)| (id, name, c))
                }) {
                    Ok((id, name, c)) => {
                        let _ = tx.send(RamMsg::Added(name, id, c));
                        n += 1;
                    }
                    Err(e) => log(format!("import {a}: {e}")),
                }
            }
            log(format!(
                "import: {n} account(s) moved in - untick Allow GetCookie in RAM now"
            ));
        });
    }

    /// Save a checked login and list the account.
    fn add_builtin(&mut self, name: String, id: u64, cookie: String) {
        if let Err(e) = self.secrets.set(&crate::rbx::cookie_key(&name), &cookie) {
            self.say(format!("saving the login failed: {e}"));
            return;
        }
        match self
            .settings
            .accounts
            .iter_mut()
            .find(|a| a.username.eq_ignore_ascii_case(&name))
        {
            Some(a) => {
                a.user_id = id;
                self.say(format!("{name}: login updated"));
            }
            None => {
                let role = if self
                    .settings
                    .accounts
                    .iter()
                    .any(|a| a.role == AccountRole::Main)
                {
                    AccountRole::Alt
                } else {
                    AccountRole::Main
                };
                self.settings.accounts.push(AccountSlot {
                    username: name.clone(),
                    role,
                    enabled: true,
                    user_id: id,
                });
                self.say(format!("{name} added ({role:?})"));
            }
        }
        self.dirty();
    }

    fn ram_load(&mut self) {
        let ram = self.ram();
        let tx = self.ram_tx.clone();
        self.ram_state = "checking…".into();
        std::thread::spawn(move || {
            if !ram.running() {
                let _ = tx.send(RamMsg::Status(
                    false,
                    "RAM isn't answering - open it and enable its web server".into(),
                ));
                return;
            }
            match ram.accounts() {
                Ok(list) => {
                    let _ = tx.send(RamMsg::Status(
                        true,
                        format!("{} account(s) in RAM", list.len()),
                    ));
                    let _ = tx.send(RamMsg::Accounts(list));
                }
                Err(e) => {
                    let _ = tx.send(RamMsg::Status(false, e.to_string()));
                }
            }
        });
    }

    /// Launch accounts through RAM. `follow_only`: these alts join the
    /// main's current server instead of using the link.
    fn ram_launch(&mut self, names: Vec<String>, follow_only: bool) {
        let ram = self.ram();
        let link = self.secrets.get(accounts::PRIVATE_SERVER_KEY);
        let tx = self.ram_tx.clone();
        let p = self.platform.clone();
        let process = self.settings.process_name.clone();
        let lock_names = self.settings.singleton_names.clone();
        let want_multi = self.settings.multi_roblox;
        let have_lock = self.multi.is_some();
        let main = self
            .settings
            .accounts
            .iter()
            .find(|a| a.role == AccountRole::Main && a.enabled)
            .map(|a| a.username.clone());
        let follow = follow_only || self.settings.alt_join == accounts::AltJoin::FollowMain;
        let builtin = self.settings.account_source == accounts::AccountSource::BuiltIn;
        // built-in: (cookie, user id) per username, read here (secrets stay on this thread)
        let logins: std::collections::HashMap<String, (String, u64)> = if builtin {
            self.settings
                .accounts
                .iter()
                .filter_map(|a| {
                    self.secrets
                        .get(&crate::rbx::cookie_key(&a.username))
                        .map(|c| (a.username.clone(), (c, a.user_id)))
                })
                .collect()
        } else {
            Default::default()
        };
        let delay = Duration::from_secs(self.settings.follow_delay_s);
        let main_in_batch = main.as_ref().is_some_and(|m| names.contains(m));
        if follow && main.is_none() {
            self.say("follow: mark one account as Main first");
            return;
        }
        self.say(format!(
            "launching {} account(s) {} - main first, alts {}s later",
            names.len(),
            if builtin {
                "with the built-in manager"
            } else {
                "through Roblox Account Manager"
            },
            self.settings.follow_delay_s
        ));
        std::thread::spawn(move || {
            let log = |s: String| {
                let _ = tx.send(RamMsg::Log(s));
            };
            if want_multi {
                match crate::multi::ensure(&*p, &process, &lock_names, have_lock, names.len(), &log)
                {
                    Ok(Some(l)) => {
                        let _ = tx.send(RamMsg::Lock(l));
                    }
                    Ok(None) => {}
                    Err(e) => log(format!("multi-Roblox failed: {e} - launching anyway")),
                }
            }
            let pids = || -> Vec<u32> {
                p.list_windows(&process)
                    .into_iter()
                    .map(|w| w.pid)
                    .collect()
            };
            let mut waited = false;
            let main_failed = std::cell::Cell::new(false);
            // which Roblox window (pid) each account got
            let pid_of: std::cell::RefCell<std::collections::HashMap<String, u32>> =
                Default::default();
            // built-in accounts: is it really in the game? (Roblox presence)
            let status = |n: &str| -> Option<crate::rbx::Presence> {
                let (c, id) = logins.get(n)?;
                if *id == 0 {
                    return None;
                }
                crate::rbx::presence(c, *id).ok()
            };
            let start_one = |n: &str| -> anyhow::Result<String> {
                let is_main = main.as_deref() == Some(n);
                // can't follow a main that didn't launch: use the server link instead
                let follow_now = follow && !main_failed.get();
                if follow && main_failed.get() && !is_main {
                    log(format!(
                        "{n}: the main didn't launch - joining by the private server link instead"
                    ));
                }
                if follow_now && !is_main {
                    let m = main.as_deref().unwrap_or_default();
                    if !builtin {
                        return ram.follow(n, m).map(|r| format!("{r} (following {m})"));
                    }
                }
                if !builtin {
                    let r = ram.launch(n, link.as_deref());
                    if is_main && r.is_err() {
                        main_failed.set(true);
                    }
                    return r;
                }
                // built-in account manager
                let (cookie, mut id) = logins.get(n).cloned().ok_or_else(|| {
                    anyhow::anyhow!("no saved login - add it on the Accounts page")
                })?;
                if id == 0 {
                    id = crate::rbx::whoami(&cookie)?.0;
                }
                log(format!("{n}: finding the server…"));
                let target = if follow_now && !is_main {
                    let m = main.as_deref().unwrap_or_default();
                    let uid = logins
                        .get(m)
                        .map(|l| l.1)
                        .filter(|i| *i > 0)
                        .map(Ok)
                        .unwrap_or_else(|| crate::rbx::user_id(m))?;
                    crate::rbx::Target::Follow { user: uid }
                } else {
                    match link.as_deref().filter(|l| !l.trim().is_empty()) {
                        Some(l) => crate::rbx::private_target(&cookie, accounts::AA_PLACE_ID, l)?,
                        None => crate::rbx::Target::Game {
                            place: accounts::AA_PLACE_ID,
                        },
                    }
                };
                if let crate::rbx::Target::Private { access, link, .. } = &target {
                    log(format!(
                        "{n}: private server link code {} chars, access code {}",
                        link.len(),
                        if access.is_empty() {
                            "not shown (using the link code)"
                        } else {
                            "found"
                        }
                    ));
                }
                crate::rbx::launch(&cookie, id, &target, &|u| p.open_uri(u))?;
                if is_main {
                    main_failed.set(false);
                }
                Ok(match &target {
                    crate::rbx::Target::Follow { .. } => "launched, following the main".into(),
                    crate::rbx::Target::Private { .. } => "launched into the private server".into(),
                    crate::rbx::Target::Game { .. } => "launched".into(),
                })
            };
            // close an account's Roblox and launch it again
            let relaunch = |n: &str| {
                if let Some(pid) = pid_of.borrow_mut().remove(n) {
                    let _ = p.kill_pid(pid);
                }
                std::thread::sleep(Duration::from_secs(3));
                let before: std::collections::HashSet<u32> = pids().into_iter().collect();
                match start_one(n) {
                    Ok(m) => log(format!("{n}: {m}")),
                    Err(e) => {
                        log(format!("{n}: launch failed - {e:#}"));
                        return;
                    }
                }
                let t = Instant::now();
                while t.elapsed() < Duration::from_secs(60) {
                    if let Some(np) = pids().into_iter().find(|x| !before.contains(x)) {
                        pid_of.borrow_mut().insert(n.to_string(), np);
                        let _ = tx.send(RamMsg::Pid(np, n.to_string()));
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
            };
            // wait for `n` to really be in the game (at least `min`); a join
            // that failed (stuck, error, kicked to the app) is relaunched, 2x max
            let wait_in_game = |n: &str, min: Duration| -> bool {
                let mut started = Instant::now();
                let mut tries = 0;
                loop {
                    std::thread::sleep(Duration::from_secs(5));
                    let st = status(n);
                    match st {
                        // no way to ask (RAM / unknown id): just the wait
                        None => {
                            if started.elapsed() >= min {
                                return true;
                            }
                        }
                        Some(crate::rbx::Presence::InGame { .. }) => {
                            if started.elapsed() >= min {
                                log(format!("{n}: in the game"));
                                return true;
                            }
                        }
                        Some(other) => {
                            let limit = Duration::from_secs(min.as_secs().max(90));
                            if started.elapsed() >= limit {
                                tries += 1;
                                if tries > 2 {
                                    log(format!(
                                        "{n}: still not in the game ({}) after 3 tries - giving up on it",
                                        other.label()
                                    ));
                                    return false;
                                }
                                log(format!(
                                    "{n}: not in the game after {}s ({}) - the join failed, closing it and launching again",
                                    limit.as_secs(),
                                    other.label()
                                ));
                                relaunch(n);
                                started = Instant::now();
                            }
                        }
                    }
                }
            };
            let mut launch = |n: &str| -> anyhow::Result<String> {
                let is_main = main.as_deref() == Some(n);
                // main first, and really in the game before any alt
                if !is_main && main_in_batch && !waited {
                    let m = main.clone().unwrap_or_default();
                    log(format!(
                        "waiting for {m} to load in before the alts (at least {}s)",
                        delay.as_secs()
                    ));
                    if !wait_in_game(&m, delay) {
                        main_failed.set(true);
                    }
                    waited = true;
                    log("launching the alts now".into());
                }
                start_one(n)
            };
            crate::multi::launch_each(
                &names,
                &pids,
                &mut launch,
                &mut |pid, n| {
                    pid_of.borrow_mut().insert(n.to_string(), pid);
                    let _ = tx.send(RamMsg::Pid(pid, n.to_string()));
                },
                &log,
                Duration::from_secs(60),
                Duration::from_millis(500),
            );
            // every alt (and a main launched alone) in the game too?
            if builtin {
                for n in &names {
                    let is_main = main.as_deref() == Some(n.as_str());
                    if is_main && waited {
                        continue; // checked before the alts
                    }
                    wait_in_game(n, Duration::from_secs(20));
                }
            }
            log("launch done".into());
            let _ = tx.send(RamMsg::LaunchDone);
        });
    }

    // ---------------- Movement ----------------

    fn load_route_text(&mut self) {
        let r = if self.route_lobby {
            self.routes.lobby.get(&self.route_sel)
        } else {
            self.routes.maps.get(&self.route_sel)
        };
        self.route_text = r
            .map(|r| serde_json::to_string_pretty(&r.steps).unwrap_or_default())
            .unwrap_or_else(|| "[]".into());
        self.route_err = None;
    }

    fn page_movement(&mut self, ui: &mut egui::Ui) {
        heading(
            ui,
            "Movement",
            "Walk to each mode's entrance from the lobby, and per-map walks after loading in",
        );
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(Vec2::new(220.0, 0.0), Layout::top_down(Align::Min), |ui| {
                card(ui, |ui| {
                    ui.set_width(196.0);
                    ui.label(RichText::new("LOBBY WALKS").size(11.0).color(MUTED));
                    let lobby: Vec<String> = self.routes.lobby.keys().cloned().collect();
                    for k in lobby {
                        let sel = self.route_lobby && self.route_sel == k;
                        if ui.selectable_label(sel, lobby_label(&k)).clicked() {
                            self.route_lobby = true;
                            self.route_sel = k;
                            self.load_route_text();
                        }
                    }
                    ui.add_space(6.0);
                    ui.label(RichText::new("MAPS (after loading in)").size(11.0).color(MUTED));
                    egui::ScrollArea::vertical().id_salt("route-maps").max_height(460.0).min_scrolled_height(460.0).show(ui, |ui| {
                        let maps: Vec<(String, u64, bool)> = self
                            .routes
                            .maps
                            .iter()
                            .map(|(k, r)| (k.clone(), r.duration_ms(), self.routes.user_maps.contains(k)))
                            .collect();
                        for (k, ms, mine) in maps {
                            let sel = !self.route_lobby && self.route_sel == k;
                            let label = if ms == 0 { format!("{k}  ·  stay") } else { format!("{k}  ·  {:.1}s", ms as f32 / 1000.0) };
                            let mut t = RichText::new(label);
                            if mine {
                                t = t.color(GOOD);
                            }
                            if ui.selectable_label(sel, t).clicked() {
                                self.route_lobby = false;
                                self.route_sel = k;
                                self.load_route_text();
                            }
                        }
                    });
                });
            });
            ui.add_space(8.0);
            ui.vertical(|ui| {
                card(ui, |ui| {
                    let title = if self.route_lobby { lobby_label(&self.route_sel) } else { self.route_sel.clone() };
                    ui.label(RichText::new(title).size(16.0).strong());
                    let src = if self.route_lobby { self.routes.lobby.get(&self.route_sel) } else { self.routes.maps.get(&self.route_sel) }
                        .map(|r| r.source.clone())
                        .unwrap_or_default();
                    ui.label(RichText::new(src).size(11.0).color(MUTED));
                    ui.label(
                        RichText::new("Steps: key_down / key_up {key} · wait {ms} · hold {keys, ms} · click {x, y} · if_image {patterns, region, then, else}. Client coords (800×599).")
                            .size(11.0)
                            .color(MUTED),
                    );
                    egui::ScrollArea::vertical().id_salt("route-text").max_height(380.0).show(ui, |ui| {
                        ui.add(egui::TextEdit::multiline(&mut self.route_text).code_editor().desired_width(f32::INFINITY).desired_rows(18));
                    });
                    if let Some(e) = &self.route_err {
                        ui.colored_label(BAD, e);
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Save as my route").clicked() {
                            self.save_route();
                        }
                        if ui.button("Revert").clicked() {
                            self.load_route_text();
                        }
                        if ui.button("Test on main").on_hover_text("walks the main client's character right now").clicked() {
                            self.test_route();
                        }
                    });
                });
            });
        });
    }

    /// `{map}` / `{act}` for routes, from the Home page.
    fn vars(&self) -> crate::movement::Vars {
        crate::movement::Vars {
            map: self.settings.run.map.clone(),
            act: self.settings.run.act.clone(),
            click_wait_ms: self.settings.step_delay_ms,
        }
    }

    fn parse_route_text(&self) -> Result<Route, String> {
        let steps = serde_json::from_str(&self.route_text)
            .map_err(|e| format!("not valid steps JSON: {e}"))?;
        let r = Route {
            map: self.route_sel.clone(),
            source: "user".into(),
            steps,
        };
        r.to_actions(&crate::game::Blind, &self.vars())
            .map_err(|e| e.to_string())?;
        Ok(r)
    }

    fn save_route(&mut self) {
        match self.parse_route_text() {
            Ok(r) => {
                let file = if self.route_lobby {
                    format!("lobby_{}", self.route_sel)
                } else {
                    self.route_sel.clone()
                };
                match self.routes.save_user(&file, &r) {
                    Ok(p) => {
                        if self.route_lobby {
                            self.routes.lobby.insert(self.route_sel.clone(), r);
                        } else {
                            self.routes.maps.insert(self.route_sel.clone(), r);
                        }
                        self.route_err = None;
                        self.say(format!("saved {}", p.display()));
                    }
                    Err(e) => self.route_err = Some(e.to_string()),
                }
            }
            Err(e) => self.route_err = Some(e),
        }
    }

    fn test_route(&mut self) {
        let Some(info) = self.main_hwnd() else {
            self.say("no main client");
            return;
        };
        let r = match self.parse_route_text() {
            Ok(r) => r,
            Err(e) => {
                self.route_err = Some(e);
                return;
            }
        };
        let screen = crate::game::LiveScreen::new(self.platform.clone(), info.hwnd, &self.lib);
        let vars = self.vars();
        let missing = screen.missing(&r.patterns(&vars));
        if !missing.is_empty() {
            self.say(format!(
                "route needs patterns you haven't captured yet: {} (Patterns page)",
                missing.join(", ")
            ));
        }
        let batch = r.to_batch(info.hwnd, &screen, &vars);
        match batch {
            Ok(b) => {
                let arb = self.arbiter();
                let tx = self.log_tx.clone();
                let name = r.map.clone();
                std::thread::spawn(move || {
                    let msg = match arb.run(b) {
                        Ok(rep) => format!("route '{name}' done in {:.1}s", rep.held.as_secs_f32()),
                        Err(e) => format!("route '{name}' stopped: {e}"),
                    };
                    let _ = tx.send(msg);
                });
            }
            Err(e) => self.route_err = Some(e.to_string()),
        }
    }

    // ---------------- Settings ----------------

    /// Summer Event card order: first one offered wins.
    fn card_priority_ui(&mut self, ui: &mut egui::Ui) {
        // start from the default; keep any new cards the default learned
        let defaults = crate::game::cards::default_priority();
        let list = &mut self.settings.run.card_priority;
        if list.is_empty() {
            *list = defaults.clone();
        }
        for d in &defaults {
            if !list.contains(d) {
                list.push(d.clone());
            }
        }
        let mut changed = false;
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("CARD PRIORITY (Summer Event)")
                        .size(11.0)
                        .color(MUTED),
                );
                if ui.small_button("Reset to default").clicked() {
                    self.settings.run.card_priority = defaults.clone();
                    changed = true;
                }
            });
            ui.label(
                RichText::new("Put them in any order: drag a card onto another, type its number, or use Top / Up / Down / Bottom. The macro reads the 3 cards with OCR and takes the first one in this list. Red cards pay Summer Pearls + secret drop chance (I +30%, II +70% & +25% drops, III +150% & +50% drops); green cards help you. Default: Plated / Health / Speed at the highest tier first.")
                    .size(11.0)
                    .color(MUTED),
            );
            let list = &mut self.settings.run.card_priority;
            // move (from, to): drag a row onto another, or type its new number
            let mut mv: Option<(usize, usize)> = None;
            let n = list.len();
            egui::ScrollArea::vertical()
                .id_salt("cardprio")
                .max_height(320.0)
                .show(ui, |ui| {
                    for (i, c) in list.iter().enumerate() {
                        let row = ui.horizontal(|ui| {
                            let mut pos = i + 1;
                            if ui
                                .add(egui::DragValue::new(&mut pos).range(1..=n).speed(0.1))
                                .on_hover_text("type the place you want it in")
                                .changed()
                            {
                                mv = Some((i, pos - 1));
                            }
                            let col = if crate::game::cards::is_green(c) {
                                GOOD
                            } else {
                                BAD
                            };
                            ui.dnd_drag_source(egui::Id::unique(("cardrow", i)), i, |ui| {
                                ui.label(RichText::new(format!("=  {c}")).color(col));
                            })
                            .response
                            .on_hover_text("drag onto another card to move it there");
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.small_button("Bottom").clicked() {
                                    mv = Some((i, n - 1));
                                }
                                if ui.small_button("Down").clicked() && i + 1 < n {
                                    mv = Some((i, i + 1));
                                }
                                if ui.small_button("Up").clicked() && i > 0 {
                                    mv = Some((i, i - 1));
                                }
                                if ui.small_button("Top").clicked() {
                                    mv = Some((i, 0));
                                }
                            });
                        });
                        if let Some(from) = row.response.dnd_release_payload::<usize>() {
                            mv = Some((*from, i));
                        }
                    }
                });
            if let Some((from, to)) = mv {
                if from != to && from < list.len() {
                    let item = list.remove(from);
                    list.insert(to.min(list.len()), item);
                    changed = true;
                }
            }
        });
        if changed {
            self.dirty();
        }
    }

    /// Settings > Recovery: disconnect / captcha handling.
    fn recovery_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        card(ui, |ui| {
            ui.label(RichText::new("DISCONNECT").size(11.0).color(MUTED));
            ui.label(
                RichText::new("A watchdog looks at every client every 5 s. Roblox's disconnect box (your disconnect_box picture, else its words): the step stops, the macro clicks Reconnect, waits for the lobby (90 s), parties up again and starts the next round. The lost stage counts as an aborted run - no Discord report for it.")
                    .size(11.0)
                    .color(MUTED),
            );
            let r = &mut self.settings.recovery;
            changed |= ui
                .checkbox(&mut r.reconnect, "Reconnect after a disconnect")
                .changed();
            ui.horizontal(|ui| {
                ui.label("At most");
                changed |= ui
                    .add(egui::DragValue::new(&mut r.max_reconnects_per_hour).range(1..=30))
                    .changed();
                ui.label("reconnects an hour");
            });
            changed |= ui
                .checkbox(
                    &mut self.settings.auto_relaunch,
                    "If reconnecting fails: close Roblox and relaunch every account (Roblox Account Manager)",
                )
                .changed();
        });
        ui.add_space(8.0);
        card(ui, |ui| {
            ui.label(RichText::new("CAPTCHA").size(11.0).color(MUTED));
            ui.label(
                RichText::new("A captcha anywhere (your captcha picture, else its words): the macro pauses, pings Discord and shows CAPTCHA on the client's card. Type it in Roblox - once it's gone the macro carries on by itself. It never solves captchas.")
                    .size(11.0)
                    .color(MUTED),
            );
            let r = &mut self.settings.recovery;
            changed |= ui
                .checkbox(&mut r.captcha_pause, "Pause while a captcha is up")
                .changed();
            changed |= ui
                .checkbox(
                    &mut r.captcha_here,
                    "Ping @here on Discord (off = only your user ping)",
                )
                .changed();
        });
        changed
    }

    /// Weather shift card list: one card per line, highest first; saved to
    /// weather_priorities.json next to the exe.
    fn weather_ui(&mut self, ui: &mut egui::Ui) {
        if self.weather_text.is_none() {
            self.weather_text = Some(crate::game::weather::to_text(&crate::game::weather::load()));
        }
        card(ui, |ui| {
            ui.label(RichText::new("WEATHER SHIFT CARDS").size(11.0).color(MUTED));
            ui.label(
                RichText::new("When the weather shifts and 3 cards are offered, the macro reads their names and votes for the one highest in this list (one card per line, top = best). None of them in the list = the leftmost card. Names as the game shows them (weather cards have no tier): \"Leyline\", \"Mud\", \"Kite Strings\". Default: the buffs first, then the SummerPearls cards from mildest to harshest.")
                    .size(11.0)
                    .color(MUTED),
            );
            let text = self.weather_text.get_or_insert_with(String::new);
            egui::ScrollArea::vertical()
                .id_salt("weatherlist")
                .max_height(320.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(text)
                            .font(egui::TextStyle::Monospace)
                            .desired_rows(14)
                            .desired_width(f32::INFINITY),
                    );
                });
            let list = crate::game::weather::from_text(text);
            let bad = crate::game::weather::unknown(&list);
            if !bad.is_empty() {
                ui.label(
                    RichText::new(format!(
                        "Not a weather card name (check spelling): {}",
                        bad.join(", ")
                    ))
                    .size(11.0)
                    .color(WARN),
                );
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!list.is_empty(), egui::Button::new("Save"))
                    .clicked()
                {
                    match crate::game::weather::save(&list) {
                        Ok(p) => self.say(format!(
                            "weather list saved ({} cards): {}",
                            list.len(),
                            p.display()
                        )),
                        Err(e) => self.say(format!("weather list not saved: {e}")),
                    }
                }
                if ui.button("Reset to default").clicked() {
                    self.weather_text = Some(crate::game::weather::to_text(
                        &crate::game::weather::default_list(),
                    ));
                }
                if ui.button("Reload from file").clicked() {
                    self.weather_text =
                        Some(crate::game::weather::to_text(&crate::game::weather::load()));
                }
                ui.label(
                    RichText::new(format!("{} cards", list.len()))
                        .size(11.0)
                        .color(MUTED),
                );
            });
        });
    }

    /// Mahoraga's adaptation order per badge (ATK / DEF / BOSS).
    fn adaptations_ui(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        card(ui, |ui| {
            ui.label(
                RichText::new("ADAPTATIONS (MAHORAGA)")
                    .size(11.0)
                    .color(MUTED),
            );
            let a = &mut self.settings.adaptations;
            changed |= toggle(ui, &mut a.enabled, "Pick adaptations automatically");
            ui.label(
                RichText::new(
                    "When Megumi's Mahoraga asks every 50 kills. Takes the highest choice here that isn't grey (grey = already taken).",
                )
                .size(11.0)
                .color(MUTED),
            );
            ui.horizontal_top(|ui| {
                for (title, list) in [
                    ("ATK", &mut a.atk),
                    ("DEF", &mut a.def),
                    ("BOSS", &mut a.boss),
                ] {
                    // keep every name once, in order
                    for n in crate::config::ADAPTATION_NAMES {
                        if !list.iter().any(|m| m == n) {
                            list.push(n.to_string());
                        }
                    }
                    ui.vertical(|ui| {
                        ui.label(RichText::new(title).strong());
                        let len = list.len();
                        let mut swap = None;
                        for i in 0..len {
                            ui.horizontal(|ui| {
                                ui.label(format!("{}. {}", i + 1, list[i]));
                                if ui
                                    .add_enabled(i > 0, egui::Button::new("Up").small())
                                    .clicked()
                                {
                                    swap = Some((i, i - 1));
                                }
                                if ui
                                    .add_enabled(i + 1 < len, egui::Button::new("Down").small())
                                    .clicked()
                                {
                                    swap = Some((i, i + 1));
                                }
                            });
                        }
                        if let Some((x, y)) = swap {
                            list.swap(x, y);
                            changed = true;
                        }
                    });
                    ui.add_space(12.0);
                }
            });
        });
        if changed {
            self.dirty();
        }
    }

    fn page_settings(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Settings", "");
        segmented(
            ui,
            &mut self.settings_tab,
            &[
                (SettingsTab::General, "General"),
                (SettingsTab::Input, "Input & timing"),
                (SettingsTab::Game, "Cards & picks"),
                (SettingsTab::Discord, "Discord"),
                (SettingsTab::Weather, "Weather"),
                (SettingsTab::Recovery, "Recovery"),
                (SettingsTab::Updates, "Updates"),
            ],
        );
        ui.add_space(8.0);
        let tab = self.settings_tab;
        let mut changed = false;
        let mut input_changed = false;
        if tab == SettingsTab::General && self.platform.is_remote_session() {
            card(ui, |ui| {
                ui.label(RichText::new("REMOTE DESKTOP").size(11.0).color(WARN));
                ui.label(
                    "You're using this PC over Remote Desktop. When the RDP window is minimized or disconnected, Windows stops all mouse / keyboard input to this PC and the macro can't click (\"Input is blocked\").",
                );
                ui.label(RichText::new("Fix (on the PC you connect FROM):").strong());
                ui.label(
                    RichText::new(
                        "1. Open regedit > HKEY_CURRENT_USER\\Software\\Microsoft\\Terminal Server Client\n2. New DWORD (32-bit) named RemoteDesktop_SuppressWhenMinimized, value 2\n3. Close and reopen Remote Desktop. Now minimizing it is fine.",
                    )
                    .size(11.0),
                );
                ui.label(
                    RichText::new(
                        "Don't just close / disconnect RDP either - that locks this PC. To leave it running: on this PC open cmd as admin, run  query session  to see your session ID, then  tscon <ID> /dest:console  (it disconnects you but keeps this PC unlocked).",
                    )
                    .size(11.0)
                    .color(MUTED),
                );
            });
            ui.add_space(8.0);
        }
        if tab == SettingsTab::General {
            card(ui, |ui| {
                ui.label(
                    RichText::new("MULTI-ROBLOX & DOCKING")
                        .size(11.0)
                        .color(MUTED),
                );
                changed |= toggle(
                    ui,
                    &mut self.settings.multi_roblox,
                    "Multi-Roblox built in (no RAM Multi Roblox needed)",
                );
                ui.label(
                RichText::new(match &self.multi {
                    Some(l) if l.shared => "On - the lock is shared with RAM, that's fine.",
                    Some(_) => "On - every client opened from now on can stay open.",
                    None => "Off right now. Launching 2+ accounts from Accounts closes any open Roblox first, switches it on, then launches them.",
                })
                .size(11.0)
                .color(if self.multi.is_some() { GOOD } else { MUTED }),
            );
                if self.settings.multi_roblox && self.multi.is_none() {
                    if ui
                        .button("Turn multi-Roblox on now")
                        .on_hover_text("closes any open Roblox first, then takes the lock")
                        .clicked()
                    {
                        let p = self.platform.clone();
                        let process = self.settings.process_name.clone();
                        let names = self.settings.singleton_names.clone();
                        let tx = self.ram_tx.clone();
                        std::thread::spawn(move || {
                            let log = |s: String| {
                                let _ = tx.send(RamMsg::Log(s));
                            };
                            // `adding` 2 forces it even with nothing open
                            match crate::multi::ensure(&*p, &process, &names, false, 2, &log) {
                                Ok(Some(l)) => {
                                    let _ = tx.send(RamMsg::Lock(l));
                                }
                                Ok(None) => {}
                                Err(e) => log(format!("multi-Roblox failed: {e:#}")),
                            }
                        });
                    }
                }
                ui.add_space(6.0);
                changed |= toggle(
                    ui,
                    &mut self.settings.dock,
                    "Dock Roblox into the app (left column)",
                );
                ui.horizontal(|ui| {
                    ui.label("Nudge docked Roblox  x");
                    changed |= ui
                        .add(egui::DragValue::new(&mut self.settings.dock_dx).range(-200..=200))
                        .changed();
                    ui.label("y");
                    changed |= ui
                        .add(egui::DragValue::new(&mut self.settings.dock_dy).range(-200..=200))
                        .changed();
                    ui.label(
                        RichText::new("px, only if it still sits off")
                            .size(11.0)
                            .color(MUTED),
                    );
                });
                if !self.dock_note.is_empty() {
                    ui.label(RichText::new(&self.dock_note).size(11.0).color(MUTED));
                }
            });
        }
        ui.add_space(8.0);
        if tab == SettingsTab::Weather {
            self.weather_ui(ui);
        }
        if tab == SettingsTab::Recovery {
            changed |= self.recovery_ui(ui);
        }
        if tab == SettingsTab::Game {
            self.card_priority_ui(ui);
            ui.add_space(8.0);
            self.adaptations_ui(ui);
        }
        ui.add_space(8.0);
        if tab == SettingsTab::General {
            card(ui, |ui| {
                ui.label(RichText::new("KEYBINDS").size(11.0).color(MUTED));
                let kb = &mut self.settings.keybinds;
                ui.horizontal(|ui| {
                    for (i, k) in kb.units.iter_mut().enumerate() {
                        ui.label(format!("Unit {}", i + 1));
                        changed |= ui
                            .add(egui::TextEdit::singleline(k).desired_width(28.0))
                            .changed();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Upgrade");
                    changed |= ui
                        .add(egui::TextEdit::singleline(&mut kb.upgrade).desired_width(28.0))
                        .changed();
                    ui.label("Cancel / close panel");
                    changed |= ui
                        .add(egui::TextEdit::singleline(&mut kb.cancel).desired_width(28.0))
                        .changed();
                });
                let bad: Vec<String> = kb
                    .units
                    .iter()
                    .chain([&kb.upgrade, &kb.cancel])
                    .filter(|k| crate::keys::Key::parse(k).is_none())
                    .cloned()
                    .collect();
                if !bad.is_empty() {
                    ui.colored_label(BAD, format!("unknown key(s): {}", bad.join(", ")));
                }
            });
        }
        ui.add_space(8.0);
        if tab == SettingsTab::Input {
            card(ui, |ui| {
                ui.label(RichText::new("CLIENTS & INPUT").size(11.0).color(MUTED));
                let s = &mut self.settings;
                egui::Grid::new("settings")
                .spacing([14.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Roblox process");
                    changed |= ui
                        .add_sized(
                            [220.0, 22.0],
                            egui::TextEdit::singleline(&mut s.process_name),
                        )
                        .changed();
                    ui.end_row();
                    ui.label("Max clients");
                    changed |= ui
                        .add(egui::DragValue::new(&mut s.max_instances).range(1..=4))
                        .changed();
                    ui.end_row();
                    ui.label("Mouse settle (ms)");
                    changed |= ui
                        .add(egui::DragValue::new(&mut s.mouse_settle_ms).range(0..=500))
                        .changed();
                    ui.end_row();
                    ui.label("Click hold (ms)");
                    changed |= ui
                        .add(egui::DragValue::new(&mut s.click_hold_ms).range(10..=500))
                        .changed();
                    ui.end_row();
                    ui.label("Delay after each click (ms)");
                    changed |= ui
                        .add(egui::DragValue::new(&mut s.step_delay_ms).range(0..=5000))
                        .on_hover_text(
                            "menus / settings need a moment to open before the next look or click",
                        )
                        .changed();
                    ui.end_row();
                    ui.label("Next below VICTORY/DEFEAT (px)");
                    changed |= ui
                        .add(egui::DragValue::new(&mut s.next_below_title).range(50..=400))
                        .on_hover_text("only used when the Next button itself isn't recognised")
                        .changed();
                    ui.end_row();
                    ui.label("Anti-AFK on alts");
                    ui.horizontal(|ui| {
                        ui.label("5 clicks every");
                        changed |= ui
                            .add(egui::DragValue::new(&mut s.afk_every_min).range(0..=19).suffix(" min"))
                            .on_hover_text("0 = off. Roblox kicks after 20 min idle")
                            .changed();
                        ui.label("(at the keep-alive spot)");
                    });
                    ui.end_row();
                    ui.label("When stuck");
                    changed |= toggle(
                        ui,
                        &mut s.auto_relaunch,
                        "close all Roblox, relaunch the accounts (main first) and start again",
                    );
                    ui.end_row();
                    ui.label("Keep-alive click");
                    ui.horizontal(|ui| {
                        changed |= toggle(ui, &mut s.idle_click, "every 5 s once all units are upgraded");
                        ui.label("at");
                        changed |= ui.add(egui::DragValue::new(&mut s.idle_click_at.0).range(0..=799)).changed();
                        changed |= ui.add(egui::DragValue::new(&mut s.idle_click_at.1).range(0..=598)).changed();
                    });
                    ui.end_row();
                    ui.label("Delay after switching account (ms)");
                    if ui
                        .add(egui::DragValue::new(&mut s.swap_delay_ms).range(0..=10000))
                        .on_hover_text("wait after swapping to another Roblox before clicking there (lag spikes)")
                        .changed()
                    {
                        changed = true;
                        input_changed = true;
                    }
                    ui.end_row();
                    ui.label("Delay between placements (ms)");
                    changed |= ui
                        .add(egui::DragValue::new(&mut s.place_delay_ms).range(0..=10000))
                        .changed();
                    ui.end_row();
                    ui.label("Pause between placing steps (ms)").on_hover_text(
                        "select unit > pause > click > pause > check it went down > pause > next account",
                    );
                    changed |= ui
                        .add(egui::DragValue::new(&mut s.place_settle_ms).range(100..=5000))
                        .changed();
                    ui.end_row();
                    ui.label("Upgrade press limit").on_hover_text(
                        "optional safety: after this many upgrade presses on one unit without seeing MAX / its level, count it as done and move to the next unit",
                    );
                    ui.horizontal(|ui| {
                        let mut on = s.upgrade_press_cap > 0;
                        if ui.checkbox(&mut on, "").changed() {
                            s.upgrade_press_cap = if on { 60 } else { 0 };
                            changed = true;
                        }
                        if on {
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut s.upgrade_press_cap)
                                        .range(1..=999)
                                        .suffix(" presses"),
                                )
                                .changed();
                        } else {
                            ui.label(RichText::new("off").color(MUTED));
                        }
                    });
                    ui.end_row();
                    ui.label("Open units with right-click").on_hover_text(
                        "try a right-click first when opening a unit to upgrade it (then left-clicks around the spot). Off = left-click. Roblox turns the camera on a right-drag, so leave it off unless it opens panels for you.",
                    );
                    changed |= ui
                        .checkbox(&mut s.upgrade_right_click, "right-click first")
                        .changed();
                    ui.end_row();
                    ui.label("Upgrade read wait (ms)").on_hover_text(
                        "after opening a unit / pressing upgrade, wait this long before reading its level",
                    );
                    changed |= ui
                        .add(egui::DragValue::new(&mut s.upgrade_read_ms).range(0..=5000))
                        .changed();
                    ui.end_row();
                    ui.label("Placed check time (ms)").on_hover_text(
                        "how long it keeps looking for the unit panel / UPG [0] after the click",
                    );
                    changed |= ui
                        .add(egui::DragValue::new(&mut s.place_check_ms).range(500..=10000))
                        .changed();
                    ui.end_row();
                    ui.label("Clicks");
                    let before = s.click_mode;
                    segmented(
                        ui,
                        &mut s.click_mode,
                        &[
                            (crate::input::ClickMode::Single, "Single"),
                            (crate::input::ClickMode::Safe, "Safe"),
                            (crate::input::ClickMode::Double, "Double"),
                        ],
                    );
                    if s.click_mode != before {
                        changed = true;
                        input_changed = true;
                    }
                    ui.end_row();
                    ui.label("Failure dumps").on_hover_text(
                        "on a failed step / unreadable read the macro saves screenshots, crops, state and log to debug/ (newest 50 kept)",
                    );
                    ui.horizontal(|ui| {
                        changed |= ui
                            .checkbox(&mut s.dump_on_miss, "also when a waited-for picture never shows")
                            .changed();
                        if ui.button("Open debug folder").clicked() {
                            let d = crate::game::dump::root();
                            let _ = std::fs::create_dir_all(&d);
                            let _ = self.platform.open_uri(&d.display().to_string());
                        }
                    });
                    ui.end_row();
                    ui.label("Step machine (beta)").on_hover_text(
                        "rounds run as steps (Lobby > Stage setup > Walk > Vote > Place > Upgrade > End): each retried by its own limits; a step that keeps failing pauses the macro with a failure dump (F4 resumes). Off = the old loop.",
                    );
                    changed |= ui
                        .checkbox(&mut s.step_machine, "use the step machine instead of the old loop")
                        .changed();
                    ui.end_row();
                    ui.label("Resume after a restart").on_hover_text(
                        "the macro saves its step; started again within 5 min mid-stage, it can carry on (e.g. upgrading) instead of starting from the vote",
                    );
                    changed |= segmented(
                        ui,
                        &mut s.resume_mode,
                        &[
                            (crate::config::ResumeMode::Ask, "Ask"),
                            (crate::config::ResumeMode::Auto, "Auto"),
                            (crate::config::ResumeMode::Off, "Off"),
                        ],
                    );
                    ui.end_row();
                    ui.label("Debug overlay").on_hover_text(
                        "boxes over the game: pattern hits (green), OCR text (cyan), clicks (crosshair), step + picture age. Click-through.",
                    );
                    ui.horizontal(|ui| {
                        let mut on = crate::debugbus::enabled();
                        if ui
                            .checkbox(&mut on, format!("show ({})", s.keybinds.overlay))
                            .changed()
                        {
                            crate::debugbus::set_show_misses(s.overlay_misses);
                            crate::debugbus::set_enabled(on);
                            #[cfg(windows)]
                            crate::platform::overlay::spawn();
                        }
                        if ui.checkbox(&mut s.overlay_misses, "misses too (red)").changed() {
                            crate::debugbus::set_show_misses(s.overlay_misses);
                            changed = true;
                        }
                    });
                    ui.end_row();
                    ui.label("Fixed buttons")
                        .on_hover_text("Settings cog, its X, party close: once found, their spot is kept and clicked straight away (no search).");
                    if ui.button("Forget kept spots").clicked() {
                        crate::game::buttonmem::forget(None);
                    }
                    ui.end_row();
                    ui.label("Placement check")
                        .on_hover_text("How the macro knows a unit went down. Try the hotbar counter if units are placed but not counted.");
                    egui::ComboBox::from_id_salt("placement_detect")
                        .selected_text(s.placement_detect.label())
                        .show_ui(ui, |ui| {
                            for d in crate::config::PlacementDetect::ALL {
                                changed |= ui
                                    .selectable_value(&mut s.placement_detect, d, d.label())
                                    .changed();
                            }
                        });
                    ui.end_row();
                    ui.label("Max input turn (ms)");
                    changed |= ui
                        .add(egui::DragValue::new(&mut s.max_lease_ms).range(100..=2000))
                        .changed();
                    ui.end_row();
                });
                ui.add_space(4.0);
                ui.label(RichText::new("Client names").color(MUTED));
                ui.horizontal(|ui| {
                    for (i, p) in self.settings.instances.iter_mut().enumerate().take(4) {
                        ui.label(format!("#{}", i + 1));
                        changed |= ui
                            .add(egui::TextEdit::singleline(&mut p.label).desired_width(90.0))
                            .changed();
                    }
                });
            });
        }
        if input_changed && !self.running {
            self.arbiter = None; // next action uses the new click mode
        }
        if changed {
            self.dirty();
        }
        ui.add_space(8.0);
        if tab == SettingsTab::General {
            card(ui, |ui| {
                ui.label(RichText::new("CAMERA (TOP-DOWN)").size(11.0).color(MUTED));
                let c = &mut self.settings.camera;
                ui.horizontal(|ui| {
                    ui.label("Zoom in (notches)");
                    changed |= ui
                        .add(egui::DragValue::new(&mut c.zoom_in).range(0..=40))
                        .changed();
                    ui.label("Look down (px)");
                    changed |= ui
                        .add(egui::DragValue::new(&mut c.look_down_px).range(0..=1500))
                        .changed();
                    ui.label("Zoom out (notches)");
                    changed |= ui
                        .add(egui::DragValue::new(&mut c.zoom_out).range(0..=60))
                        .changed();
                });
                ui.label(RichText::new("Zoom fully in (first person), look at the floor, zoom back out = top-down view. Try it with Placement > Camera top-down.").size(11.0).color(MUTED));
            });
        }
        if changed {
            self.dirty();
        }
        ui.add_space(8.0);
        if tab == SettingsTab::Discord {
            card(ui, |ui| self.webhook_section(ui));
        }
        ui.add_space(8.0);
        if tab == SettingsTab::Updates {
            card(ui, |ui| self.updates_section(ui));
        }
        ui.add_space(8.0);
        if tab != SettingsTab::General {
            return;
        }
        ui.label(
            RichText::new(format!("Settings: {}", self.settings_path.display()))
                .size(11.0)
                .color(MUTED),
        );
        ui.label(
            RichText::new(format!("Patterns: {}", self.lib.dir.display()))
                .size(11.0)
                .color(MUTED),
        );
        ui.label(
            RichText::new(format!(
                "Routes (yours): {}",
                self.routes.user_dir.display()
            ))
            .size(11.0)
            .color(MUTED),
        );
    }

    fn webhook_section(&mut self, ui: &mut egui::Ui) {
        use crate::webhook;
        ui.label(RichText::new("DISCORD WEBHOOK").size(11.0).color(MUTED));
        let mut changed = false;
        changed |= toggle(
            ui,
            &mut self.settings.webhook.enabled,
            "Send notifications to Discord",
        );
        let has = self.secrets.has(webhook::URL_KEY);
        ui.horizontal(|ui| {
            ui.label("Webhook URL");
            ui.add(
                egui::TextEdit::singleline(&mut self.hook_input)
                    .password(true)
                    .desired_width(330.0)
                    .hint_text(if has { "saved (encrypted)" } else { "https://discord.com/api/webhooks/…" }),
            );
            if ui.button("Save").clicked() {
                let u = self.hook_input.trim().to_string();
                if !u.is_empty() && !webhook::valid_url(&u) {
                    self.say("that isn't a Discord webhook URL (Server Settings > Integrations > Webhooks > Copy URL)");
                } else if self.secrets.set(webhook::URL_KEY, &u).is_ok() {
                    self.hook_input.clear();
                    self.say(if u.is_empty() { "webhook removed" } else { "webhook saved (encrypted)" });
                }
            }
            if ui.add_enabled(has, egui::Button::new("Test")).clicked() {
                let url = self.secrets.get(webhook::URL_KEY).unwrap_or_default();
                let ping = self.settings.webhook.ping_user.clone();
                let shot = self.main_hwnd().and_then(|c| self.platform.capture_client(c.hwnd).ok());
                let tx = self.log_tx.clone();
                std::thread::spawn(move || {
                    let m = webhook::Message {
                        kind: webhook::Kind::Info,
                        title: "Webhook works".into(),
                        body: "Notifications from Anime Adventure Macro will arrive here.".into(),
                        screenshot: shot,
                        ping: true,
                        fields: vec![],
                        here: false,
                    };
                    let _ = tx.send(match webhook::send(&url, &ping, &m) {
                        Ok(()) => "webhook test sent - check Discord".into(),
                        Err(e) => format!("webhook test failed: {e}"),
                    });
                });
            }
        });
        ui.horizontal(|ui| {
            ui.label("Ping my Discord user ID");
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut self.settings.webhook.ping_user)
                        .desired_width(170.0)
                        .hint_text("digits, optional"),
                )
                .changed();
        });
        ui.horizontal(|ui| {
            let w = &mut self.settings.webhook;
            changed |= toggle(ui, &mut w.on_result, "Win / loss");
            changed |= toggle(ui, &mut w.loot_report, "as loot report");
            changed |= toggle(ui, &mut w.screenshot, "with screenshot");
            changed |= toggle(ui, &mut w.on_disconnect, "Disconnects");
            changed |= toggle(ui, &mut w.on_start_stop, "Start / stop");
        });
        ui.label(
            RichText::new("Loot report: after every run - Run #, event currency, pearls, charm (+ its picture), session totals. The macro waits for the upload (max 15 s) before Replay. History: runtime/run_history.json.")
                .size(11.0)
                .color(MUTED),
        );
        ui.label(
            RichText::new("The URL is stored encrypted and never shown again. User ID: Discord > Settings > Advanced > Developer Mode, then right-click your name > Copy User ID.")
                .size(11.0)
                .color(MUTED),
        );
        if changed {
            self.dirty();
        }
    }

    fn updates_section(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("UPDATES").size(11.0).color(MUTED));
        ui.label(format!("This build: {}", update::version_label()));
        let has_tok = self.secrets.has(update::TOKEN_KEY);
        ui.horizontal(|ui| {
            ui.label("GitHub token");
            ui.add(
                egui::TextEdit::singleline(&mut self.token_input)
                    .password(true)
                    .desired_width(260.0)
                    .hint_text(if has_tok {
                        "saved (encrypted)"
                    } else {
                        "github_pat_…"
                    }),
            );
            if ui.button("Save token").clicked() {
                let t = self.token_input.clone();
                match self.secrets.set(update::TOKEN_KEY, &t) {
                    Ok(()) => {
                        self.token_input.clear();
                        self.say(if t.trim().is_empty() {
                            "token removed"
                        } else {
                            "token saved (encrypted)"
                        });
                    }
                    Err(e) => self.say(format!("saving token failed: {e}")),
                }
            }
        });
        if toggle(
            ui,
            &mut self.settings.auto_update_check,
            "Check for updates when the app opens",
        ) {
            self.dirty();
        }
        ui.horizontal(|ui| {
            let busy = matches!(self.upd, Upd::Checking | Upd::Installing);
            if ui
                .add_enabled(!busy, egui::Button::new("Check now"))
                .clicked()
            {
                self.check_update();
            }
            match &self.upd {
                Upd::Idle => {}
                Upd::Checking => {
                    ui.spinner();
                }
                Upd::UpToDate => {
                    ui.colored_label(GOOD, "Up to date");
                }
                Upd::Available(a) => {
                    ui.colored_label(
                        GOOD,
                        format!("Build {} available - see the banner at the top", a.build),
                    );
                }
                Upd::Installing => {
                    ui.label("Installing…");
                }
                Upd::Installed(_) => {
                    ui.colored_label(GOOD, "Installed - restart from the banner");
                }
                Upd::Failed(e) => {
                    ui.colored_label(WARN, e);
                }
            }
        });
        ui.label(
            RichText::new("Token: GitHub > Settings > Developer settings > Fine-grained tokens > Generate. Repository access: only aa-macro-rs. Permissions: Contents = Read-only.")
                .size(11.0)
                .color(MUTED),
        );
    }

    // ---------------- per-frame housekeeping ----------------

    fn tick(&mut self, ctx: &egui::Context) {
        while let Ok(m) = self.log_rx.try_recv() {
            self.say(m);
        }
        while let Ok(u) = self.upd_rx.try_recv() {
            if let Upd::Failed(e) = &u {
                self.say(format!("update: {e}"));
            }
            self.upd = u;
        }
        while let Ok(m) = self.ram_rx.try_recv() {
            match m {
                RamMsg::Status(ok, s) => {
                    self.ram_ok = Some(ok);
                    self.ram_state = s.clone();
                    self.say(format!("RAM: {s}"));
                }
                RamMsg::Accounts(l) => self.ram_list = l,
                RamMsg::Log(s) => self.say(s),
                RamMsg::Pid(pid, name) => {
                    self.names.insert(pid, name);
                    self.refresh_quiet();
                    self.apply_account_placing();
                }
                RamMsg::PidLog(pid, name, log) => {
                    // never two clients with one name: first one wins
                    let dup = self.names.iter().any(|(p, n)| *p != pid && *n == name);
                    self.name_logs.insert(pid, log);
                    if !dup {
                        self.names.insert(pid, name);
                        self.apply_account_placing();
                    }
                }
                RamMsg::Lock(l) => self.multi = Some(l),
                RamMsg::Added(name, id, cookie) => self.add_builtin(name, id, cookie),
                RamMsg::LaunchDone => {
                    if self.relaunch.is_some() {
                        self.relaunch_launched = Some(Instant::now());
                    }
                    self.refresh_clients();
                    if !self.settings.dock {
                        self.tile();
                    }
                }
            }
        }
        while let Ok(k) = self.hotkeys.try_recv() {
            self.hotkey(k);
        }
        if let Some(r) = self.resume_prompt.clone() {
            let mut answer = None;
            egui::Window::new("Pick up where it stopped?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    let map = r.step_params["map"].as_str().unwrap_or("?").to_string();
                    let n = crate::game::resume::placed(&r).len();
                    ui.label(format!(
                        "The macro was at '{}' on {map} {} s ago{}.",
                        r.step,
                        r.age_secs(),
                        if n > 0 { format!(" with {n} unit(s) placed") } else { String::new() }
                    ));
                    ui.label(
                        RichText::new("Resume carries on from there (e.g. upgrading). Start fresh begins from the lobby / vote.")
                            .size(11.0)
                            .color(MUTED),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Resume").clicked() {
                            answer = Some(true);
                        }
                        if ui.button("Start fresh").clicked() {
                            answer = Some(false);
                        }
                    });
                });
            if let Some(a) = answer {
                self.resume_prompt = None;
                self.resume_answer = Some(a);
                self.start();
            }
        }
        self.run_events();
        self.relaunch_tick();
        if self.last_refresh.elapsed() > Duration::from_secs(3) {
            self.refresh_quiet();
        }
        let wants_live =
            self.live && matches!(self.page, Page::Home | Page::Placement | Page::Patterns);
        if wants_live && self.last_cap.elapsed() > Duration::from_millis(300) {
            self.capture(ctx);
        }
        if self.page == Page::Home
            && self.settings.run.own_team
            && self.last_alt_cap.elapsed() > Duration::from_secs(2)
        {
            self.capture_alts(ctx);
        }
        if let Some(t) = self.dirty_at {
            if t.elapsed() > Duration::from_millis(600) {
                self.dirty_at = None;
                if let Err(e) = self.settings.save(&self.settings_path) {
                    self.say(format!("saving settings failed: {e}"));
                }
            }
        }
        ctx.request_repaint_after(Duration::from_millis(
            if wants_live || self.running || self.settings.dock {
                300
            } else {
                1000
            },
        ));
    }

    fn capture_alts(&mut self, ctx: &egui::Context) {
        self.last_alt_cap = Instant::now();
        self.alt_tex.resize_with(self.clients.len(), || None);
        for (i, c) in self.clients.clone().iter().enumerate() {
            if i == self.main {
                continue;
            }
            if let Ok(f) = self.platform.capture_client(c.hwnd) {
                let small = downscale(&f, 4);
                let img = ColorImage::from_rgba_unmultiplied(
                    [small.width as usize, small.height as usize],
                    &tools::rgba(&small),
                );
                match &mut self.alt_tex[i] {
                    Some(t) => t.set(img, TextureOptions::LINEAR),
                    slot => {
                        *slot =
                            Some(ctx.load_texture(format!("alt{i}"), img, TextureOptions::LINEAR))
                    }
                }
            }
        }
    }
}

// ---------------- small widgets ----------------

fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(12)
        .inner_margin(14)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn heading(ui: &mut egui::Ui, title: &str, sub: &str) {
    ui.label(RichText::new(title).size(20.0).strong().color(TEXT));
    if !sub.is_empty() {
        ui.label(RichText::new(sub).color(MUTED));
    }
    ui.add_space(8.0);
}

fn chip(ui: &mut egui::Ui, text: &str, color: Color32, dot_color: Option<Color32>) {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(14)
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(c) = dot_color {
                    dot(ui, c);
                }
                ui.label(RichText::new(text).size(12.0).color(color));
            });
        });
}

fn dot(ui: &mut egui::Ui, c: Color32) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(10.0, 10.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 4.0, c);
}

fn stat(ui: &mut egui::Ui, value: &str, label: &str, color: Color32) {
    ui.vertical(|ui| {
        ui.label(RichText::new(value).size(20.0).strong().color(color));
        ui.label(RichText::new(label).size(11.0).color(MUTED));
    });
    ui.add_space(6.0);
}

/// iOS-style switch + label. Returns true when flipped.
fn toggle(ui: &mut egui::Ui, on: &mut bool, label: &str) -> bool {
    let mut flipped = false;
    ui.horizontal(|ui| {
        let size = Vec2::new(34.0, 18.0);
        let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
        if resp.clicked() {
            *on = !*on;
            flipped = true;
            resp.mark_changed();
        }
        let t = ui.ctx().animate_bool_responsive(resp.id, *on);
        let fill = if *on { PURPLE } else { LINE };
        ui.painter().rect_filled(rect, CornerRadius::same(9), fill);
        let x = egui::lerp(rect.left() + 9.0..=rect.right() - 9.0, t);
        ui.painter()
            .circle_filled(egui::pos2(x, rect.center().y), 7.0, Color32::WHITE);
        if !label.is_empty()
            && ui
                .add(egui::Label::new(RichText::new(label).color(TEXT)).sense(Sense::click()))
                .clicked()
        {
            *on = !*on;
            flipped = true;
        }
    });
    flipped
}

/// Small on/off pill (FARM / BUFF).
/// `tag` with a tooltip.
fn tag_tip(ui: &mut egui::Ui, on: &mut bool, label: &str, tip: &str) -> bool {
    let txt =
        RichText::new(label)
            .size(10.5)
            .strong()
            .color(if *on { Color32::WHITE } else { MUTED });
    let b = egui::Button::new(txt)
        .fill(if *on { PURPLE } else { CARD })
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(8)
        .min_size(Vec2::new(24.0, 22.0));
    if ui.add(b).on_hover_text(tip).clicked() {
        *on = !*on;
        return true;
    }
    false
}

/// Pill-shaped segmented control. Returns true when the choice changed.
fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, v: &mut T, opts: &[(T, &str)]) -> bool {
    let mut changed = false;
    egui::Frame::new()
        .fill(BG)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(10)
        .inner_margin(3)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (val, label) in opts {
                    let sel = *v == *val;
                    let b = egui::Button::new(RichText::new(*label).color(if sel {
                        Color32::WHITE
                    } else {
                        MUTED
                    }))
                    .fill(if sel { CARD_HI } else { Color32::TRANSPARENT })
                    .stroke(Stroke::NONE)
                    .corner_radius(8)
                    .min_size(Vec2::new(84.0, 26.0));
                    if ui.add(b).clicked() && !sel {
                        *v = *val;
                        changed = true;
                    }
                }
            });
        });
    changed
}

fn stage_key(m: Mode) -> Option<&'static str> {
    match m {
        Mode::Story => Some("story"),
        Mode::Legend => Some("legend"),
        Mode::Raid => Some("raid"),
        _ => None,
    }
}

fn lobby_label(k: &str) -> String {
    match k {
        "summer" => "Lobby > Summer Event",
        "story" => "Lobby > Story",
        "legend" => "Lobby > Legend",
        "raid" => "Lobby > Raid",
        "castle" => "Lobby > Infinity Castle",
        "challenge" => "Lobby > Challenge",
        other => other,
    }
    .to_string()
}

fn client_name(i: usize, s: &Settings) -> String {
    s.instances
        .get(i)
        .map(|p| p.label.clone())
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| format!("Client {}", i + 1))
}

/// Every `k`-th pixel, for cheap thumbnails.
fn downscale(f: &Frame, k: u32) -> Frame {
    let (w, h) = (f.width / k, f.height / k);
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let i = (((y * k) * f.width + x * k) * 4) as usize;
            v.extend_from_slice(&f.bgra[i..i + 4]);
        }
    }
    Frame::new(w, h, v)
}

fn apply_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = CARD;
    v.extreme_bg_color = BG;
    v.faint_bg_color = CARD_HI;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = PURPLE_DIM;
    v.selection.stroke = Stroke::new(1.0, PURPLE);
    v.hyperlink_color = PURPLE;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    let r = CornerRadius::same(8);
    v.widgets.noninteractive.bg_fill = CARD;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.bg_fill = CARD_HI;
    v.widgets.inactive.weak_bg_fill = CARD_HI;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.bg_fill = Color32::from_rgb(40, 30, 78);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(40, 30, 78);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, PURPLE_DIM);
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.bg_fill = PURPLE_DIM;
    v.widgets.active.weak_bg_fill = PURPLE_DIM;
    v.widgets.active.corner_radius = r;
    v.widgets.open.bg_fill = CARD_HI;
    v.widgets.open.corner_radius = r;
    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 8.0);
        s.spacing.button_padding = Vec2::new(12.0, 5.0);
        s.spacing.interact_size.y = 26.0;
    });
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.own_hwnd.is_none() {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(h) = frame.window_handle() {
                if let RawWindowHandle::Win32(w) = h.as_raw() {
                    self.own_hwnd = Some(crate::geom::Hwnd(w.hwnd.get()));
                }
            }
        }
        self.tick(&ctx);
        egui::Panel::top("top")
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(10, 8)),
            )
            .show(ui, |ui| {
                self.top_bar(ui);
            });
        egui::Panel::bottom("logdock")
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(10, 6)),
            )
            .show(ui, |ui| {
                self.log_dock(ui);
            });
        egui::Panel::left("rail")
            .exact_size(158.0)
            .frame(egui::Frame::new().fill(BG))
            .show(ui, |ui| {
                self.rail(ui);
            });
        let ppp = ctx.pixels_per_point();
        if self.settings.dock && self.page != Page::Dashboard {
            let w = self.settings.client_w as f32 / ppp + 24.0;
            egui::Panel::left("dock")
                .exact_size(w)
                .resizable(false)
                .frame(egui::Frame::new().fill(PANEL).inner_margin(12))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("dockscroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.dock_column(ui));
                });
        } else {
            self.dock_rect = None;
        }
        egui::CentralPanel::default_margins()
            .frame(egui::Frame::new().fill(PANEL).inner_margin(12))
            .show(ui, |ui| {
                if self.other_instance {
                    egui::Frame::new()
                        .fill(Color32::from_rgb(70, 22, 30))
                        .stroke(Stroke::new(1.0, BAD))
                        .corner_radius(10)
                        .inner_margin(10)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new("Another copy of the macro is already running. It can click and place on its own - close it in Task Manager, then restart this one.")
                                    .color(TEXT),
                            );
                        });
                    ui.add_space(6.0);
                }
                self.admin_banner(ui);
                self.update_banner(ui);
                egui::ScrollArea::vertical()
                    .id_salt("page")
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.page {
                        Page::Dashboard => self.page_dashboard(ui),
                        Page::Logs => self.page_logs(ui),
                        Page::Home => self.page_home(ui),
                        Page::Accounts => self.page_accounts(ui),
                        Page::Movement => self.page_movement(ui),
                        Page::Placement => self.page_placement(ui),
                        Page::Patterns => self.page_patterns(ui),
                        Page::Settings => self.page_settings(ui),
                    });
            });
        if self.page == Page::Dashboard {
            self.dash_dock_tick(ppp);
        } else {
            self.dash_release();
            self.dock_tick(ppp);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.control.stop();
        self.dash_release();
        for c in &self.clients {
            let _ = self.platform.set_topmost(c.hwnd, false);
        }
    }
}

pub fn run(
    platform: Arc<dyn Platform>,
    settings: Settings,
    settings_path: PathBuf,
) -> anyhow::Result<()> {
    let title = format!("{APP_NAME} {APP_BY}");
    let opts = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title(title.clone())
            .with_inner_size([1460.0, 940.0])
            .with_min_inner_size([980.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        &title,
        opts,
        Box::new(move |cc| {
            apply_theme(&cc.egui_ctx);
            Ok(Box::new(App::new(platform, settings, settings_path)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("window failed: {e}"))
}
