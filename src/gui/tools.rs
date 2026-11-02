//! Tools carried over from the first UI: capture, tiling, doctor, updates,
//! placement designer and the pattern tool.

use super::*;
use crate::accounts::AccountRole;
use crate::flows::FLOWS;
use crate::geom::{Point, Rect};
use crate::input::{ArbiterHandle, ArbiterOptions, Batch};
use crate::layout;
use crate::vision::{self, Pattern};
use eframe::egui::{Align2, DragValue, FontId, Pos2, Slider, StrokeKind};

impl App {
    pub(super) fn say(&mut self, s: impl Into<String>) {
        let s = s.into();
        if self
            .log
            .last()
            .is_some_and(|l| l == &s || l.starts_with(&format!("{s} (x")))
        {
            let last = self.log.last_mut().unwrap();
            let n = last
                .rsplit_once(" (x")
                .and_then(|(_, r)| r.trim_end_matches(')').parse::<u32>().ok())
                .unwrap_or(1);
            *last = format!("{s} (x{})", n + 1);
            return;
        }
        log::info!("{s}");
        self.log.push(s);
        if self.log.len() > 300 {
            self.log.drain(..100);
        }
    }

    pub(super) fn check_update(&mut self) {
        let Some(tok) = self.secrets.get(update::TOKEN_KEY) else {
            self.upd = Upd::Failed("add a GitHub token first (Settings > Updates)".into());
            return;
        };
        self.upd = Upd::Checking;
        let (tx, repo) = (self.upd_tx.clone(), self.settings.update_repo.clone());
        std::thread::spawn(move || {
            let r = match update::check(&repo, &tok) {
                Ok(None) => Upd::UpToDate,
                Ok(Some(a)) => Upd::Available(a),
                Err(e) => Upd::Failed(e.to_string()),
            };
            let _ = tx.send(r);
        });
    }

    pub(super) fn install_update(&mut self, a: Available) {
        let Some(tok) = self.secrets.get(update::TOKEN_KEY) else {
            return;
        };
        self.upd = Upd::Installing;
        let tx = self.upd_tx.clone();
        std::thread::spawn(move || {
            let r = match update::install(&a, &tok) {
                Ok(p) => Upd::Installed(p),
                Err(e) => Upd::Failed(e.to_string()),
            };
            let _ = tx.send(r);
        });
    }

    pub(super) fn update_banner(&mut self, ui: &mut egui::Ui) {
        let mut act: Option<Upd> = None;
        match &self.upd {
            Upd::Available(a) => {
                ui.horizontal(|ui| {
                    ui.colored_label(GOOD, format!("New build {} is out.", a.build));
                    if ui.button("Update now").clicked() {
                        act = Some(Upd::Available(a.clone()));
                    }
                });
            }
            Upd::Installing => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Downloading update…");
                });
            }
            Upd::Installed(p) => {
                ui.horizontal(|ui| {
                    ui.colored_label(GOOD, "Update installed.");
                    if ui.button("Restart now").clicked() {
                        act = Some(Upd::Installed(p.clone()));
                    }
                });
            }
            _ => {}
        }
        match act {
            Some(Upd::Available(a)) => self.install_update(a),
            Some(Upd::Installed(p)) => {
                self.settings.save(&self.settings_path).ok();
                match update::restart(&p) {
                    Ok(()) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                    Err(e) => self.say(format!(
                        "restart failed: {e} - start aa-macro again yourself"
                    )),
                }
            }
            _ => {}
        }
    }

    pub(super) fn dirty(&mut self) {
        self.dirty_at = Some(Instant::now());
    }

    pub(super) fn refresh_clients(&mut self) {
        self.refresh_inner(true);
    }

    /// Background refresh every few seconds: drops closed windows, picks up
    /// new ones, only logs when something changed.
    pub(super) fn refresh_quiet(&mut self) {
        self.refresh_inner(false);
    }

    fn refresh_inner(&mut self, loud: bool) {
        self.last_refresh = Instant::now();
        let main_h = self.clients.get(self.main).map(|c| c.hwnd);
        let sel_h = self.clients.get(self.sel).map(|c| c.hwnd);
        let mut w = self.platform.list_windows(&self.settings.process_name);
        w.truncate(self.settings.max_instances);
        let changed = w.iter().map(|c| c.hwnd).collect::<Vec<_>>()
            != self.clients.iter().map(|c| c.hwnd).collect::<Vec<_>>();
        self.settings
            .ensure(w.len().max(self.settings.max_instances));
        // forget usernames of closed clients
        let alive: std::collections::HashSet<u32> = w.iter().map(|c| c.pid).collect();
        self.names.retain(|pid, _| alive.contains(pid));
        self.name_tries.retain(|pid, _| alive.contains(pid));
        self.clients = w;
        let admin = self
            .clients
            .iter()
            .any(|c| self.platform.needs_admin(c.hwnd));
        if admin && !self.admin_needed {
            self.say("Roblox runs as administrator - restart the macro as admin (red banner) or start RAM/Roblox without admin");
        }
        self.admin_needed = admin;
        self.lookup_names();
        let find = |h: Option<crate::geom::Hwnd>| {
            h.and_then(|h| self.clients.iter().position(|c| c.hwnd == h))
        };
        // the RAM account marked Main becomes the main client
        let main_acc = self
            .settings
            .accounts
            .iter()
            .find(|a| a.role == AccountRole::Main && a.enabled)
            .map(|a| a.username.clone());
        let by_name = main_acc.and_then(|n| {
            self.clients
                .iter()
                .position(|c| self.names.get(&c.pid) == Some(&n))
        });
        self.main = by_name.or(find(main_h)).unwrap_or(0);
        self.sel = find(sel_h).unwrap_or(self.main);
        if changed {
            self.alt_tex.clear();
            if self.clients.get(self.sel).is_some() {
                self.live = true;
            }
        }
        if loud || changed {
            let n = self.clients.len();
            let who: Vec<String> = (0..n).map(|i| self.client_label(i)).collect();
            self.say(if n == 0 {
                "no Roblox clients open".to_string()
            } else {
                format!("found {n} Roblox client(s): {}", who.join(", "))
            });
        }
    }

    pub(super) fn arbiter(&mut self) -> ArbiterHandle {
        if self.arbiter.is_none() {
            let s = &self.settings;
            self.arbiter = Some(Arbiter::start(
                self.platform.clone(),
                self.control.clone(),
                ArbiterOptions {
                    mouse_settle_ms: s.mouse_settle_ms,
                    click_hold_ms: s.click_hold_ms,
                    max_lease_ms: s.max_lease_ms,
                    click_mode: s.click_mode,
                    swap_delay_ms: s.swap_delay_ms,
                },
            ));
        }
        self.arbiter.as_ref().unwrap().handle()
    }

    /// Which clients place units, each with its own team (two accounts
    /// spam-placing two different loadouts at once).
    pub(super) fn placers_ui(&mut self, ui: &mut egui::Ui) {
        self.settings.fix_teams();
        let mut changed = false;
        ui.label(
            egui::RichText::new("Clients that place units - each can use its own team")
                .color(MUTED),
        );
        for i in 0..self.clients.len().max(1) {
            let n = i + 1;
            ui.horizontal(|ui| {
                let mut on = self.settings.placers.contains(&n);
                if super::toggle(ui, &mut on, &self.client_label(i)) {
                    if on {
                        self.settings.placers.push(n);
                        self.settings.placers.sort();
                    } else {
                        self.settings.placers.retain(|x| *x != n);
                    }
                    changed = true;
                }
                if on {
                    ui.label("team");
                    let cur = self.settings.placer_teams.get(&n).copied();
                    let shown = match cur {
                        Some(t) if t < self.settings.teams.len() => {
                            self.settings.teams[t].name.clone()
                        }
                        _ => format!("in use ({})", self.settings.teams[self.settings.team].name),
                    };
                    ComboBox::from_id_salt(("placer-team", n))
                        .selected_text(shown)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(cur.is_none(), "the team in use")
                                .clicked()
                            {
                                self.settings.placer_teams.remove(&n);
                                changed = true;
                            }
                            for (t, team) in self.settings.teams.iter().enumerate() {
                                if ui.selectable_label(cur == Some(t), &team.name).clicked() {
                                    self.settings.placer_teams.insert(n, t);
                                    changed = true;
                                }
                            }
                        });
                }
            });
        }
        if self.settings.placers.len() > 1 {
            ui.small("All of these place at the same time (input takes turns). Make teams on Home > Control (+ New), then pick one per client here.");
        }
        if changed {
            self.dirty();
        }
    }

    pub(super) fn read_level_test(&mut self) {
        let Some(info) = self.clients.get(self.sel).cloned() else {
            self.say("no client");
            return;
        };
        let scr = crate::game::LiveScreen::new(self.platform.clone(), info.hwnd, &self.lib);
        let Some(f) = scr.frame() else {
            self.say("capture failed");
            return;
        };
        let area = scr.level_area();
        match self.platform.ocr(&f, area) {
            Some(t) => {
                let lv = crate::game::parse_level(&t);
                self.say(format!(
                    "OCR at {},{} {}x{}: '{}' -> {}",
                    area.x,
                    area.y,
                    area.w,
                    area.h,
                    t.trim(),
                    match lv {
                        Some(Ok(n)) => format!("level {n}"),
                        Some(Err(())) => "MAX".into(),
                        None => "no level found".into(),
                    }
                ));
            }
            None => self.say("OCR not available (needs Windows 10/11 with an OCR language - English is built in)"),
        }
    }

    pub(super) fn capture(&mut self, ctx: &egui::Context) {
        self.last_cap = Instant::now();
        let Some(info) = self.clients.get(self.sel).cloned() else {
            return;
        };
        match self.platform.capture_client(info.hwnd) {
            Ok(f) => {
                let img = ColorImage::from_rgba_unmultiplied(
                    [f.width as usize, f.height as usize],
                    &rgba(&f),
                );
                match &mut self.tex {
                    Some(t) => t.set(img, TextureOptions::NEAREST),
                    None => {
                        self.tex = Some(ctx.load_texture("client", img, TextureOptions::NEAREST))
                    }
                }
                self.frame = Some(f);
                self.update_preview(ctx);
            }
            Err(e) => {
                // usually the window just closed: re-scan instead of nagging
                self.refresh_quiet();
                if self.clients.get(self.sel).map(|c| c.hwnd) == Some(info.hwnd) {
                    self.live = false;
                    self.say(format!("capture failed: {e:#}"));
                }
            }
        }
    }

    pub(super) fn tile(&mut self) {
        self.refresh_quiet();
        if self.admin_needed {
            self.say("can't tile: Roblox runs as administrator - restart the macro as admin");
            return;
        }
        let p = self.platform.clone();
        let Some(first) = self.clients.first().and_then(|c| p.geometry(c.hwnd)) else {
            self.say("no Roblox clients open to tile");
            return;
        };
        let (ow, oh) = first.outer_for_client(self.settings.client_w, self.settings.client_h);
        let rects = layout::tile(self.clients.len(), p.work_area(), ow, oh);
        let mut failed = 0;
        for (c, r) in self.clients.clone().iter().zip(rects) {
            if let Err(e) = p.move_resize(c.hwnd, r) {
                failed += 1;
                self.say(format!("tile failed: {e:#}"));
            }
            if let Some(g) = p.geometry(c.hwnd) {
                if (g.client.w, g.client.h) != (self.settings.client_w, self.settings.client_h) {
                    let (w, h) = g.outer_for_client(self.settings.client_w, self.settings.client_h);
                    let _ = p.move_resize(c.hwnd, Rect::new(g.outer.x, g.outer.y, w, h));
                }
            }
        }
        if failed == 0 {
            self.say(format!("tiled {} client(s)", self.clients.len()));
        }
    }

    pub(super) fn doctor(&mut self) {
        let p = self.platform.clone();
        for (i, c) in self.clients.clone().iter().enumerate() {
            let n = i + 1;
            let dpi = p.dpi(c.hwnd);
            let size = p.geometry(c.hwnd).map(|g| (g.client.w, g.client.h));
            let cap = p.capture_client(c.hwnd);
            let mut bad = vec![];
            if p.needs_admin(c.hwnd) {
                bad.push(
                    "Roblox runs as admin, the macro doesn't (restart the macro as admin)"
                        .to_string(),
                );
            }
            if dpi != 96 {
                bad.push(format!("scaling {}% (set 100%)", dpi * 100 / 96));
            }
            if size != Some((self.settings.client_w, self.settings.client_h)) {
                bad.push(format!("client {size:?} (press Tile)"));
            }
            match cap {
                Ok(f) if f.is_blank() => bad.push("capture is black".into()),
                Err(e) => bad.push(format!("capture failed: {e}")),
                _ => {}
            }
            if bad.is_empty() {
                self.say(format!("#{n} ok"));
            } else {
                self.say(format!("#{n} PROBLEM: {}", bad.join(", ")));
            }
        }
    }

    // ---------- pages ----------

    pub(super) fn page_placement(&mut self, ui: &mut egui::Ui) {
        ui.heading("Placement designer");
        ui.horizontal(|ui| {
            ui.label("Map:");
            let mut maps: Vec<String> = self
                .placements
                .maps
                .keys()
                .map(|k| k.split(" @").next().unwrap_or(k).to_string())
                .collect();
            maps.extend(self.routes.maps.keys().cloned());
            maps.sort();
            maps.dedup();
            ComboBox::from_id_salt("map")
                .width(180.0)
                .selected_text(self.map.clone())
                .show_ui(ui, |ui| {
                    for m in maps {
                        ui.selectable_value(&mut self.map, m.clone(), m);
                    }
                });
            ui.add(
                egui::TextEdit::singleline(&mut self.new_map)
                    .desired_width(140.0)
                    .hint_text("new map name"),
            );
            if ui.button("Add map").clicked() && !self.new_map.trim().is_empty() {
                self.map = self.new_map.trim().to_string();
                self.placements.maps.entry(self.map.clone()).or_default();
                self.new_map.clear();
            }
        });
        ui.horizontal(|ui| {
            ui.label("New spots for:");
            ComboBox::from_id_salt("pslot")
                .width(90.0)
                .selected_text(if self.place_slot == 0 {
                    "any unit".to_string()
                } else {
                    format!("slot {}", self.place_slot)
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.place_slot, 0, "any unit");
                    for s in 1..=6u8 {
                        ui.selectable_value(&mut self.place_slot, s, format!("slot {s}"));
                    }
                });
            if ui.button("Undo").clicked() {
                self.placements.undo(&self.spot_key());
                self.save_placements();
            }
            if ui.button("Clear map").clicked() {
                self.placements.clear(&self.spot_key());
                self.save_placements();
            }
            if ui
                .button("Hover test")
                .on_hover_text("moves the mouse over each spot on the selected client - no clicks")
                .clicked()
            {
                self.hover_test();
            }
        });
        self.placers_ui(ui);
        ui.horizontal(|ui| {
            if super::toggle(
                ui,
                &mut self.settings.spiral_fallback,
                "Shape when custom spots run out",
            ) {
                self.dirty();
            }
            let cur = self.settings.place_shape;
            ComboBox::from_id_salt("shape")
                .selected_text(cur.label())
                .show_ui(ui, |ui| {
                    for sh in crate::game::placing::Shape::ALL {
                        ui.selectable_value(&mut self.settings.place_shape, sh, sh.label());
                    }
                });
            if self.settings.place_shape != cur {
                self.dirty();
            }
            super::toggle(ui, &mut self.show_spiral, "show shape");
            ui.label("delay");
            if ui
                .add(
                    DragValue::new(&mut self.settings.place_delay_ms)
                        .range(0..=10000)
                        .suffix(" ms"),
                )
                .on_hover_text("pause between placements (2000 = 2 s)")
                .changed()
            {
                self.dirty();
            }
            if ui
                .button("Camera top-down")
                .on_hover_text("zoom in, look down, zoom out on the selected client (in a stage)")
                .clicked()
            {
                self.camera_test();
            }
            if ui
                .button("Test place units")
                .on_hover_text(
                    "places your enabled units on the selected client's spots (in a stage)",
                )
                .clicked()
            {
                self.place_test();
            }
            if ui
                .button("Read unit level")
                .on_hover_text(
                    "open a unit's panel in Roblox first - reads its UPG [n] with Windows OCR",
                )
                .clicked()
            {
                self.read_level_test();
            }
        });
        ui.small("Left click = add spot. Right click = remove nearest. Each placing client has its own spots (pick it in the client box above the picture). Units fill spots in number order (reserved slot spots first), then the spiral.");
        let spots = self.placements.spots(&self.spot_key()).to_vec();
        let n = spots.len();
        if let Some((resp, rect, scale)) = self.client_view(ui) {
            let painter = ui.painter_at(rect);
            for (i, s) in spots.iter().enumerate() {
                let c = rect.min + Vec2::new(s.x as f32, s.y as f32) * scale;
                let col = if s.slot == 0 {
                    ACCENT
                } else {
                    slot_color(s.slot)
                };
                painter.circle_filled(c, 12.0, Color32::from_black_alpha(190));
                painter.circle_stroke(c, 12.0, Stroke::new(2.5, col));
                painter.circle_filled(c, 2.0, col);
                painter.text(
                    c + Vec2::new(0.0, -1.0),
                    Align2::CENTER_BOTTOM,
                    format!("{}", i + 1),
                    FontId::proportional(12.0),
                    Color32::WHITE,
                );
                if s.slot != 0 {
                    painter.text(
                        c + Vec2::new(11.0, -11.0),
                        Align2::LEFT_BOTTOM,
                        format!("s{}", s.slot),
                        FontId::proportional(10.0),
                        col,
                    );
                }
            }
            if self.show_spiral {
                for (i, p) in crate::game::placing::shape_spots(
                    self.settings.place_shape,
                    40,
                    self.sel as u64 + 1,
                )
                .iter()
                .enumerate()
                {
                    let c = rect.min + Vec2::new(p.x as f32, p.y as f32) * scale;
                    painter.circle_stroke(c, 5.0, Stroke::new(1.0, Color32::from_white_alpha(140)));
                    if i < 9 {
                        painter.text(
                            c,
                            Align2::CENTER_CENTER,
                            format!("{}", i + 1),
                            FontId::proportional(9.0),
                            Color32::WHITE,
                        );
                    }
                }
            }
            if let Some(pos) = resp.interact_pointer_pos() {
                let p = to_client(pos, rect, scale);
                if resp.clicked() {
                    self.placements.add(&self.spot_key(), p, self.place_slot);
                    self.save_placements();
                } else if resp.secondary_clicked() {
                    self.placements.remove_near(&self.spot_key(), p, 14);
                    self.save_placements();
                }
            }
        }
        ui.label(format!(
            "{n} custom spot(s) for {} on {}",
            self.client_label(self.sel),
            self.map
        ));
    }

    /// Spot-set key for the map + client chosen on the Placement page.
    pub(super) fn spot_key(&self) -> String {
        crate::game::placing::spots_key(&self.map, self.sel + 1)
    }

    pub(super) fn camera_test(&mut self) {
        let Some(info) = self.clients.get(self.sel).cloned() else {
            return;
        };
        let b = crate::game::placing::camera_batch(info.hwnd, &self.settings.camera);
        let arb = self.arbiter();
        let tx = self.log_tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(match arb.run(b) {
                Ok(_) => "camera set top-down".into(),
                Err(e) => format!("camera: {e}"),
            });
        });
    }

    /// Place every enabled unit on the selected client (custom spots, then spiral).
    pub(super) fn place_test(&mut self) {
        use crate::game::placing::{place_slot, Keys};
        let Some(info) = self.clients.get(self.sel).cloned() else {
            return;
        };
        let keys = match Keys::from(&self.settings.keybinds) {
            Ok(k) => k,
            Err(e) => {
                self.say(e);
                return;
            }
        };
        let key = self.spot_key();
        let mut jobs = vec![];
        for (i, u) in self
            .settings
            .units
            .iter()
            .enumerate()
            .filter(|(_, u)| u.enabled && u.place > 0)
        {
            let slot = i + 1;
            let mut spots: Vec<Point> = self
                .placements
                .for_slot(&key, slot as u8)
                .iter()
                .map(|s| Point::new(s.x, s.y))
                .collect();
            if self.settings.spiral_fallback {
                spots.extend(crate::game::placing::shape_spots(
                    self.settings.place_shape,
                    60,
                    self.sel as u64 * 7 + slot as u64,
                ));
            }
            jobs.push((slot, u.place as u32, spots));
        }
        if jobs.is_empty() {
            self.say("turn on at least one unit (Home > Control) with a placement count");
            return;
        }
        let screen = crate::game::LiveScreen::new(self.platform.clone(), info.hwnd, &self.lib);
        let has_panel = self.lib.get("unit_placed").is_some();
        let delay = self.settings.place_delay_ms;
        let arb = self.arbiter();
        let tx = self.log_tx.clone();
        std::thread::spawn(move || {
            let confirm =
                || has_panel.then(|| screen.find(&["unit_placed".to_string()], None).is_some());
            let mut used: Vec<Point> = vec![];
            for (slot, count, spots) in jobs {
                let free: Vec<Point> = spots.into_iter().filter(|p| !used.contains(p)).collect();
                match place_slot(
                    &arb,
                    info.hwnd,
                    &keys,
                    slot,
                    count,
                    &free,
                    &confirm,
                    delay,
                    &|| true,
                    &|_| {},
                ) {
                    Ok(done) => {
                        let _ = tx.send(format!("slot {slot}: placed {}/{count}", done.len()));
                        used.extend(done);
                    }
                    Err(e) => {
                        let _ = tx.send(format!("slot {slot}: stopped - {e}"));
                        return;
                    }
                }
            }
        });
    }

    pub(super) fn save_placements(&mut self) {
        if let Err(e) = self.placements.save(&self.placements_path) {
            self.say(format!("saving placements failed: {e}"));
        }
    }

    pub(super) fn hover_test(&mut self) {
        let Some(info) = self.clients.get(self.sel).cloned() else {
            self.say("no client selected");
            return;
        };
        let spots = self.placements.spots(&self.spot_key()).to_vec();
        let arb = self.arbiter();
        let tx = self.log_tx.clone();
        std::thread::spawn(move || {
            for (i, s) in spots.iter().enumerate() {
                let b = Batch::new(info.hwnd, "hover-test")
                    .move_to(s.x, s.y)
                    .wait(350);
                if let Err(e) = arb.run(b) {
                    let _ = tx.send(format!("hover test stopped at spot {}: {e}", i + 1));
                    return;
                }
            }
            let _ = tx.send(format!("hover test done ({} spots)", spots.len()));
        });
    }

    pub(super) fn page_patterns(&mut self, ui: &mut egui::Ui) {
        ui.heading("Patterns");
        ui.small("Drag a box around button text / an icon on the picture, name it, Save. Close pop-ups first.");
        egui::Frame::new().fill(CARD_HI).corner_radius(10).inner_margin(10).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new("Hover capture (F6)").strong());
            ui.label("In Roblox, put the mouse on the middle of a button's text and press F6. The label is found and saved as the next missing name below (or the name you typed). No dragging.");
            match self.next_missing() {
                Some(n) => ui.label(egui::RichText::new(format!("Next: {n}")).color(ACCENT).strong()),
                None => ui.label(egui::RichText::new("Checklist complete").color(GOOD)),
            };
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.strong("Checklist - capture these names");
            if ui
                .small_button("Export patterns (zip)")
                .on_hover_text("patterns-export.zip next to the exe - send it to bake your captures into the next build")
                .clicked()
            {
                let to = Library::default_dir()
                    .parent()
                    .map(|d| d.join("patterns-export.zip"))
                    .unwrap_or_else(|| "patterns-export.zip".into());
                match self.lib.export_zip(&to) {
                    Ok(n) => self.say(format!("exported {n} pattern(s) to {}", to.display())),
                    Err(e) => self.say(format!("export failed: {e:#}")),
                }
            }
        });
        let mut pick: Option<(String, bool)> = None;
        let mine = crate::flows::stage_needs(
            self.settings.run.mode,
            &self.settings.run.map,
            &self.settings.run.act,
        );
        if !mine.is_empty() {
            let have = mine
                .iter()
                .filter(|(n, _)| self.lib.get(n).is_some())
                .count();
            let col = if have == mine.len() { GOOD } else { WARN };
            egui::CollapsingHeader::new(egui::RichText::new(format!("Your stage ({})  {}/{}", self.settings.run.mode.label(), have, mine.len())).color(col))
                .id_salt("your-stage")
                .default_open(true)
                .show(ui, |ui| {
                    ui.small("Pick the map + act on Home first. Open the Select World list, then capture the world title and the act button.");
                    for (name, hint) in &mine {
                        ui.horizontal(|ui| {
                            let ok = self.lib.get(name).is_some();
                            ui.colored_label(if ok { GOOD } else { WARN }, if ok { "OK " } else { "missing" });
                            ui.monospace(name);
                            ui.small(format!("(text) {hint}"));
                            if ui.small_button(if ok { "redo" } else { "capture" }).clicked() {
                                pick = Some((name.clone(), false));
                            }
                        });
                    }
                });
        }
        for flow in FLOWS {
            let have = flow
                .needs
                .iter()
                .filter(|n| self.lib.get(n.name).is_some())
                .count();
            let col = if have == flow.needs.len() { GOOD } else { WARN };
            egui::CollapsingHeader::new(
                egui::RichText::new(format!("{}  {}/{}", flow.name, have, flow.needs.len()))
                    .color(col),
            )
            .id_salt(flow.name)
            .show(ui, |ui| {
                ui.small(flow.steps);
                for need in flow.needs {
                    ui.horizontal(|ui| {
                        let ok = self.lib.get(need.name).is_some();
                        ui.colored_label(
                            if ok { GOOD } else { WARN },
                            if ok { "OK " } else { "missing" },
                        );
                        ui.monospace(need.name);
                        ui.small(format!("({}) {}", need.kind, need.hint));
                        if ui
                            .small_button(if ok { "redo" } else { "capture" })
                            .clicked()
                        {
                            pick = Some((need.name.to_string(), need.kind == "icon"));
                        }
                    });
                }
            });
        }
        {
            let mut maps: Vec<String> = self.routes.maps.keys().cloned().collect();
            for p in self.lib.names() {
                if p.starts_with("loading_") {
                    let m = crate::game::run::map_from_pattern(&p);
                    if !maps.iter().any(|x| vision::key(x) == vision::key(&m)) {
                        maps.push(m);
                    }
                }
            }
            let needs = crate::flows::map_needs(&maps);
            let have = needs
                .iter()
                .filter(|(n, _)| self.lib.get(n).is_some())
                .count();
            egui::CollapsingHeader::new(
                egui::RichText::new(format!("Map detection (random maps)  {have}/{}", needs.len()))
                    .color(if have > 0 { GOOD } else { WARN }),
            )
            .id_salt("map-detect")
            .show(ui, |ui| {
                ui.small("The Summer Event changes map every hour. While a stage loads the macro looks for these; the one it sees picks that map's walk and your spots for it. Capture the map name on the loading screen (F6 on it) for each map you meet. A map can have a 2nd look: name it loading_<map>_2. Not recognised = stays at spawn and spiral-places.");
                ui.horizontal(|ui| {
                    ui.label("Map not listed:");
                    ui.add(egui::TextEdit::singleline(&mut self.detect_map).desired_width(160.0).hint_text("e.g. Summer Beach"));
                    if ui.small_button("use name").clicked() && !self.detect_map.trim().is_empty() {
                        pick = Some((crate::game::run::loading_pattern(self.detect_map.trim()), false));
                        self.detect_map.clear();
                    }
                });
                for (name, hint) in &needs {
                    ui.horizontal(|ui| {
                        let ok = self.lib.get(name).is_some();
                        ui.colored_label(if ok { GOOD } else { MUTED }, if ok { "OK " } else { "-" });
                        ui.monospace(name);
                        ui.small(hint);
                        if ui.small_button(if ok { "redo" } else { "capture" }).clicked() {
                            pick = Some((name.clone(), false));
                        }
                    });
                }
            });
        }
        if let Some((name, icon)) = pick {
            self.pat_name = name.clone();
            self.pat_image = icon;
            self.say(format!(
                "name set to '{name}' - F6 over it in Roblox, or drag a box on the picture and Save pattern"
            ));
        }
        ui.separator();

        let mut redo_preview = false;
        ui.horizontal(|ui| {
            ui.label("Name:");
            ui.add(
                egui::TextEdit::singleline(&mut self.pat_name)
                    .desired_width(160.0)
                    .hint_text("Vote Start"),
            );
            redo_preview |= ui.radio_value(&mut self.pat_image, false, "Text").changed();
            redo_preview |= ui
                .radio_value(&mut self.pat_image, true, "Colour icon")
                .changed();
            ui.label("Search margin");
            ui.add(DragValue::new(&mut self.pat_margin).range(0..=400));
        });
        if !self.pat_image {
            ui.horizontal(|ui| {
                redo_preview |= ui.checkbox(&mut self.pat_auto, "auto threshold").changed();
                redo_preview |= ui
                    .add_enabled(!self.pat_auto, Slider::new(&mut self.pat_thr, 0..=255))
                    .changed();
            });
        }
        ui.horizontal(|ui| {
            let can =
                self.crop.is_some() && self.frame.is_some() && !self.pat_name.trim().is_empty();
            if ui
                .add_enabled(can, egui::Button::new("Save pattern"))
                .clicked()
            {
                self.save_pattern();
            }
            if ui.button("Test all").clicked() {
                self.test_patterns(None);
            }
            if let Some(c) = self.crop {
                ui.monospace(format!("box {},{},{},{}", c.x, c.y, c.w, c.h));
            }
        });
        if let Some(t) = &self.pat_preview {
            ui.horizontal(|ui| {
                ui.label("Preview:");
                let s = t.size_vec2();
                let k = (60.0 / s.y).clamp(0.5, 4.0);
                ui.image((t.id(), s * k));
            });
        }
        if let Some((resp, rect, scale)) = self.client_view(ui) {
            if resp.drag_started() {
                self.drag_from = resp
                    .interact_pointer_pos()
                    .map(|p| to_client(p, rect, scale));
            }
            if resp.dragged() {
                if let (Some(a), Some(p)) = (self.drag_from, resp.interact_pointer_pos()) {
                    let b = to_client(p, rect, scale);
                    self.crop = Some(Rect::new(
                        a.x.min(b.x),
                        a.y.min(b.y),
                        (a.x - b.x).abs().max(2),
                        (a.y - b.y).abs().max(2),
                    ));
                }
            }
            if resp.drag_stopped() {
                self.drag_from = None;
                redo_preview = true;
            }
            let painter = ui.painter_at(rect);
            let to_screen = |r: Rect| {
                egui::Rect::from_min_size(
                    rect.min + Vec2::new(r.x as f32, r.y as f32) * scale,
                    Vec2::new(r.w as f32, r.h as f32) * scale,
                )
            };
            for (name, search, hit) in &self.hits {
                painter.rect_stroke(
                    to_screen(*search),
                    2.0,
                    Stroke::new(1.0, WARN),
                    StrokeKind::Outside,
                );
                if let Some(h) = hit {
                    let r = to_screen(Rect::new(h.at.x, h.at.y, h.w, h.h));
                    painter.rect_stroke(r, 2.0, Stroke::new(2.0, GOOD), StrokeKind::Outside);
                    painter.text(
                        r.left_top(),
                        Align2::LEFT_BOTTOM,
                        name,
                        FontId::proportional(11.0),
                        GOOD,
                    );
                }
            }
            if let Some(c) = self.crop {
                painter.rect_stroke(
                    to_screen(c),
                    0.0,
                    Stroke::new(2.0, ACCENT),
                    StrokeKind::Outside,
                );
            }
        }
        if redo_preview {
            let ctx = ui.ctx().clone();
            self.update_preview(&ctx);
        }
        ui.separator();
        ui.strong(format!("Saved ({})", self.lib.len()));
        let names: Vec<String> = self.lib.names().map(String::from).collect();
        let mut remove = None;
        for n in names {
            ui.horizontal(|ui| {
                let found = self.hits.iter().find(|h| h.0 == n).map(|h| h.2.is_some());
                let (txt, col) = match found {
                    Some(true) => ("FOUND", GOOD),
                    Some(false) => ("-", WARN),
                    None => ("", Color32::GRAY),
                };
                ui.label(&n);
                ui.colored_label(col, txt);
                if ui.small_button("test").clicked() {
                    self.test_patterns(Some(&n));
                }
                if ui.small_button("delete").clicked() {
                    remove = Some(n.clone());
                }
            });
        }
        if let Some(n) = remove {
            let _ = self.lib.remove(&n);
            self.hits.retain(|h| h.0 != n);
            self.say(format!("deleted pattern {n}"));
        }
    }

    pub(super) fn update_preview(&mut self, ctx: &egui::Context) {
        self.pat_preview = None;
        let (Some(f), Some(c)) = (&self.frame, self.crop) else {
            return;
        };
        if self.pat_image {
            let crop = f.crop(c.x.max(0) as u32, c.y.max(0) as u32, c.w as u32, c.h as u32);
            if crop.width == 0 || crop.height == 0 {
                return;
            }
            let img = ColorImage::from_rgba_unmultiplied(
                [crop.width as usize, crop.height as usize],
                &rgba(&crop),
            );
            self.pat_preview = Some(ctx.load_texture("pat-preview", img, TextureOptions::NEAREST));
            return;
        }
        let thr = (!self.pat_auto).then_some(self.pat_thr);
        if let Ok(p) = Pattern::text_from(f, "preview", c, 0, thr) {
            if self.pat_auto {
                self.pat_thr = p.threshold;
            }
            if let Some(b) = p.bit_image() {
                let pv = vision::preview(b);
                let img = ColorImage::from_rgba_unmultiplied(
                    [pv.width as usize, pv.height as usize],
                    &rgba(&pv),
                );
                self.pat_preview =
                    Some(ctx.load_texture("pat-preview", img, TextureOptions::NEAREST));
            }
        }
    }

    pub(super) fn save_pattern(&mut self) {
        let (Some(f), Some(c)) = (self.frame.clone(), self.crop) else {
            return;
        };
        let name = self.pat_name.trim().to_string();
        let made = if self.pat_image {
            Pattern::image_from(&f, &name, c, self.pat_margin)
        } else {
            Pattern::text_from(
                &f,
                &name,
                c,
                self.pat_margin,
                (!self.pat_auto).then_some(self.pat_thr),
            )
        };
        match made.and_then(|p| self.lib.save(p)) {
            Ok(path) => {
                self.say(format!("saved pattern '{name}' -> {}", path.display()));
                self.test_patterns(Some(&name));
                self.crop = None;
                self.pat_name.clear();
                self.pat_preview = None;
            }
            Err(e) => self.say(format!("save failed: {e}")),
        }
    }

    pub(super) fn test_patterns(&mut self, only: Option<&str>) {
        let Some(f) = self.frame.clone() else {
            self.say("capture a client first");
            return;
        };
        let names: Vec<String> = match only {
            Some(n) => vec![n.to_string()],
            None => self.lib.names().map(String::from).collect(),
        };
        for n in names {
            let Some(p) = self.lib.get(&n) else { continue };
            let t = Instant::now();
            let hit = p.find_in(&f, None);
            let us = t.elapsed().as_micros();
            let search = p.search;
            self.hits.retain(|h| vision::key(&h.0) != vision::key(&n));
            self.hits.push((n.clone(), search, hit));
            match hit {
                Some(h) => self.say(format!(
                    "{n}: FOUND at ({}, {}) score {:.2} [{us} us]",
                    h.at.x, h.at.y, h.score
                )),
                None => self.say(format!("{n}: not found [{us} us]")),
            }
        }
        if self.hits.is_empty() {
            self.say("no patterns saved yet");
        }
    }

    pub(super) fn client_view(
        &mut self,
        ui: &mut egui::Ui,
    ) -> Option<(egui::Response, egui::Rect, f32)> {
        ui.horizontal(|ui| {
            let labels: Vec<String> = self
                .clients
                .iter()
                .enumerate()
                .map(|(i, _c)| self.client_label(i))
                .collect();
            if !labels.is_empty() {
                ComboBox::from_id_salt("viewsel")
                    .selected_text(labels[self.sel.min(labels.len() - 1)].clone())
                    .show_ui(ui, |ui| {
                        for (i, l) in labels.iter().enumerate() {
                            ui.selectable_value(&mut self.sel, i, l);
                        }
                    });
            }
            if ui.button("Capture").clicked() {
                let ctx = ui.ctx().clone();
                self.capture(&ctx);
            }
            ui.checkbox(&mut self.live, "live");
        });
        let Some(tex) = &self.tex else {
            ui.colored_label(WARN, "Press Capture to see the client here.");
            return None;
        };
        let size = tex.size_vec2();
        let scale = (ui.available_width() / size.x).min(1.0);
        let (resp, painter) = ui.allocate_painter(size * scale, Sense::click_and_drag());
        let rect = resp.rect;
        painter.image(
            tex.id(),
            rect,
            egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        if let Some(p) = resp.hover_pos() {
            let c = to_client(p, rect, scale);
            painter.text(
                rect.right_bottom() - Vec2::new(4.0, 4.0),
                Align2::RIGHT_BOTTOM,
                format!("{}, {}", c.x, c.y),
                FontId::monospace(12.0),
                Color32::YELLOW,
            );
        }
        Some((resp, rect, scale))
    }
}

fn to_client(p: Pos2, rect: egui::Rect, scale: f32) -> Point {
    let v = (p - rect.min) / scale;
    Point::new(v.x.round() as i32, v.y.round() as i32)
}

pub(super) fn rgba(f: &Frame) -> Vec<u8> {
    let mut v = f.bgra.clone();
    for p in v.chunks_exact_mut(4) {
        p.swap(0, 2);
        p[3] = 255;
    }
    v
}

fn slot_color(slot: u8) -> Color32 {
    match slot {
        1 => Color32::from_rgb(240, 90, 90),
        2 => Color32::from_rgb(240, 170, 60),
        3 => Color32::from_rgb(230, 220, 70),
        4 => Color32::from_rgb(90, 200, 120),
        5 => Color32::from_rgb(80, 200, 220),
        _ => Color32::from_rgb(190, 120, 240),
    }
}
