//! Routes page: minimap map files (maps/data/<Map>.json) - make them from
//! the saved minimap pictures, and click the walk points onto a minimap.

use super::{card, App, BAD, GOOD, MUTED, PURPLE, TEXT, WARN};
use crate::game::mapnav::{self, MapData};
use eframe::egui::{
    self, Color32, ColorImage, RichText, Sense, Stroke, TextureHandle, TextureOptions, Vec2,
};

/// Editor zoom (minimap px -> UI points).
const ZOOM: f32 = 3.0;

#[derive(Default)]
pub struct MapsUi {
    sel: String,
    data: Option<MapData>,
    tex: Option<(String, TextureHandle)>,
    msg: String,
    dirty: bool,
}

impl App {
    pub(super) fn minimap_maps_ui(&mut self, ui: &mut egui::Ui) {
        let all = mapnav::load_all();
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("MINIMAP MAPS").size(11.0).color(MUTED));
                ui.label(
                    RichText::new(format!("{} map file(s)", all.len()))
                        .size(11.0)
                        .color(TEXT),
                );
                if ui
                    .button("Make map files from saved minimaps")
                    .on_hover_text("every picture in maps/minimap/ without a map file gets one: base, spawn and the enemy path found on it")
                    .clicked()
                {
                    let made = mapnav::build_missing();
                    self.maps_ui.msg = if made.is_empty() {
                        "no new minimap pictures (each map's picture is saved the first time the macro sees it)".into()
                    } else {
                        format!("made {}: {}", made.len(), made.join(", "))
                    };
                }
                ui.checkbox(&mut self.settings.minimap_nav, "walk by the minimap when a map has no route")
                    .changed()
                    .then(|| self.dirty());
            });
            ui.label(
                RichText::new("A map without a recorded route walks to its map file's points by the minimap: W tells which way is which, the arrow is re-read every 200 ms and steered; stuck for 600 ms = back up and strafe. Pick a map, click the points to walk to in order.")
                    .size(11.0)
                    .color(MUTED),
            );
            if !self.maps_ui.msg.is_empty() {
                ui.label(RichText::new(&self.maps_ui.msg).size(11.0).color(WARN));
            }
            ui.add_space(6.0);
            ui.horizontal_top(|ui| {
                // the list
                ui.vertical(|ui| {
                    ui.set_width(200.0);
                    egui::ScrollArea::vertical()
                        .id_salt("mapfiles")
                        .max_height(440.0)
                        .show(ui, |ui| {
                            for d in all.values() {
                                let sel = self.maps_ui.sel == d.map;
                                let txt = format!(
                                    "{}  ·  {} walk pt{}",
                                    d.map,
                                    d.walk.len(),
                                    if d.walk.len() == 1 { "" } else { "s" }
                                );
                                let col = if d.walk.is_empty() { MUTED } else { GOOD };
                                if ui
                                    .selectable_label(sel, RichText::new(txt).color(col))
                                    .clicked()
                                {
                                    self.maps_ui.sel = d.map.clone();
                                    self.maps_ui.data = Some(d.clone());
                                    self.maps_ui.dirty = false;
                                }
                            }
                        });
                });
                // the editor
                ui.vertical(|ui| self.map_editor(ui));
            });
        });
    }

    fn map_editor(&mut self, ui: &mut egui::Ui) {
        let Some(mut d) = self.maps_ui.data.clone() else {
            ui.label(RichText::new("pick a map").color(MUTED));
            return;
        };
        // its minimap picture
        if self.maps_ui.tex.as_ref().is_none_or(|(n, _)| *n != d.map) {
            let p = crate::game::minimap::dir().join(format!("{}.png", d.map));
            self.maps_ui.tex = crate::capture::Frame::load_png(&p).ok().map(|f| {
                let mut rgba = f.bgra.clone();
                for px in rgba.chunks_exact_mut(4) {
                    px.swap(0, 2);
                }
                let img = ColorImage::from_rgba_unmultiplied(
                    [f.width as usize, f.height as usize],
                    &rgba,
                );
                (
                    d.map.clone(),
                    ui.ctx().load_texture(
                        format!("navmap-{}", d.map),
                        img,
                        TextureOptions::NEAREST,
                    ),
                )
            });
        }
        let size = Vec2::splat(mapnav::SIZE as f32 * ZOOM);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 6.0, Color32::from_rgb(20, 22, 30));
        if let Some((_, t)) = &self.maps_ui.tex {
            p.image(
                t.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            p.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "no minimap picture saved for this map",
                egui::FontId::proportional(13.0),
                MUTED,
            );
        }
        let at = |q: [i32; 2]| rect.min + Vec2::new(q[0] as f32, q[1] as f32) * ZOOM;
        // the enemy path, base, spawn
        for w in d.path.windows(2) {
            p.line_segment(
                [at(w[0]), at(w[1])],
                Stroke::new(1.5, Color32::from_rgba_unmultiplied(80, 255, 80, 140)),
            );
        }
        if let Some(b) = d.base {
            p.circle_stroke(at(b), 7.0, Stroke::new(2.0, Color32::WHITE));
        }
        if let Some(s) = d.spawn {
            p.circle_stroke(at(s), 7.0, Stroke::new(2.0, BAD));
        }
        // the walk, numbered
        for (i, w) in d.walk.iter().enumerate() {
            if i > 0 {
                p.line_segment([at(d.walk[i - 1]), at(*w)], Stroke::new(2.0, PURPLE));
            }
            p.circle_filled(at(*w), 7.0, PURPLE);
            p.text(
                at(*w),
                egui::Align2::CENTER_CENTER,
                (i + 1).to_string(),
                egui::FontId::proportional(10.0),
                Color32::WHITE,
            );
        }
        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let q = (pos - rect.min) / ZOOM;
                d.walk.push([q.x.round() as i32, q.y.round() as i32]);
                self.maps_ui.dirty = true;
            }
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(&d.map).strong());
            if ui.button("Undo point").clicked() {
                d.walk.pop();
                self.maps_ui.dirty = true;
            }
            if ui.button("Clear").clicked() {
                d.walk.clear();
                self.maps_ui.dirty = true;
            }
            let save = egui::Button::new(if self.maps_ui.dirty { "Save *" } else { "Save" });
            if ui.add(save).clicked() {
                self.maps_ui.msg = match mapnav::save(&d) {
                    Ok(p) => {
                        self.maps_ui.dirty = false;
                        format!("saved {}", p.display())
                    }
                    Err(e) => format!("not saved: {e}"),
                };
            }
        });
        ui.label(
            RichText::new("click = add a point (walked in order) · white ring = base · red ring = enemy spawn · green = enemy path")
                .size(11.0)
                .color(MUTED),
        );
        self.maps_ui.data = Some(d);
    }
}
