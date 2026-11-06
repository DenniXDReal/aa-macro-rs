//! Command Center: the Dashboard (2x2 client cards with the Roblox windows
//! docked into them), the terminal-style log dock and the Logs page.
//!
//! Docking = "window overlay": the cards draw empty slots, and every tick the
//! Roblox window is moved (`SetWindowPos`) exactly over its slot, borders
//! stripped (`WS_POPUP`). No `SetParent` - the window stays a normal
//! top-level window, so DirectX keeps drawing and capture keeps working.
//!
//! Macro state reaches the UI through [`Bus`] (`ArcSwap`): the macro threads
//! publish a snapshot, the UI just loads the latest - it never blocks them.
//! For now a demo feeder publishes dummy data (wired to the real macro next).

use super::{App, Page, BAD, BG, CARD, GOOD, MUTED, PURPLE, TEXT, WARN};
use crate::geom::Hwnd;
use arc_swap::ArcSwap;
use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Cards on the dashboard (2x2).
pub const CARDS: usize = 4;
/// Log lines kept in the snapshot.
pub const LOG_KEEP: usize = 500;
/// How often docked windows are re-checked / moved to follow their slots.
const DOCK_EVERY: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Health {
    #[default]
    Ok,
    Retrying,
    Paused,
}

impl Health {
    fn color(self) -> Color32 {
        match self {
            Health::Ok => GOOD,
            Health::Retrying => WARN,
            Health::Paused => BAD,
        }
    }
}

/// One client's card, as the macro last published it.
#[derive(Clone, Debug, Default)]
pub struct ClientView {
    pub step: String,
    /// Input generation of that client.
    pub gen: u64,
    pub attempt: u32,
    pub attempts: u32,
    pub health: Health,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Info,
    Warn,
    Error,
    Fatal,
}

impl Level {
    /// Level of a plain log line by its words (FATAL / ERROR / WARN).
    pub fn of(text: &str) -> Level {
        let t = text.to_ascii_uppercase();
        if t.contains("FATAL") {
            Level::Fatal
        } else if t.contains("ERROR") || t.contains("FAILED") {
            Level::Error
        } else if t.contains("WARN") {
            Level::Warn
        } else {
            Level::Info
        }
    }
    fn color(self) -> Color32 {
        match self {
            Level::Info => Color32::from_rgb(200, 200, 215),
            Level::Warn => WARN,
            Level::Error => BAD,
            Level::Fatal => Color32::from_rgb(255, 70, 160),
        }
    }
    fn tag(self) -> &'static str {
        match self {
            Level::Info => "INFO ",
            Level::Warn => "WARN ",
            Level::Error => "ERROR",
            Level::Fatal => "FATAL",
        }
    }
}

#[derive(Clone, Debug)]
pub struct LogLine {
    /// "HH:MM:SS"
    pub at: String,
    pub level: Level,
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub clients: Vec<ClientView>,
    pub logs: Vec<LogLine>,
    /// Demo data (not from the macro yet).
    pub demo: bool,
}

/// Macro -> UI state. Cheap to clone; `load` never blocks a writer.
#[derive(Clone, Default)]
pub struct Bus(Arc<ArcSwap<Snapshot>>);

impl Bus {
    pub fn load(&self) -> Arc<Snapshot> {
        self.0.load_full()
    }
    /// Replace one client's card (grows the list as needed).
    pub fn set_client(&self, i: usize, v: ClientView) {
        self.0.rcu(|s| {
            let mut s = (**s).clone();
            if s.clients.len() <= i {
                s.clients.resize(i + 1, ClientView::default());
            }
            s.clients[i] = v.clone();
            s
        });
    }
    /// Add a log line (keeps the last [`LOG_KEEP`]).
    pub fn log(&self, at: String, text: String) {
        let line = LogLine {
            at,
            level: Level::of(&text),
            text,
        };
        self.0.rcu(|s| {
            let mut s = (**s).clone();
            s.logs.push(line.clone());
            let n = s.logs.len();
            if n > LOG_KEEP {
                s.logs.drain(..n - LOG_KEEP);
            }
            s
        });
    }
    fn set_demo(&self, on: bool) {
        self.0.rcu(|s| Snapshot {
            demo: on,
            ..(**s).clone()
        });
    }
}

fn clock() -> String {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // local time is wired with the real data; demo shows UTC+8 (SG)
    let s = (s + 8 * 3600) % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// Dummy feeder: fake step / gen / attempt for 4 clients + log lines.
fn spawn_demo(bus: Bus, stop: Arc<AtomicBool>) {
    bus.set_demo(true);
    std::thread::Builder::new()
        .name("dashboard-demo".into())
        .spawn(move || {
            const STEPS: [&str; 6] = ["LOBBY", "WALK", "VOTE", "PLACE", "UPGRADE", "END"];
            let mut t: u64 = 0;
            let mut gens = [284u64, 368, 120, 77];
            while !stop.load(Ordering::Relaxed) {
                for (i, g) in gens.iter_mut().enumerate() {
                    *g += 1 + (t + i as u64) % 3;
                    let step = STEPS[((t / 12 + i as u64) % STEPS.len() as u64) as usize];
                    let attempt = 1 + ((t / 4 + i as u64) % 3) as u32;
                    let health = match (t / 8 + i as u64) % 7 {
                        5 => Health::Retrying,
                        6 if i == 3 => Health::Paused,
                        _ => Health::Ok,
                    };
                    bus.set_client(
                        i,
                        ClientView {
                            step: step.into(),
                            gen: *g,
                            attempt,
                            attempts: 3,
                            health,
                        },
                    );
                }
                if t.is_multiple_of(3) {
                    let c = 1 + (t / 3) % 2;
                    let text = match (t / 3) % 9 {
                        4 => {
                            format!("Client {c}: WARN panel didn't open, clicking unit again (1/3)")
                        }
                        7 => {
                            format!("Client {c}: ERROR input dispatch failed (error 5) - retrying")
                        }
                        8 if t.is_multiple_of(2) => format!("Client {c}: FATAL demo fatal line"),
                        _ => format!("Client {c}: slot {} upgraded to {}.", 1 + t % 6, 1 + t % 9),
                    };
                    bus.log(clock(), text);
                }
                t += 1;
                std::thread::sleep(Duration::from_millis(700));
            }
        })
        .ok();
}

/// Dashboard state kept in the App.
pub struct Dash {
    pub bus: Bus,
    demo_stop: Arc<AtomicBool>,
    /// This frame's slots: (client, slot rect in UI points, fully on show).
    slots: Vec<(Hwnd, egui::Rect, bool)>,
    /// Clients whose borders we stripped (restored on release).
    borderless: Vec<Hwnd>,
    /// Clients we made topmost.
    tops: Vec<Hwnd>,
    last: Instant,
    pub log_open: bool,
    log_filter: Level,
    /// A docking problem to show on the dashboard.
    note: String,
}

impl Default for Dash {
    fn default() -> Self {
        let bus = Bus::default();
        let demo_stop = Arc::new(AtomicBool::new(false));
        spawn_demo(bus.clone(), demo_stop.clone());
        Dash {
            bus,
            demo_stop,
            slots: vec![],
            borderless: vec![],
            tops: vec![],
            last: Instant::now() - DOCK_EVERY,
            log_open: true,
            log_filter: Level::Info,
            note: String::new(),
        }
    }
}

impl Drop for Dash {
    fn drop(&mut self) {
        self.demo_stop.store(true, Ordering::Relaxed);
    }
}

/// Soft coloured glow around `r`.
fn glow(p: &egui::Painter, r: egui::Rect, c: Color32) {
    for (k, a) in [(1.0f32, 230u8), (3.0, 90), (6.0, 40), (10.0, 16)] {
        p.rect_stroke(
            r.expand(k),
            14.0 + k,
            Stroke::new(
                if k == 1.0 { 1.6 } else { 3.0 },
                c.gamma_multiply(a as f32 / 255.0),
            ),
            egui::StrokeKind::Outside,
        );
    }
}

impl App {
    // ---------------- Dashboard ----------------

    pub(super) fn page_dashboard(&mut self, ui: &mut egui::Ui) {
        let snap = self.dash.bus.load();
        let ppp = ui.ctx().pixels_per_point();
        // the slot is the client's exact pixel size: the macro's coordinates
        // are for an 800x599 viewport, so the window is never scaled
        let slot = Vec2::new(
            self.settings.client_w as f32 / ppp,
            self.settings.client_h as f32 / ppp,
        );
        let pad = 10.0;
        let gap = 18.0;
        let card_w = slot.x + pad * 2.0;
        let cols = if ui.available_width() >= card_w * 2.0 + gap {
            2
        } else {
            1
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new("Dashboard").size(18.0).strong().color(TEXT));
            if snap.demo {
                super::chip(ui, "DEMO DATA", TEXT, Some(WARN));
            }
            if !self.settings.dock {
                super::chip(ui, "docking off - Window ▸ Dock Roblox", TEXT, Some(MUTED));
            }
            if !self.dash.note.is_empty() {
                ui.label(RichText::new(&self.dash.note).size(11.0).color(WARN));
            }
        });
        ui.add_space(10.0);
        let clip = ui.clip_rect();
        let mut slots = vec![];
        let n = CARDS.max(self.clients.len().min(CARDS));
        for row in 0..n.div_ceil(cols) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for col in 0..cols {
                    let i = row * cols + col;
                    if i >= n {
                        break;
                    }
                    let client = self.clients.get(i).map(|c| c.hwnd);
                    let view = snap.clients.get(i).cloned().unwrap_or_default();
                    let health = if self.paused {
                        Health::Paused
                    } else {
                        view.health
                    };
                    let label = if client.is_some() {
                        self.client_label(i)
                    } else {
                        "empty".into()
                    };
                    let resp = egui::Frame::new()
                        .fill(CARD)
                        .corner_radius(14)
                        .inner_margin(pad)
                        .show(ui, |ui| {
                            ui.set_width(slot.x);
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("CLIENT {}", i + 1))
                                            .size(12.0)
                                            .strong()
                                            .color(TEXT),
                                    );
                                    ui.label(RichText::new(label).size(11.0).color(MUTED));
                                });
                                ui.add_space(4.0);
                                // top half: the docking slot
                                let (r, _) = ui.allocate_exact_size(slot, Sense::hover());
                                ui.painter().rect_filled(r, 8.0, BG);
                                ui.painter().text(
                                    r.center(),
                                    egui::Align2::CENTER_CENTER,
                                    if client.is_none() {
                                        "No Roblox client"
                                    } else if self.settings.dock {
                                        "Roblox docks here"
                                    } else {
                                        "Live game preview (dock to show)"
                                    },
                                    egui::FontId::proportional(14.0),
                                    MUTED,
                                );
                                if let Some(h) = client {
                                    slots.push((h, r, clip.contains_rect(r)));
                                }
                                ui.add_space(6.0);
                                // bottom half: status
                                let mono = |s: String, c: Color32| {
                                    RichText::new(s).monospace().size(13.0).color(c)
                                };
                                ui.horizontal(|ui| {
                                    ui.label(mono("Step:".into(), MUTED));
                                    ui.label(
                                        mono(
                                            if view.step.is_empty() {
                                                "-".into()
                                            } else {
                                                view.step.clone()
                                            },
                                            TEXT,
                                        )
                                        .strong(),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let (txt, c) = match health {
                                                Health::Ok => ("OK", GOOD),
                                                Health::Retrying => ("RETRYING", WARN),
                                                Health::Paused => ("PAUSED", BAD),
                                            };
                                            ui.label(mono(txt.into(), c).strong());
                                        },
                                    );
                                });
                                ui.label(mono(
                                    format!(
                                        "Gen: {}  |  Attempt: {}/{}",
                                        view.gen,
                                        view.attempt.max(1),
                                        view.attempts.max(1)
                                    ),
                                    TEXT,
                                ));
                            });
                        });
                    glow(ui.painter(), resp.response.rect, health.color());
                }
            });
            ui.add_space(gap);
        }
        self.dash.slots = slots;
    }

    // ---------------- docking ----------------

    /// Keep every docked client exactly over its card's slot (100 ms).
    /// Slots scrolled out of view: that client waits behind the app.
    pub(super) fn dash_dock_tick(&mut self, ppp: f32) {
        if !self.settings.dock {
            self.dash_release();
            return;
        }
        if self.dash.last.elapsed() < DOCK_EVERY {
            return;
        }
        self.dash.last = Instant::now();
        self.set_top(None); // the old single-window dock's topmost
        let p = self.platform.clone();
        if self.own_hwnd.is_none() {
            self.own_hwnd = p.own_window();
        }
        let Some(own) = self.own_hwnd else { return };
        if self.admin_needed {
            self.dash.note = "can't dock: Roblox runs as administrator".into();
            return;
        }
        if p.is_minimized(own) {
            self.dash_tops(vec![]);
            return;
        }
        let Some(app) = p.geometry(own) else { return };
        let fg = p.foreground();
        let clients: Vec<Hwnd> = self.clients.iter().map(|c| c.hwnd).collect();
        let ours = fg == Some(own) || fg.is_some_and(|f| clients.contains(&f));
        let (cw, ch) = (self.settings.client_w, self.settings.client_h);
        let mut on_show = vec![];
        let mut off = 0;
        for (h, r, vis) in self.dash.slots.clone() {
            if !clients.contains(&h) {
                continue;
            }
            if !self.dash.borderless.contains(&h) {
                if p.set_borderless(h, true).is_ok() {
                    self.dash.borderless.push(h);
                }
            } else {
                // Roblox may put its frame back (e.g. after alt-enter)
                let _ = p.set_borderless(h, true);
            }
            if !vis {
                let _ = p.place_below(h, own);
                continue;
            }
            let mut target = crate::layout::ui_to_screen(
                crate::geom::Point::new(app.client.x, app.client.y),
                r.min.x,
                r.min.y,
                ppp,
            );
            target.x += self.settings.dock_dx;
            target.y += self.settings.dock_dy;
            let Some(g) = p.geometry(h) else { continue };
            if (g.client.x, g.client.y, g.client.w, g.client.h) != (target.x, target.y, cw, ch) {
                let _ = p.move_resize(h, crate::layout::outer_for_client_at(&g, target, cw, ch));
                if p.geometry(h)
                    .is_some_and(|g| (g.client.x, g.client.y) != (target.x, target.y))
                {
                    off += 1;
                }
            }
            on_show.push(h);
        }
        self.dash.note = if off > 0 {
            format!("{off} client(s) didn't move into place (Roblox fullscreen / minimum size?)")
        } else {
            String::new()
        };
        if ours {
            self.dash_tops(on_show);
        } else {
            self.dash_tops(vec![]);
            if let Some(f) = fg {
                // another app in front: tuck Roblox behind it
                for h in &on_show {
                    let _ = p.place_below(*h, f);
                }
            }
        }
    }

    /// Exactly `want` topmost (only touching what changed).
    fn dash_tops(&mut self, want: Vec<Hwnd>) {
        for h in self.dash.tops.clone() {
            if !want.contains(&h) {
                let _ = self.platform.set_topmost(h, false);
            }
        }
        for h in &want {
            // re-assert: a focus change can bump it
            let _ = self.platform.set_topmost(*h, true);
        }
        self.dash.tops = want;
    }

    /// Leave the dashboard dock: frames back, nothing topmost.
    pub(super) fn dash_release(&mut self) {
        if self.dash.tops.is_empty() && self.dash.borderless.is_empty() {
            return;
        }
        self.dash_tops(vec![]);
        for h in std::mem::take(&mut self.dash.borderless) {
            let _ = self.platform.set_borderless(h, false);
        }
        self.dash.slots.clear();
    }

    // ---------------- logs ----------------

    fn log_lines(&self, ui: &mut egui::Ui, snap: &super::command::Snapshot, min: Level) {
        egui::ScrollArea::vertical()
            .id_salt(("logscroll", min as u8))
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                for l in snap.logs.iter().filter(|l| l.level >= min) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&l.at).monospace().size(12.0).color(MUTED));
                        ui.label(
                            RichText::new(l.level.tag())
                                .monospace()
                                .size(12.0)
                                .color(l.level.color()),
                        );
                        ui.label(RichText::new(&l.text).monospace().size(12.0).color(
                            if l.level == Level::Info {
                                TEXT
                            } else {
                                l.level.color()
                            },
                        ));
                    });
                }
            });
    }

    /// Bottom panel: one line when collapsed, a terminal when open.
    pub(super) fn log_dock(&mut self, ui: &mut egui::Ui) {
        let snap = self.dash.bus.load();
        ui.horizontal(|ui| {
            let arrow = if self.dash.log_open { "▾" } else { "▸" };
            if ui
                .add(
                    egui::Button::new(
                        RichText::new(format!("{arrow} LOG"))
                            .monospace()
                            .strong()
                            .color(PURPLE),
                    )
                    .frame(false),
                )
                .clicked()
            {
                self.dash.log_open = !self.dash.log_open;
            }
            if !self.dash.log_open {
                if let Some(l) = snap.logs.last() {
                    ui.label(
                        RichText::new(format!("[{}] {}", l.at, l.text))
                            .monospace()
                            .size(12.0)
                            .color(l.level.color()),
                    );
                }
            } else if snap.demo {
                super::chip(ui, "DEMO", TEXT, Some(WARN));
            }
        });
        if self.dash.log_open {
            ui.allocate_ui(Vec2::new(ui.available_width(), 150.0), |ui| {
                self.log_lines(ui, &snap, Level::Info)
            });
        }
    }

    pub(super) fn page_logs(&mut self, ui: &mut egui::Ui) {
        let snap = self.dash.bus.load();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Logs").size(18.0).strong().color(TEXT));
            ui.add_space(12.0);
            for (l, name) in [
                (Level::Info, "All"),
                (Level::Warn, "WARN+"),
                (Level::Error, "ERROR+"),
                (Level::Fatal, "FATAL"),
            ] {
                ui.selectable_value(&mut self.dash.log_filter, l, name);
            }
        });
        ui.add_space(6.0);
        let min = self.dash.log_filter;
        egui::Frame::new()
            .fill(BG)
            .corner_radius(10)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_min_height(ui.available_height().max(300.0));
                self.log_lines(ui, &snap, min)
            });
    }
}

/// Sidebar entries: (page, label). The first five are the Command Center;
/// the rest are the setup pages.
pub(super) const NAV: [(Page, &str); 5] = [
    (Page::Dashboard, "📊  Dashboard"),
    (Page::Patterns, "🖼  Patterns"),
    (Page::Movement, "🗺  Routes"),
    (Page::Settings, "⚙  Settings"),
    (Page::Logs, "📝  Logs"),
];
pub(super) const NAV_SETUP: [(Page, &str); 3] = [
    (Page::Home, "▶  Run setup"),
    (Page::Accounts, "👤  Accounts"),
    (Page::Placement, "📍  Placement"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_from_words() {
        assert_eq!(Level::of("slot 2 upgraded to 5."), Level::Info);
        assert_eq!(Level::of("WARN panel didn't open"), Level::Warn);
        assert_eq!(Level::of("upgrade: Error 5"), Level::Error);
        assert_eq!(Level::of("Fatal: input"), Level::Fatal);
    }

    #[test]
    fn bus_keeps_last_lines_and_grows_clients() {
        let b = Bus::default();
        for i in 0..LOG_KEEP + 20 {
            b.log("00:00:00".into(), format!("line {i}"));
        }
        let s = b.load();
        assert_eq!(s.logs.len(), LOG_KEEP);
        assert_eq!(s.logs[0].text, "line 20");
        b.set_client(
            2,
            ClientView {
                step: "UPGRADE".into(),
                ..Default::default()
            },
        );
        let s = b.load();
        assert_eq!(s.clients.len(), 3);
        assert_eq!(s.clients[2].step, "UPGRADE");
    }
}
