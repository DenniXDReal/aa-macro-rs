//! TEMPORARY debug tool for Teleport to Spawn (remove once it works):
//! a card on the Patterns page to scan the 4 TP patterns without clicking,
//! show where the click would land, and run the TP step by step with a
//! log line + screenshot per step (tp-debug-*.png next to the exe).

use super::*;
use crate::geom::{Point, Rect};
use crate::input::{Action, Batch};
use crate::vision;
use eframe::egui;

const NAMES: [&str; 4] = [
    "settings_button",
    "tp_spawn",
    "tp_spawn_button",
    "settings_close",
];

impl App {
    pub(super) fn debug_tp_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(
                RichText::new("TP SPAWN DEBUG (temporary)")
                    .size(11.0)
                    .color(WARN),
            );
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button("Scan (no clicks)")
                    .on_hover_text(
                        "open Settings in Roblox first, scrolled to Teleport back to Spawn",
                    )
                    .clicked()
                {
                    self.debug_tp_scan();
                }
                if ui
                    .button("Run Teleport to Spawn now")
                    .on_hover_text(
                        "in a stage: does the whole TP on the selected client, logging every step",
                    )
                    .clicked()
                {
                    self.debug_tp_run();
                }
            });
            ui.label(
                RichText::new("Scan: yellow = where it looks, green = found, red cross = where it would click. Run: every step goes to Activity with a screenshot tp-debug-N.png next to the exe.")
                    .size(11.0)
                    .color(MUTED),
            );
        });
        ui.add_space(6.0);
    }

    fn debug_tp_scan(&mut self) {
        let Some(info) = self.clients.get(self.sel).cloned() else {
            self.say("TP debug: no client");
            return;
        };
        let f = match self.platform.capture_client(info.hwnd) {
            Ok(f) => f,
            Err(e) => return self.say(format!("TP debug: capture failed - {e:#}")),
        };
        let mut label = None;
        let mut button = None;
        for n in NAMES {
            let Some(p) = self.lib.get(n) else {
                self.say(format!("TP debug: {n} - not captured"));
                continue;
            };
            let hit = p.find_in(&f, None);
            let s = p.search;
            self.hits.retain(|h| vision::key(&h.0) != vision::key(n));
            self.hits.push((n.to_string(), s, hit));
            match hit {
                Some(h) => {
                    self.say(format!(
                        "TP debug: {n} FOUND at {},{} size {}x{} score {:.2} (looks in {},{} {}x{})",
                        h.at.x, h.at.y, h.w, h.h, h.score, s.x, s.y, s.w, s.h
                    ));
                    if n == "tp_spawn" {
                        label = Some(h.center());
                    }
                    if n == "tp_spawn_button" {
                        button = Some(h.center());
                    }
                }
                None => self.say(format!(
                    "TP debug: {n} not found (looks in {},{} {}x{}) - re-capture it, or it isn't on screen",
                    s.x, s.y, s.w, s.h
                )),
            }
        }
        self.tp_dbg_click = match (button, label) {
            (Some(b), _) => {
                self.say(format!(
                    "TP debug: would click tp_spawn_button at {},{}",
                    b.x, b.y
                ));
                Some(b)
            }
            (None, Some(l)) => {
                let c = Point::new(l.x + 178, l.y + 7);
                self.say(format!(
                    "TP debug: no tp_spawn_button - would click label +178,+7 = {},{}",
                    c.x, c.y
                ));
                Some(c)
            }
            _ => {
                self.say("TP debug: can't click anything - tp_spawn not found");
                None
            }
        };
        self.frame = Some(f);
    }

    fn debug_tp_run(&mut self) {
        let Some(info) = self.clients.get(self.sel).cloned() else {
            self.say("TP debug: no client");
            return;
        };
        let scr = crate::game::LiveScreen::new(self.platform.clone(), info.hwnd, &self.lib);
        let arb = self.arbiter();
        let tx = self.log_tx.clone();
        let h = info.hwnd;
        let dir = Library::default_dir()
            .parent()
            .map(|d| d.to_path_buf())
            .unwrap_or_default();
        std::thread::spawn(move || {
            let step = std::cell::Cell::new(0);
            let say = |s: String| {
                let n = step.get() + 1;
                step.set(n);
                if let Some(f) = scr.frame() {
                    let _ = f.save_png(&dir.join(format!("tp-debug-{n}.png")));
                }
                let _ = tx.send(format!("TP debug {n}: {s}"));
            };
            let find = |n: &str| scr.find(&[n.to_string()], None);
            let click = |p: Point, why: &str| {
                arb.run(Batch::new(h, why).click(p.x, p.y))
                    .map_err(|e| e.to_string())
            };
            // 1. settings button
            let at = find("settings_button");
            let p = at.unwrap_or(Point::new(26, 570));
            say(format!(
                "settings_button {} - clicking {},{}",
                if at.is_some() {
                    "found"
                } else {
                    "NOT found, using the fallback spot"
                },
                p.x,
                p.y
            ));
            if let Err(e) = click(p, "settings") {
                return say(format!("click failed: {e}"));
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
            say("waited 2 s - settings should be open".into());
            // 2. scroll to the label
            let mut label = find("tp_spawn");
            let mut notches = 0;
            while label.is_none() && notches < 12 {
                let _ = arb.run(
                    Batch::new(h, "scroll")
                        .move_to(400, 300)
                        .then(Action::Scroll {
                            at: Point::new(400, 300),
                            notches: -1,
                        }),
                );
                std::thread::sleep(std::time::Duration::from_millis(250));
                notches += 1;
                label = find("tp_spawn");
            }
            let Some(l) = label else {
                return say(format!("tp_spawn NOT found after {notches} scroll notch(es) - re-capture the 'Teleport back to Spawn' text"));
            };
            say(format!(
                "tp_spawn found at {},{} after {notches} notch(es)",
                l.x, l.y
            ));
            // 3. the button
            let (p, how) = match find("tp_spawn_button") {
                Some(b) => (b, "tp_spawn_button found".to_string()),
                None => (
                    Point::new(l.x + 178, l.y + 7),
                    "no tp_spawn_button - label +178,+7".to_string(),
                ),
            };
            say(format!("{how} - clicking {},{}", p.x, p.y));
            if let Err(e) = click(p, "tp") {
                return say(format!("click failed: {e}"));
            }
            std::thread::sleep(std::time::Duration::from_millis(1200));
            say("clicked - did the character teleport?".into());
            // 4. close
            let at = find("settings_close");
            let p = at.unwrap_or(Point::new(583, 147));
            if let Err(e) = click(p, "close") {
                return say(format!("close failed: {e}"));
            }
            std::thread::sleep(std::time::Duration::from_millis(800));
            say(format!(
                "settings_close {} at {},{} - done",
                if at.is_some() {
                    "found"
                } else {
                    "NOT found, used the fallback"
                },
                p.x,
                p.y
            ));
        });
    }

    /// Red cross where the TP click would land (drawn on the Patterns picture).
    pub(super) fn debug_tp_overlay(
        &self,
        painter: &egui::Painter,
        to_screen: &dyn Fn(Rect) -> egui::Rect,
    ) {
        if let Some(p) = self.tp_dbg_click {
            let c = to_screen(Rect::new(p.x, p.y, 0, 0)).min;
            let st = Stroke::new(2.0, Color32::from_rgb(248, 80, 80));
            painter.line_segment([c + Vec2::new(-8.0, 0.0), c + Vec2::new(8.0, 0.0)], st);
            painter.line_segment([c + Vec2::new(0.0, -8.0), c + Vec2::new(0.0, 8.0)], st);
            painter.circle_stroke(c, 6.0, st);
        }
    }
}
