//! Command Center: the Dashboard (2x2 client cards with the Roblox windows
//! docked into them), the terminal-style log dock and the Logs page.
//!
//! Docking = "window overlay": the cards draw empty slots, and every tick the
//! Roblox window is moved (`SetWindowPos`) exactly over its slot, borders
//! stripped (`WS_POPUP`). No `SetParent` - the window stays a normal
//! top-level window, so DirectX keeps drawing and capture keeps working.
//!
//! Real macro state only: step + retry attempts from `crate::status`
//! (`ArcSwap`, written by the macro threads), input generation per client
//! from `input::generation`, log lines from the app log (the macro sends
//! them over a channel). Nothing here blocks the macro.

use super::{App, Page, BAD, BG, CARD, GOOD, MUTED, PURPLE, TEXT, WARN};
use crate::geom::Hwnd;
use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2};
use std::time::{Duration, Instant};

/// Cards on the dashboard (2x2).
pub const CARDS: usize = 4;
/// How often docked windows are re-checked / moved to follow their slots.
const DOCK_EVERY: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Health {
    /// Macro not running.
    #[default]
    Idle,
    Ok,
    Retrying,
    Paused,
}

impl Health {
    fn color(self) -> Color32 {
        match self {
            Health::Idle => MUTED,
            Health::Ok => GOOD,
            Health::Retrying => WARN,
            Health::Paused => BAD,
        }
    }
}

/// One client's card.
#[derive(Clone, Debug, Default)]
pub struct ClientView {
    pub step: String,
    /// Input generation of that client (0 = no input sent yet).
    pub gen: u64,
    /// Current try (1-based) and how many there are; None = nothing retried.
    pub attempt: Option<(u32, u32)>,
    pub health: Health,
}

/// A retry shown on its card for this long after its last try.
const ATTEMPT_SHOWN: Duration = Duration::from_secs(10);

impl ClientView {
    /// The card of client `h` from the live state.
    fn build(st: &crate::status::Status, h: Option<Hwnd>, running: bool, paused: bool) -> Self {
        let gen = h.map(crate::input::generation).unwrap_or(0);
        if !running {
            return ClientView {
                step: "IDLE".into(),
                gen,
                attempt: None,
                health: Health::Idle,
            };
        }
        // the watchdog's finding (captcha / disconnected) beats the step
        if let Some(w) = h.and_then(|h| st.incident(h)) {
            return ClientView {
                step: w.into(),
                gen,
                attempt: None,
                health: Health::Paused,
            };
        }
        let att = h
            .and_then(|h| st.attempt(h))
            .filter(|a| a.at.elapsed() < ATTEMPT_SHOWN);
        let health = if paused {
            Health::Paused
        } else if att.is_some_and(|a| a.n > 1) {
            Health::Retrying
        } else {
            Health::Ok
        };
        ClientView {
            // started, first step not reached yet: it begins in the lobby
            step: if st.step.is_empty() {
                "LOBBY".into()
            } else {
                st.step.to_uppercase()
            },
            gen,
            attempt: att.map(|a| (a.n, a.of)),
            health,
        }
    }
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

/// Dashboard state kept in the App.
pub struct Dash {
    /// This frame's slots: (client, slot rect in UI points, fully on show).
    slots: Vec<(Hwnd, egui::Rect, bool)>,
    /// Clients whose borders we stripped (restored on release).
    borderless: Vec<Hwnd>,
    /// Clients we made topmost, and when that was last re-asserted.
    tops: Vec<Hwnd>,
    tops_at: Instant,
    last: Instant,
    pub log_open: bool,
    log_filter: Level,
    /// A docking problem to show on the dashboard.
    note: String,
}

impl Default for Dash {
    fn default() -> Self {
        Dash {
            slots: vec![],
            borderless: vec![],
            tops: vec![],
            tops_at: Instant::now(),
            last: Instant::now() - DOCK_EVERY,
            log_open: true,
            log_filter: Level::Info,
            note: String::new(),
        }
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
        let st = crate::status::snapshot();
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
            let (txt, c) = if !self.running {
                ("Stopped - press START ALL (F2)", MUTED)
            } else if self.paused {
                ("Paused", WARN)
            } else {
                ("Running", GOOD)
            };
            super::chip(ui, txt, TEXT, Some(c));
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
                    let view = ClientView::build(&st, client, self.running, self.paused);
                    let health = view.health;
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
                                    // the step up here too: always on show,
                                    // even when the card's bottom is below the fold
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                RichText::new(&view.step)
                                                    .monospace()
                                                    .size(12.0)
                                                    .strong()
                                                    .color(health.color()),
                                            );
                                        },
                                    );
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
                                                Health::Idle => ("IDLE", MUTED),
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
                                        "Gen: {}  |  Attempt: {}",
                                        view.gen,
                                        view.attempt
                                            .map(|(n, of)| if of > 0 {
                                                format!("{n}/{of}")
                                            } else {
                                                format!("{n}")
                                            })
                                            .unwrap_or_else(|| "-".into())
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

    /// Exactly `want` topmost: only what changed, plus a re-assert every 2 s
    /// (a focus change can bump it). Never activates a window, so it doesn't
    /// fight the macro focusing the client it's sending input to.
    fn dash_tops(&mut self, want: Vec<Hwnd>) {
        let reassert = self.dash.tops_at.elapsed() > Duration::from_secs(2);
        for h in self.dash.tops.clone() {
            if !want.contains(&h) {
                let _ = self.platform.set_topmost(h, false);
            }
        }
        for h in &want {
            if reassert || !self.dash.tops.contains(h) {
                let _ = self.platform.set_topmost(*h, true);
            }
        }
        if reassert {
            self.dash.tops_at = Instant::now();
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

    /// App log line "[HH:MM:SS] text" -> (time, text).
    fn split_line(l: &str) -> (&str, &str) {
        l.strip_prefix('[')
            .and_then(|r| r.split_once("] "))
            .unwrap_or(("", l))
    }

    fn log_lines(&self, ui: &mut egui::Ui, min: Level) {
        egui::ScrollArea::vertical()
            .id_salt(("logscroll", min as u8))
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                if self.log.is_empty() {
                    ui.label(
                        RichText::new("no log lines yet")
                            .monospace()
                            .size(12.0)
                            .color(MUTED),
                    );
                }
                for l in &self.log {
                    let lv = Level::of(l);
                    if lv < min {
                        continue;
                    }
                    let (at, text) = Self::split_line(l);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(at).monospace().size(12.0).color(MUTED));
                        ui.label(
                            RichText::new(lv.tag())
                                .monospace()
                                .size(12.0)
                                .color(lv.color()),
                        );
                        ui.label(
                            RichText::new(text)
                                .monospace()
                                .size(12.0)
                                .color(if lv == Level::Info { TEXT } else { lv.color() }),
                        );
                    });
                }
            });
    }

    /// Bottom panel: one line when collapsed, a terminal when open.
    pub(super) fn log_dock(&mut self, ui: &mut egui::Ui) {
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
                if let Some(l) = self.log.last() {
                    ui.label(
                        RichText::new(l.as_str())
                            .monospace()
                            .size(12.0)
                            .color(Level::of(l).color()),
                    );
                }
            }
        });
        if self.dash.log_open {
            ui.allocate_ui(Vec2::new(ui.available_width(), 150.0), |ui| {
                self.log_lines(ui, Level::Info)
            });
        }
    }

    pub(super) fn page_logs(&mut self, ui: &mut egui::Ui) {
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
                self.log_lines(ui, min)
            });
    }
}

/// Sidebar entries: (page, label). The first five are the Command Center;
/// the rest are the setup pages.
pub(super) const NAV: [(Page, &str); 5] = [
    (Page::Dashboard, "📊  Dashboard"),
    (Page::Patterns, "🖼  Patterns"),
    (Page::Movement, "🗺  Movement"),
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
    fn card_from_live_state() {
        let h = Hwnd(0xCA4D);
        let st = crate::status::Status {
            step: "upgrade".into(),
            step_since: Some(Instant::now()),
            attempts: vec![crate::status::Attempt {
                hwnd: h,
                n: 2,
                of: 4,
                at: Instant::now(),
            }],
            incidents: vec![(Hwnd(0xBAD), "CAPTCHA")],
        };
        let idle = ClientView::build(&st, Some(h), false, false);
        let fresh = ClientView::build(&crate::status::Status::default(), Some(h), true, false);
        assert_eq!(fresh.step, "LOBBY");
        assert_eq!((idle.step.as_str(), idle.health), ("IDLE", Health::Idle));
        let v = ClientView::build(&st, Some(h), true, false);
        assert_eq!(v.step, "UPGRADE");
        assert_eq!(v.attempt, Some((2, 4)));
        assert_eq!(v.health, Health::Retrying);
        let p = ClientView::build(&st, Some(h), true, true);
        assert_eq!(p.health, Health::Paused);
        // the watchdog's finding shows instead of the step
        let cap = ClientView::build(&st, Some(Hwnd(0xBAD)), true, false);
        assert_eq!((cap.step.as_str(), cap.health), ("CAPTCHA", Health::Paused));
        let other = ClientView::build(&st, Some(Hwnd(1)), true, false);
        assert_eq!((other.attempt, other.health), (None, Health::Ok));
    }

    #[test]
    fn log_line_split() {
        assert_eq!(
            App::split_line("[21:04:10] slot 2 upgraded to 5."),
            ("21:04:10", "slot 2 upgraded to 5.")
        );
        assert_eq!(App::split_line("plain"), ("", "plain"));
    }
}
