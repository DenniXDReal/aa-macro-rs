//! Anime Adventure Macro By DenniXD - the app window.
//!
//! Shell: top bar (status, Start/Pause/Stop, F1-F4), icon rail, pages.
//! Pages: Home (live view, mode, units, alts, session), Accounts (Roblox
//! Account Manager), Movement (routes), Placement, Patterns, Settings.

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

// ---- theme ----
const BG: Color32 = Color32::from_rgb(13, 10, 24);
const PANEL: Color32 = Color32::from_rgb(19, 15, 33);
const CARD: Color32 = Color32::from_rgb(27, 21, 46);
const CARD_HI: Color32 = Color32::from_rgb(36, 28, 62);
const LINE: Color32 = Color32::from_rgb(52, 42, 86);
const PURPLE: Color32 = Color32::from_rgb(139, 92, 246);
const PURPLE_DIM: Color32 = Color32::from_rgb(88, 62, 170);
const TEXT: Color32 = Color32::from_rgb(232, 228, 245);
const MUTED: Color32 = Color32::from_rgb(150, 142, 178);
const ACCENT: Color32 = PURPLE;
const GOOD: Color32 = Color32::from_rgb(74, 222, 128);
const WARN: Color32 = Color32::from_rgb(251, 191, 36);
const BAD: Color32 = Color32::from_rgb(248, 113, 113);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Accounts,
    Movement,
    Placement,
    Patterns,
    Settings,
}

const PAGES: [(Page, &str); 6] = [
    (Page::Home, "Home"),
    (Page::Accounts, "Accounts"),
    (Page::Movement, "Movement"),
    (Page::Placement, "Placement"),
    (Page::Patterns, "Patterns"),
    (Page::Settings, "Settings"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum HomeTab {
    Control,
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
    hits: Vec<(String, Rect, Option<Hit>)>,

    // placement
    map: String,
    new_map: String,
    place_slot: u8,

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
    ram_rx: Receiver<RamMsg>,
    ram_tx: Sender<RamMsg>,

    // run state (game loop arrives with your patterns)
    running: bool,
    paused: bool,
    started: Option<Instant>,
    wins: u32,
    losses: u32,
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
}

impl App {
    pub fn new(platform: Arc<dyn Platform>, settings: Settings, settings_path: PathBuf) -> Self {
        let lib = Library::load(&Library::default_dir()).unwrap_or_default();
        let placements_path = Placements::default_path();
        let placements = Placements::load(&placements_path).unwrap_or_default();
        let map = placements
            .maps
            .keys()
            .next()
            .cloned()
            .unwrap_or_else(|| "Space Center".into());
        let routes = Routes::load(&Routes::user_dir_default());
        let route_sel = "Space Center".to_string();
        let (log_tx, log_rx) = mpsc::channel();
        let (upd_tx, upd_rx) = mpsc::channel();
        let (ram_tx, ram_rx) = mpsc::channel();
        let (hk_tx, hotkeys) = mpsc::channel();
        #[cfg(windows)]
        crate::platform::win32::spawn_hotkeys(hk_tx);
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
            page: Page::Home,
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
            hits: vec![],
            map,
            new_map: String::new(),
            place_slot: 0,
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
            ram_rx,
            ram_tx,
            running: false,
            paused: false,
            started: None,
            wins: 0,
            losses: 0,
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
        app.refresh_clients();
        app.load_route_text();
        app.page = match std::env::var("AA_START_PAGE").as_deref() {
            Ok("accounts") => Page::Accounts,
            Ok("movement") => Page::Movement,
            Ok("placement") => Page::Placement,
            Ok("patterns") => Page::Patterns,
            Ok("settings") => Page::Settings,
            _ => Page::Home,
        };
        if app.settings.auto_update_check && app.secrets.has(update::TOKEN_KEY) {
            app.check_update();
        }
        app
    }

    // ---------------- run controls ----------------

    fn start(&mut self) {
        if self.clients.is_empty() {
            self.say("no Roblox clients - open Roblox first");
            return;
        }
        self.control.reset();
        self.running = true;
        self.paused = false;
        self.started.get_or_insert(Instant::now());
        self.say(format!(
            "Start: {} on client #{} - the game loop is switched on once your patterns are captured (Patterns page)",
            self.settings.run.mode.label(),
            self.main + 1
        ));
    }

    fn stop(&mut self) {
        self.control.stop();
        if self.running {
            self.say("Stopped");
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
            1 => self.tile(),
            2 => self.start(),
            3 => self.stop(),
            4 => self.toggle_pause(),
            _ => {}
        }
    }

    fn main_hwnd(&self) -> Option<WinInfo> {
        self.clients.get(self.main).cloned()
    }

    // ---------------- shell ----------------

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(APP_NAME).size(17.0).strong().color(TEXT));
                ui.label(
                    RichText::new(format!("{APP_BY} · {}", update::version_label()))
                        .size(11.0)
                        .color(MUTED),
                );
            });
            ui.add_space(18.0);
            let n = self.clients.len();
            chip(ui, "Roblox · 800×599 viewport", MUTED, None);
            chip(
                ui,
                &format!("{n} client{}", if n == 1 { "" } else { "s" }),
                TEXT,
                Some(if n > 0 { GOOD } else { BAD }),
            );
            if ui.button("Refresh").clicked() {
                self.refresh_clients();
            }
            if ui
                .button("Tile")
                .on_hover_text("F1 · every client to an 800×599 viewport, laid out on screen")
                .clicked()
            {
                self.tile();
            }
            if ui
                .button("Doctor")
                .on_hover_text("check scaling, size and capture of every client")
                .clicked()
            {
                self.doctor();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        let stop =
                            egui::Button::new(RichText::new("Stop").color(if self.running {
                                BAD
                            } else {
                                MUTED
                            }))
                            .min_size(Vec2::new(92.0, 30.0));
                        let pause = egui::Button::new(if self.paused { "Resume" } else { "Pause" })
                            .min_size(Vec2::new(92.0, 30.0));
                        let start = egui::Button::new(
                            RichText::new("Start").strong().color(Color32::WHITE),
                        )
                        .fill(if self.running { PURPLE_DIM } else { PURPLE })
                        .min_size(Vec2::new(120.0, 30.0));
                        // right-to-left: add in reverse
                        if ui.add(stop).clicked() {
                            self.stop();
                        }
                        if ui.add_enabled(self.running, pause).clicked() {
                            self.toggle_pause();
                        }
                        if ui.add_enabled(!self.running, start).clicked() {
                            self.start();
                        }
                    });
                    ui.label(
                        RichText::new("F1 fit Roblox   F2 start   F3 stop   F4 pause")
                            .size(11.0)
                            .color(MUTED),
                    );
                });
            });
        });
    }

    fn rail(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            for (p, name) in PAGES {
                let selected = self.page == p;
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(46.0, 46.0), Sense::click());
                if selected {
                    ui.painter().rect_filled(rect.shrink(2.0), 10.0, PURPLE_DIM);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect.shrink(2.0), 10.0, CARD_HI);
                }
                draw_icon(
                    ui.painter(),
                    rect,
                    p,
                    if selected { Color32::WHITE } else { MUTED },
                );
                if resp.on_hover_text(name).clicked() {
                    self.page = p;
                }
                ui.add_space(4.0);
            }
        });
    }

    // ---------------- Home ----------------

    fn page_home(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_width();
        let left_w = (avail * 0.58).max(420.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(left_w, 0.0),
                Layout::top_down(Align::Min),
                |ui| {
                    self.home_preview(ui);
                    ui.add_space(8.0);
                    ui.horizontal_top(|ui| {
                        let w = (ui.available_width() - 8.0) / 2.0;
                        ui.allocate_ui_with_layout(
                            Vec2::new(w, 0.0),
                            Layout::top_down(Align::Min),
                            |ui| self.session_card(ui),
                        );
                        ui.allocate_ui_with_layout(
                            Vec2::new(w, 0.0),
                            Layout::top_down(Align::Min),
                            |ui| self.quick_card(ui),
                        );
                    });
                },
            );
            ui.add_space(8.0);
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), 0.0),
                Layout::top_down(Align::Min),
                |ui| {
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
                            (HomeTab::Activity, "Activity"),
                        ],
                    );
                    ui.add_space(6.0);
                    match self.home_tab {
                        HomeTab::Control => self.units_grid(ui),
                        HomeTab::Activity => self.activity(ui),
                    }
                },
            );
        });
    }

    fn home_preview(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("MAIN").size(11.0).color(MUTED));
                let labels: Vec<String> = (0..self.clients.len())
                    .map(|i| client_name(i, &self.settings))
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
                            RichText::new(format!("{} · 800×599", client_name(*i, &self.settings)))
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
                self.say(format!(
                    "{} is now the main",
                    client_name(i, &self.settings)
                ));
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

    fn units_grid(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        let keys = self.settings.keybinds.units.clone();
        let avail = ui.available_width();
        let cols = if avail >= 3.0 * 200.0 + 16.0 { 3 } else { 2 };
        let w = (avail - 8.0 * (cols as f32 - 1.0)) / cols as f32 - 1.0;
        for row in 0..6usize.div_ceil(cols) {
            ui.horizontal_top(|ui| {
                for col in 0..cols {
                    let i = row * cols + col;
                    if i >= 6 {
                        break;
                    }
                    let key = keys.get(i).cloned().unwrap_or_default();
                    let u = &mut self.settings.units[i];
                    ui.allocate_ui_with_layout(
                        Vec2::new(w, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            egui::Frame::new()
                                .fill(if u.enabled { CARD_HI } else { CARD })
                                .stroke(Stroke::new(1.0, if u.enabled { PURPLE_DIM } else { LINE }))
                                .corner_radius(12)
                                .inner_margin(10)
                                .show(ui, |ui| {
                                    ui.set_width(w - 22.0);
                                    ui.horizontal(|ui| {
                                        changed |=
                                            toggle(ui, &mut u.enabled, &format!("Unit {}", i + 1));
                                        ui.with_layout(
                                            Layout::right_to_left(Align::Center),
                                            |ui| {
                                                ui.label(
                                                    RichText::new(format!("key {key}"))
                                                        .size(10.5)
                                                        .color(MUTED),
                                                );
                                            },
                                        );
                                    });
                                    changed |= ui
                                        .add(
                                            egui::TextEdit::singleline(&mut u.name)
                                                .hint_text("unnamed")
                                                .desired_width(f32::INFINITY),
                                        )
                                        .changed();
                                    ui.horizontal(|ui| {
                                        changed |= tag(ui, &mut u.farm, "FARM");
                                        changed |= tag(ui, &mut u.buff, "BUFF");
                                    });
                                    changed |= stepper(ui, "Placement", &mut u.place, 0, 10);
                                    changed |= stepper(ui, "Priority", &mut u.priority, 1, 6);
                                    let mut limit = matches!(u.upgrade, Upgrade::Level(_));
                                    ui.horizontal(|ui| {
                                        if toggle(ui, &mut limit, "Limit") {
                                            u.upgrade = if limit {
                                                Upgrade::Level(5)
                                            } else {
                                                Upgrade::Max
                                            };
                                            changed = true;
                                        }
                                        if let Upgrade::Level(n) = &mut u.upgrade {
                                            changed |= ui
                                                .add(egui::DragValue::new(n).range(1..=20))
                                                .changed();
                                        }
                                    });
                                    changed |=
                                        toggle(ui, &mut u.upg_immediately, "UPG immediately");
                                    let up = match u.upgrade {
                                        Upgrade::None => "no upgrades".to_string(),
                                        Upgrade::Max => "until MAX".to_string(),
                                        Upgrade::Level(n) => format!("to level {n}"),
                                    };
                                    ui.label(
                                        RichText::new(format!(
                                            "slot {} · {} · x{} · {up}",
                                            i + 1,
                                            if u.enabled { "on" } else { "off" },
                                            u.place
                                        ))
                                        .size(10.5)
                                        .color(MUTED),
                                    );
                                });
                        },
                    );
                }
            });
            ui.add_space(6.0);
        }
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
            "Roblox Account Manager launches your main + alts into your private server",
        );
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
                    if ui.button("+ Add").clicked() {
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
                        });
                        changed = true;
                    }
                });
            });
            let names = self.ram_list.clone();
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
                            if ui.small_button("Remove").clicked() {
                                remove = Some(i);
                            }
                        });
                        ui.end_row();
                    }
                });
            if let Some(i) = remove {
                self.settings.accounts.remove(i);
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
            self.ram_launch(list);
        }
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

    fn ram_launch(&mut self, names: Vec<String>) {
        let ram = self.ram();
        let link = self.secrets.get(accounts::PRIVATE_SERVER_KEY);
        let tx = self.ram_tx.clone();
        self.say(format!("launching {} account(s) through RAM…", names.len()));
        std::thread::spawn(move || {
            for (i, n) in names.iter().enumerate() {
                if i > 0 {
                    std::thread::sleep(Duration::from_secs(8)); // let each client get past the splash
                }
                let msg = match ram.launch(n, link.as_deref()) {
                    Ok(m) => format!("{n}: {m}"),
                    Err(e) => format!("{n}: launch failed - {e}"),
                };
                let _ = tx.send(RamMsg::Log(msg));
            }
            let _ = tx.send(RamMsg::Log(
                "launch done - press Refresh when the clients are in game, then Tile".into(),
            ));
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

    fn page_settings(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Settings", "");
        let mut changed = false;
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
        ui.add_space(8.0);
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
        if changed {
            self.dirty();
        }
        ui.add_space(8.0);
        card(ui, |ui| self.updates_section(ui));
        ui.add_space(8.0);
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
            }
        }
        while let Ok(k) = self.hotkeys.try_recv() {
            self.hotkey(k);
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
        ctx.request_repaint_after(Duration::from_millis(if wants_live || self.running {
            300
        } else {
            1000
        }));
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
        .inner_margin(12)
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

/// Simple line icons drawn with the painter (no icon font needed).
fn draw_icon(p: &egui::Painter, r: egui::Rect, page: Page, c: Color32) {
    use egui::pos2;
    let s = Stroke::new(1.8, c);
    let cx = r.center().x;
    let cy = r.center().y;
    match page {
        Page::Home => {
            p.line_segment([pos2(cx - 9.0, cy - 1.0), pos2(cx, cy - 9.0)], s);
            p.line_segment([pos2(cx, cy - 9.0), pos2(cx + 9.0, cy - 1.0)], s);
            p.rect_stroke(
                egui::Rect::from_min_max(pos2(cx - 6.5, cy - 3.0), pos2(cx + 6.5, cy + 8.0)),
                1.0,
                s,
                egui::StrokeKind::Middle,
            );
        }
        Page::Accounts => {
            p.circle_stroke(pos2(cx - 3.5, cy - 4.0), 3.5, s);
            p.circle_stroke(pos2(cx + 5.0, cy - 2.5), 2.8, s);
            p.line_segment([pos2(cx - 10.0, cy + 8.0), pos2(cx + 3.0, cy + 8.0)], s);
            p.line_segment([pos2(cx - 10.0, cy + 8.0), pos2(cx - 3.5, cy + 1.5)], s);
            p.line_segment([pos2(cx + 3.0, cy + 8.0), pos2(cx - 3.5, cy + 1.5)], s);
            p.line_segment([pos2(cx + 5.0, cy + 2.0), pos2(cx + 10.0, cy + 7.0)], s);
        }
        Page::Movement => {
            p.circle_stroke(pos2(cx, cy), 9.0, s);
            p.add(egui::Shape::convex_polygon(
                vec![pos2(cx, cy - 6.5), pos2(cx + 3.0, cy), pos2(cx - 3.0, cy)],
                c,
                Stroke::NONE,
            ));
            p.add(egui::Shape::convex_polygon(
                vec![pos2(cx - 3.0, cy), pos2(cx + 3.0, cy), pos2(cx, cy + 6.5)],
                Color32::TRANSPARENT,
                s,
            ));
        }
        Page::Placement => {
            p.circle_stroke(pos2(cx, cy - 3.0), 6.0, s);
            p.circle_filled(pos2(cx, cy - 3.0), 2.0, c);
            p.line_segment([pos2(cx - 4.2, cy + 1.3), pos2(cx, cy + 9.0)], s);
            p.line_segment([pos2(cx + 4.2, cy + 1.3), pos2(cx, cy + 9.0)], s);
        }
        Page::Patterns => {
            p.circle_stroke(pos2(cx - 2.0, cy - 2.0), 6.5, s);
            p.line_segment(
                [pos2(cx + 2.8, cy + 2.8), pos2(cx + 8.5, cy + 8.5)],
                Stroke::new(2.4, c),
            );
        }
        Page::Settings => {
            p.circle_stroke(pos2(cx, cy), 3.0, s);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::TAU / 8.0;
                let (sn, cs) = a.sin_cos();
                p.line_segment(
                    [
                        pos2(cx + cs * 5.5, cy + sn * 5.5),
                        pos2(cx + cs * 9.0, cy + sn * 9.0),
                    ],
                    Stroke::new(2.2, c),
                );
            }
            p.circle_stroke(pos2(cx, cy), 6.5, s);
        }
    }
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
fn tag(ui: &mut egui::Ui, on: &mut bool, label: &str) -> bool {
    let txt = RichText::new(label)
        .size(10.5)
        .color(if *on { Color32::WHITE } else { MUTED });
    let b = egui::Button::new(txt)
        .fill(if *on { PURPLE } else { CARD })
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(8);
    if ui.add(b).clicked() {
        *on = !*on;
        return true;
    }
    false
}

/// `label   –  n  +`
fn stepper(ui: &mut egui::Ui, label: &str, v: &mut u8, min: u8, max: u8) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(MUTED));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("+").clicked() && *v < max {
                *v += 1;
                changed = true;
            }
            ui.label(RichText::new(v.to_string()).strong().color(TEXT));
            if ui.small_button("–").clicked() && *v > min {
                *v -= 1;
                changed = true;
            }
        });
    });
    changed
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
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(10);
    let r = CornerRadius::same(8);
    v.widgets.noninteractive.bg_fill = CARD;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.bg_fill = CARD_HI;
    v.widgets.inactive.weak_bg_fill = CARD_HI;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.bg_fill = Color32::from_rgb(50, 39, 86);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(50, 39, 86);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, PURPLE_DIM);
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.bg_fill = PURPLE_DIM;
    v.widgets.active.weak_bg_fill = PURPLE_DIM;
    v.widgets.active.corner_radius = r;
    v.widgets.open.bg_fill = CARD_HI;
    v.widgets.open.corner_radius = r;
    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 7.0);
        s.spacing.button_padding = Vec2::new(10.0, 5.0);
    });
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
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
        egui::Panel::left("rail")
            .exact_size(62.0)
            .frame(egui::Frame::new().fill(BG))
            .show(ui, |ui| {
                self.rail(ui);
            });
        egui::CentralPanel::default_margins()
            .frame(egui::Frame::new().fill(PANEL).inner_margin(12))
            .show(ui, |ui| {
                self.update_banner(ui);
                egui::ScrollArea::vertical()
                    .id_salt("page")
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.page {
                        Page::Home => self.page_home(ui),
                        Page::Accounts => self.page_accounts(ui),
                        Page::Movement => self.page_movement(ui),
                        Page::Placement => self.page_placement(ui),
                        Page::Patterns => self.page_patterns(ui),
                        Page::Settings => self.page_settings(ui),
                    });
            });
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
            .with_inner_size([1320.0, 860.0])
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
