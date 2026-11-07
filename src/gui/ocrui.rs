//! OCR areas page: see every fixed read on a capture of the client, drag one
//! to where it really is (drag inside = move, drag the corner = resize),
//! test-read it.

use super::*;
use crate::geom::Rect;
use eframe::egui::{Align2, FontId, StrokeKind};

/// What the OCR areas page remembers between frames.
#[derive(Default)]
pub(super) struct OcrUi {
    pub sel: String,
    /// The area while dragging, and whether it's a resize.
    drag: Option<(Rect, bool)>,
    pub read: String,
}

impl App {
    pub(super) fn page_ocr_areas(&mut self, ui: &mut egui::Ui) {
        ui.heading("OCR areas");
        ui.label(
            egui::RichText::new("Where each fixed read happens on the client. Pick one, drag it on the picture (inside = move, bottom-right corner = resize), then Test read. Saved at once; Reset puts the built-in one back.")
                .color(MUTED),
        );
        let areas = crate::ocrareas::all();
        if self.ocr_ui.sel.is_empty() {
            self.ocr_ui.sel = areas[0].key.to_string();
        }
        let cur = areas
            .iter()
            .find(|a| a.key == self.ocr_ui.sel)
            .unwrap_or(&areas[0]);
        let rect_now = self
            .settings
            .ocr_areas
            .get(cur.key)
            .map(|r| Rect::new(r[0], r[1], r[2], r[3]))
            .unwrap_or(cur.default);
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("ocr_area")
                .width(320.0)
                .selected_text(cur.label)
                .show_ui(ui, |ui| {
                    for a in &areas {
                        let moved = self.settings.ocr_areas.contains_key(a.key);
                        let label = if moved {
                            format!("{} (moved)", a.label)
                        } else {
                            a.label.to_string()
                        };
                        ui.selectable_value(&mut self.ocr_ui.sel, a.key.to_string(), label);
                    }
                });
            ui.label(format!(
                "x {} y {} w {} h {}",
                rect_now.x, rect_now.y, rect_now.w, rect_now.h
            ));
            if ui.button("Test read").clicked() {
                self.ocr_ui.read = match &self.frame {
                    Some(f) => match self.platform.ocr(f, rect_now) {
                        Some(t) if !t.trim().is_empty() => format!("read: \"{}\"", t.trim()),
                        Some(_) => "read nothing there".into(),
                        None => "no OCR on this PC".into(),
                    },
                    None => "Capture the client first".into(),
                };
            }
            if ui.button("Reset").clicked() {
                self.settings.ocr_areas.remove(cur.key);
                crate::ocrareas::set_all(&self.settings.ocr_areas);
                self.dirty();
            }
        });
        if !self.ocr_ui.read.is_empty() {
            ui.label(egui::RichText::new(&self.ocr_ui.read).color(ACCENT));
        }
        let key = cur.key;
        let Some((resp, rect, scale)) = self.client_view(ui) else {
            return;
        };
        let painter = ui.painter_at(rect);
        let to_screen = |r: Rect| {
            egui::Rect::from_min_size(
                rect.min + Vec2::new(r.x as f32, r.y as f32) * scale,
                Vec2::new(r.w as f32, r.h as f32) * scale,
            )
        };
        // the others, faint
        for a in &areas {
            if a.key == key {
                continue;
            }
            let r = crate::ocrareas::area(a.key);
            painter.rect_stroke(
                to_screen(r),
                0.0,
                Stroke::new(1.0, Color32::from_white_alpha(70)),
                StrokeKind::Inside,
            );
        }
        // dragging
        let shown = self.ocr_ui.drag.map(|d| d.0).unwrap_or(rect_now);
        let sr = to_screen(shown);
        let corner = sr.max;
        if resp.drag_started() {
            if let Some(p) = resp.interact_pointer_pos() {
                let resize = (p - corner).length() < 12.0;
                if resize || sr.contains(p) {
                    self.ocr_ui.drag = Some((rect_now, resize));
                }
            }
        }
        if let Some((mut r, resize)) = self.ocr_ui.drag {
            let d = resp.drag_delta() / scale;
            let (dx, dy) = (d.x.round() as i32, d.y.round() as i32);
            if resize {
                r.w = (r.w + dx).max(6);
                r.h = (r.h + dy).max(6);
            } else {
                r.x += dx;
                r.y += dy;
            }
            self.ocr_ui.drag = Some((r, resize));
            if resp.drag_stopped() {
                self.ocr_ui.drag = None;
                self.settings
                    .ocr_areas
                    .insert(key.to_string(), [r.x, r.y, r.w, r.h]);
                crate::ocrareas::set_all(&self.settings.ocr_areas);
                self.dirty();
            }
        }
        let shown = self.ocr_ui.drag.map(|d| d.0).unwrap_or(rect_now);
        let sr = to_screen(shown);
        painter.rect_stroke(sr, 0.0, Stroke::new(2.0, ACCENT), StrokeKind::Inside);
        painter.rect_filled(
            egui::Rect::from_center_size(sr.max, Vec2::splat(8.0)),
            1.0,
            ACCENT,
        );
        painter.text(
            sr.min - Vec2::new(0.0, 2.0),
            Align2::LEFT_BOTTOM,
            cur.label,
            FontId::proportional(11.0),
            Color32::WHITE,
        );
    }
}
