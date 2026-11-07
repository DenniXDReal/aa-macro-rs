//! Movement page: minimap map files (maps/data/<Map>.json) - make them from
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
    /// A click adds a walk point (else: sets where it stops on the path).
    click_walk: bool,
    /// The editor's rename box.
    rename: String,
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
                ui.checkbox(&mut self.settings.minimap_nav, "walk by the minimap when a map has no recorded movement")
                    .changed()
                    .then(|| self.dirty());
                ui.label("Max Path Distance %");
                if ui
                    .add(
                        egui::DragValue::new(&mut self.settings.minimap_walk_pct)
                            .range(5..=100)
                            .suffix(" %"),
                    )
                    .on_hover_text("where to stop: % of the enemies' walk from their spawn (0 % = their spawn, 100 % = your base). Walks out from the base along the path to it, then places. Sandstorm: at least 25 %.")
                    .changed()
                {
                    self.dirty();
                }
            });
            ui.label(
                RichText::new("A map without recorded movement walks to its map file's points by the minimap: W tells which way is which, the arrow is re-read every 200 ms and steered; stuck for 600 ms = back up and strafe. Pick a map, click the points to walk to in order.")
                    .size(11.0)
                    .color(MUTED),
            );
            if !self.maps_ui.msg.is_empty() {
                ui.label(RichText::new(&self.maps_ui.msg).size(11.0).color(WARN));
            }
            ui.add_space(4.0);
            self.client_walk_ui(ui);
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
                                let txt = if d.walk.is_empty() {
                                    format!(
                                        "{}  ·  path {}%",
                                        d.map,
                                        d.walk_pct.unwrap_or(self.settings.minimap_walk_pct)
                                    )
                                } else {
                                    format!(
                                        "{}  ·  {} walk pt{}",
                                        d.map,
                                        d.walk.len(),
                                        if d.walk.len() == 1 { "" } else { "s" }
                                    )
                                };
                                let col = if d.walk.is_empty() && d.path.len() < 2 {
                                    MUTED
                                } else {
                                    GOOD
                                };
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
            self.maps_ui.tex = crate::capture::Frame::load_png(&p)
                .ok()
                .or_else(|| mapnav::library_picture(&d.map))
                .map(|f| {
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
        // other lanes (2- / 3-lane maps)
        for lane in &d.lanes {
            for w in lane.windows(2) {
                p.line_segment(
                    [at(w[0]), at(w[1])],
                    Stroke::new(1.5, Color32::from_rgba_unmultiplied(220, 90, 255, 160)),
                );
            }
            if let Some(s) = lane.first() {
                p.circle_stroke(at(*s), 7.0, Stroke::new(2.0, BAD));
            }
        }
        if let Some(b) = d.base {
            p.circle_stroke(at(b), 7.0, Stroke::new(2.0, Color32::WHITE));
        }
        if let Some(s) = d.spawn {
            p.circle_stroke(at(s), 7.0, Stroke::new(2.0, BAD));
        }
        // lane names at their spawns (2+ lanes)
        let polys = mapnav::lane_polys(&d);
        if polys.len() > 1 {
            for (poly, name) in polys.iter().zip(mapnav::lane_names(&d)) {
                p.text(
                    at(poly[0]) + Vec2::new(0.0, -14.0),
                    egui::Align2::CENTER_BOTTOM,
                    name,
                    egui::FontId::proportional(11.0),
                    Color32::WHITE,
                );
            }
        }
        // each client's stop (lane + % from the client table)
        if d.walk.is_empty() {
            for no in 1..=self.settings.max_instances.max(1) {
                let cw = self
                    .settings
                    .client_walk
                    .get(&no)
                    .copied()
                    .unwrap_or_default();
                if cw == mapnav::ClientWalk::default() {
                    continue;
                }
                let plan = mapnav::plan(&d, cw, self.settings.minimap_walk_pct, 0);
                if let Some(e) = plan.points.last() {
                    let c = at([e.0 as i32, e.1 as i32]);
                    p.circle_stroke(c, 6.0, Stroke::new(2.0, Color32::from_rgb(255, 210, 80)));
                    p.text(
                        c + Vec2::new(8.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        format!("C{no}"),
                        egui::FontId::proportional(11.0),
                        Color32::from_rgb(255, 210, 80),
                    );
                }
            }
        }
        // no points of yours: the part of the path it walks (dotted)
        if d.walk.is_empty() {
            let auto = mapnav::walk_points(&d, self.settings.minimap_walk_pct);
            for q in &auto {
                p.circle_filled(
                    at([q.0 as i32, q.1 as i32]),
                    2.5,
                    Color32::from_rgb(190, 150, 255),
                );
            }
            if let Some(end) = auto.last() {
                p.circle_stroke(
                    at([end.0 as i32, end.1 as i32]),
                    8.0,
                    Stroke::new(2.0, PURPLE),
                );
                // more than one lane: the reach it has to keep to all of them
                if !d.lanes.is_empty() {
                    p.circle_stroke(
                        at([end.0 as i32, end.1 as i32]),
                        mapnav::COVER_PX * ZOOM,
                        Stroke::new(1.0, Color32::from_rgba_unmultiplied(190, 150, 255, 120)),
                    );
                }
            }
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
        // each lane's own stop (multi-lane maps), with its %
        let polys_n = mapnav::lane_polys(&d).len();
        if d.walk.is_empty() && polys_n > 1 {
            for (name, pct) in d.lane_pct.clone() {
                let pick = match name.as_str() {
                    "left" => mapnav::LanePick::Left,
                    "right" => mapnav::LanePick::Right,
                    "center" => mapnav::LanePick::Center,
                    _ => continue,
                };
                let plan = mapnav::plan(
                    &d,
                    mapnav::ClientWalk {
                        lane: pick,
                        pct: None,
                    },
                    self.settings.minimap_walk_pct,
                    0,
                );
                if let Some(e) = plan.points.last() {
                    let c = at([e.0 as i32, e.1 as i32]);
                    p.circle_stroke(c, 8.0, Stroke::new(2.0, PURPLE));
                    p.text(
                        c + Vec2::new(10.0, -10.0),
                        egui::Align2::LEFT_BOTTOM,
                        format!("{name} {pct}%"),
                        egui::FontId::proportional(11.0),
                        Color32::WHITE,
                    );
                }
            }
        }
        // hovering the path: where it would stop, and its %
        if !self.maps_ui.click_walk {
            if let Some(h) = resp.hover_pos() {
                let q = (h - rect.min) / ZOOM;
                if let Some((_, pct, spot)) = mapnav::stop_from_click(&d, (q.x, q.y), 10.0) {
                    let c = at([spot.0.round() as i32, spot.1.round() as i32]);
                    p.circle_stroke(c, 8.0, Stroke::new(1.0, Color32::from_white_alpha(180)));
                    p.text(
                        c + Vec2::new(10.0, 10.0),
                        egui::Align2::LEFT_TOP,
                        format!("{pct}%"),
                        egui::FontId::proportional(12.0),
                        Color32::WHITE,
                    );
                }
            }
        }
        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let q = (pos - rect.min) / ZOOM;
                if self.maps_ui.click_walk {
                    d.walk.push([q.x.round() as i32, q.y.round() as i32]);
                    self.maps_ui.dirty = true;
                } else {
                    // a click on a path: that's where it stops (saved at once)
                    match mapnav::stop_from_click(&d, (q.x, q.y), 10.0) {
                        Some((lane, pct, _)) => {
                            mapnav::set_stop(&mut d, lane, pct);
                            let which = if polys_n > 1 {
                                format!("{} lane", mapnav::lane_names(&d)[lane])
                            } else {
                                "the path".into()
                            };
                            self.maps_ui.msg = match mapnav::save(&d) {
                                Ok(_) => {
                                    self.maps_ui.dirty = false;
                                    format!(
                                        "{}: stops at {pct}% of {which} (of the enemies' walk from their spawn) - saved{}",
                                        d.map,
                                        if d.walk.is_empty() {
                                            ""
                                        } else {
                                            " (your walk points still win - Clear them to use it)"
                                        }
                                    )
                                }
                                Err(e) => format!("not saved: {e}"),
                            };
                        }
                        None => {
                            self.maps_ui.msg =
                                "click on a path (the green / violet line) to set where it stops"
                                    .into();
                        }
                    }
                }
            }
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.maps_ui.rename)
                    .desired_width(160.0)
                    .hint_text("real map name"),
            );
            if ui
                .button("Rename")
                .on_hover_text("name this map file after the real map (e.g. Minimap 03 -> Navy Bay); the built-in picture stays as well")
                .clicked()
                && !self.maps_ui.rename.trim().is_empty()
            {
                let old = d.map.clone();
                d.map = self.maps_ui.rename.trim().to_string();
                self.maps_ui.msg = match mapnav::save(&d) {
                    Ok(_) => {
                        let old_file = mapnav::user_dir().join(format!("{old}.json"));
                        let _ = std::fs::remove_file(old_file);
                        // its picture under the new name too
                        if let Some(f) = crate::capture::Frame::load_png(
                            &crate::game::minimap::dir().join(format!("{old}.png")),
                        )
                        .ok()
                        .or_else(|| mapnav::library_picture(&old))
                        {
                            let _ = std::fs::create_dir_all(crate::game::minimap::dir());
                            let _ = f.save_png(
                                &crate::game::minimap::dir().join(format!("{}.png", d.map)),
                            );
                        }
                        self.maps_ui.sel = d.map.clone();
                        self.maps_ui.dirty = false;
                        self.maps_ui.rename.clear();
                        format!("renamed {old} -> {}", d.map)
                    }
                    Err(e) => format!("not renamed: {e}"),
                };
            }
        });
        ui.horizontal(|ui| {
            ui.label("click =");
            ui.radio_value(&mut self.maps_ui.click_walk, false, "set the stop point")
                .on_hover_text("click on the path: it stops there (the % is worked out and saved)");
            ui.radio_value(&mut self.maps_ui.click_walk, true, "add a walk point")
                .on_hover_text("your own points, walked in order (they win over the %)");
        });
        ui.horizontal(|ui| {
            let mut own = d.walk_pct.is_some();
            if ui
                .checkbox(&mut own, "own path %")
                .on_hover_text(
                    "this map walks a different share of the path than the setting (e.g. 35%)",
                )
                .changed()
            {
                d.walk_pct = own.then_some(self.settings.minimap_walk_pct);
                self.maps_ui.dirty = true;
            }
            if let Some(pct) = d.walk_pct.as_mut() {
                if ui
                    .add(egui::DragValue::new(pct).range(5..=100).suffix(" %"))
                    .changed()
                {
                    self.maps_ui.dirty = true;
                }
            }
        });
        // per lane: its own %
        let names = mapnav::lane_names(&d);
        if names.len() > 1 {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("lane %").color(MUTED));
                for n in &names {
                    let mut own = d.lane_pct.contains_key(n);
                    if ui
                        .checkbox(&mut own, n.as_str())
                        .on_hover_text("this lane stops at its own % (a client's own % still wins)")
                        .changed()
                    {
                        if own {
                            let v = d.walk_pct.unwrap_or(self.settings.minimap_walk_pct);
                            d.lane_pct.insert(n.clone(), v);
                        } else {
                            d.lane_pct.remove(n);
                        }
                        self.maps_ui.dirty = true;
                    }
                    if let Some(v) = d.lane_pct.get_mut(n) {
                        if ui
                            .add(egui::DragValue::new(v).range(5..=100).suffix(" %"))
                            .changed()
                        {
                            self.maps_ui.dirty = true;
                        }
                    }
                }
            });
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
            RichText::new("click on a path = it stops there (or add walk points, see above) · white ring = base · red ring = enemy spawn · green = enemy path · violet = other lanes (stop covers all) · purple ring = where it stops (Auto) · yellow C1/C2 = each client's stop · % = of the enemies' walk from their spawn")
                .size(11.0)
                .color(MUTED),
        );
        self.maps_ui.data = Some(d);
    }

    /// Which lane each client walks and its own %.
    fn client_walk_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("CLIENTS ON MULTI-LANE MAPS")
                .size(11.0)
                .color(MUTED),
        );
        let mut changed = false;
        egui::Grid::new("client_walk").num_columns(3).show(ui, |ui| {
            for no in 1..=self.settings.max_instances.max(1) {
                let mut cw = self.settings.client_walk.get(&no).copied().unwrap_or_default();
                ui.label(format!("Client {no}"));
                egui::ComboBox::from_id_salt(("lane", no))
                    .selected_text(cw.lane.label())
                    .show_ui(ui, |ui| {
                        for l in mapnav::LanePick::ALL {
                            changed |= ui.selectable_value(&mut cw.lane, l, l.label()).changed();
                        }
                    });
                ui.horizontal(|ui| {
                    let mut own = cw.pct.is_some();
                    if ui
                        .checkbox(&mut own, "own %")
                        .on_hover_text("this client stops at its own % on every map (else the lane's / map's / Max Path Distance %)")
                        .changed()
                    {
                        cw.pct = own.then_some(self.settings.minimap_walk_pct);
                        changed = true;
                    }
                    if let Some(v) = cw.pct.as_mut() {
                        changed |= ui
                            .add(egui::DragValue::new(v).range(5..=100).suffix(" %"))
                            .changed();
                    }
                });
                ui.end_row();
                if cw == mapnav::ClientWalk::default() {
                    self.settings.client_walk.remove(&no);
                } else {
                    self.settings.client_walk.insert(no, cw);
                }
            }
        });
        ui.label(
            RichText::new("Auto = the only lane / a spot covering both of 2 / the center of 3. A lane the map hasn't got = Auto. Sandstorm raises any stop under 25 % to 25 % (also mid-match, after the vote).")
                .size(11.0)
                .color(MUTED),
        );
        if changed {
            self.dirty();
        }
    }
}
