//! The app window: Home (clients), Units, Placement designer, Patterns, Settings.

use crate::capture::Frame;
use crate::config::{Mode, Role, Settings, Upgrade};
use crate::control::Control;
use crate::geom::{Point, Rect};
use crate::input::{Arbiter, ArbiterHandle, ArbiterOptions, Batch};
use crate::layout;
use crate::placement::Placements;
use crate::platform::{Platform, WinInfo};
use crate::vision::{self, Hit, Library, Pattern};
use eframe::egui::{
    self, Align2, Color32, ColorImage, ComboBox, DragValue, FontId, Pos2, Sense, Slider, Stroke,
    StrokeKind, TextureHandle, TextureOptions, Vec2,
};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Units,
    Placement,
    Patterns,
    Settings,
}

const ACCENT: Color32 = Color32::from_rgb(120, 170, 255);
const GOOD: Color32 = Color32::from_rgb(90, 200, 120);
const WARN: Color32 = Color32::from_rgb(240, 170, 60);

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
    page: Page,

    // live picture of the selected client
    frame: Option<Frame>,
    tex: Option<TextureHandle>,
    live: bool,
    last_cap: Instant,

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

    log: Vec<String>,
    log_tx: Sender<String>,
    log_rx: Receiver<String>,
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
            .unwrap_or_else(|| "Summer Hunt".into());
        let (log_tx, log_rx) = mpsc::channel();
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
            page: Page::Home,
            frame: None,
            tex: None,
            live: false,
            last_cap: Instant::now() - Duration::from_secs(10),
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
            log: vec![],
            log_tx,
            log_rx,
        };
        app.refresh_clients();
        app.page = match std::env::var("AA_START_PAGE").as_deref() {
            Ok("units") => Page::Units,
            Ok("placement") => Page::Placement,
            Ok("patterns") => Page::Patterns,
            Ok("settings") => Page::Settings,
            _ => Page::Home,
        };
        if app.page == Page::Placement || app.page == Page::Patterns {
            app.live = true; // shows the picture straight away
        }
        app
    }

    fn say(&mut self, s: impl Into<String>) {
        let s = s.into();
        log::info!("{s}");
        self.log.push(s);
        if self.log.len() > 300 {
            self.log.drain(..100);
        }
    }

    fn dirty(&mut self) {
        self.dirty_at = Some(Instant::now());
    }

    fn refresh_clients(&mut self) {
        let mut w = self.platform.list_windows(&self.settings.process_name);
        w.truncate(self.settings.max_instances);
        self.settings
            .ensure(w.len().max(self.settings.max_instances));
        self.clients = w;
        if self.sel >= self.clients.len() {
            self.sel = 0;
        }
        let n = self.clients.len();
        self.say(format!("found {n} Roblox client(s)"));
    }

    fn arbiter(&mut self) -> ArbiterHandle {
        if self.arbiter.is_none() {
            let s = &self.settings;
            self.arbiter = Some(Arbiter::start(
                self.platform.clone(),
                self.control.clone(),
                ArbiterOptions {
                    mouse_settle_ms: s.mouse_settle_ms,
                    click_hold_ms: s.click_hold_ms,
                    max_lease_ms: s.max_lease_ms,
                },
            ));
        }
        self.arbiter.as_ref().unwrap().handle()
    }

    fn capture(&mut self, ctx: &egui::Context) {
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
                self.live = false;
                self.say(format!("capture failed: {e}"));
            }
        }
    }

    fn tile(&mut self) {
        let p = self.platform.clone();
        let Some(first) = self.clients.first().and_then(|c| p.geometry(c.hwnd)) else {
            self.say("no clients to tile");
            return;
        };
        let (ow, oh) = first.outer_for_client(self.settings.client_w, self.settings.client_h);
        let rects = layout::tile(self.clients.len(), p.work_area(), ow, oh);
        for (c, r) in self.clients.clone().iter().zip(rects) {
            if let Err(e) = p.move_resize(c.hwnd, r) {
                self.say(format!("tile failed: {e}"));
            }
            if let Some(g) = p.geometry(c.hwnd) {
                if (g.client.w, g.client.h) != (self.settings.client_w, self.settings.client_h) {
                    let (w, h) = g.outer_for_client(self.settings.client_w, self.settings.client_h);
                    let _ = p.move_resize(c.hwnd, Rect::new(g.outer.x, g.outer.y, w, h));
                }
            }
        }
        self.say(format!("tiled {} client(s)", self.clients.len()));
    }

    fn doctor(&mut self) {
        let p = self.platform.clone();
        for (i, c) in self.clients.clone().iter().enumerate() {
            let n = i + 1;
            let dpi = p.dpi(c.hwnd);
            let size = p.geometry(c.hwnd).map(|g| (g.client.w, g.client.h));
            let cap = p.capture_client(c.hwnd);
            let mut bad = vec![];
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

    fn page_home(&mut self, ui: &mut egui::Ui) {
        ui.heading("Clients");
        ui.horizontal(|ui| {
            if ui.button("Refresh").clicked() {
                self.refresh_clients();
            }
            if ui
                .button("Tile")
                .on_hover_text("800x599 viewport each, laid out on the main screen")
                .clicked()
            {
                self.tile();
            }
            if ui
                .button("Doctor")
                .on_hover_text("check scaling, size and capture")
                .clicked()
            {
                self.doctor();
            }
        });
        ui.add_space(6.0);
        if self.clients.is_empty() {
            ui.colored_label(
                WARN,
                "No Roblox clients found. Open Roblox (windowed) and press Refresh.",
            );
        }
        let mut changed = false;
        egui::Grid::new("clients")
            .striped(true)
            .spacing([14.0, 8.0])
            .show(ui, |ui| {
                for h in ["#", "Name", "Role", "Mode", "Map / act", "On"] {
                    ui.strong(h);
                }
                ui.end_row();
                for i in 0..self.clients.len() {
                    let prof = &mut self.settings.instances[i];
                    ui.label(format!("{}", i + 1));
                    changed |= ui
                        .add_sized([120.0, 22.0], egui::TextEdit::singleline(&mut prof.label))
                        .changed();
                    ComboBox::from_id_salt(("role", i))
                        .width(110.0)
                        .selected_text(role_label(&prof.role))
                        .show_ui(ui, |ui| {
                            for r in [Role::Independent, Role::Host, Role::Follower] {
                                let l = role_label(&r);
                                changed |= ui.selectable_value(&mut prof.role, r, l).changed();
                            }
                        });
                    ComboBox::from_id_salt(("mode", i))
                        .width(130.0)
                        .selected_text(prof.mode.label())
                        .show_ui(ui, |ui| {
                            for m in Mode::ALL {
                                changed |=
                                    ui.selectable_value(&mut prof.mode, m, m.label()).changed();
                            }
                        });
                    changed |= ui
                        .add_sized(
                            [170.0, 22.0],
                            egui::TextEdit::singleline(&mut prof.map)
                                .hint_text("e.g. Planet Greenie"),
                        )
                        .changed();
                    changed |= ui.checkbox(&mut prof.enabled, "").changed();
                    ui.end_row();
                }
            });
        if changed {
            self.dirty();
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_enabled(false, egui::Button::new("▶ Start all"))
                .on_disabled_hover_text("The game loop arrives in Phase 3");
            ui.add_enabled(false, egui::Button::new("■ Stop"));
        });
        ui.small("Independent = farms on its own. Host = runs the stage, waits for followers. Follower = only joins the host's lobby.");
    }

    fn page_units(&mut self, ui: &mut egui::Ui) {
        ui.heading("Units (hotbar slots 1-6)");
        ui.small(
            "Shared by every client for now. Spots for each map are set on the Placement page.",
        );
        ui.add_space(6.0);
        let mut changed = false;
        egui::Grid::new("units")
            .striped(true)
            .spacing([14.0, 8.0])
            .show(ui, |ui| {
                for h in ["Slot", "On", "Name", "Place", "Upgrade", "Priority", "Buff"] {
                    ui.strong(h);
                }
                ui.end_row();
                for (i, u) in self.settings.units.iter_mut().enumerate().take(6) {
                    ui.label(format!("{}", i + 1));
                    changed |= ui.checkbox(&mut u.enabled, "").changed();
                    changed |= ui
                        .add_sized(
                            [130.0, 22.0],
                            egui::TextEdit::singleline(&mut u.name).hint_text("e.g. Erwin"),
                        )
                        .changed();
                    changed |= ui.add(DragValue::new(&mut u.place).range(0..=10)).changed();
                    let cur = match u.upgrade {
                        Upgrade::None => "None".to_string(),
                        Upgrade::Max => "Max".to_string(),
                        Upgrade::Level(n) => format!("Level {n}"),
                    };
                    ui.horizontal(|ui| {
                        ComboBox::from_id_salt(("upg", i))
                            .width(80.0)
                            .selected_text(cur)
                            .show_ui(ui, |ui| {
                                changed |= ui
                                    .selectable_value(&mut u.upgrade, Upgrade::None, "None")
                                    .changed();
                                changed |= ui
                                    .selectable_value(&mut u.upgrade, Upgrade::Max, "Max")
                                    .changed();
                                if ui
                                    .selectable_label(
                                        matches!(u.upgrade, Upgrade::Level(_)),
                                        "Level…",
                                    )
                                    .clicked()
                                {
                                    u.upgrade = Upgrade::Level(5);
                                    changed = true;
                                }
                            });
                        if let Upgrade::Level(n) = &mut u.upgrade {
                            changed |= ui.add(DragValue::new(n).range(1..=20)).changed();
                        }
                    });
                    changed |= ui
                        .add(DragValue::new(&mut u.priority).range(1..=6))
                        .changed();
                    changed |= ui
                        .checkbox(&mut u.buff, "")
                        .on_hover_text("Turn Auto ability on one copy at a time")
                        .changed();
                    ui.end_row();
                }
            });
        if changed {
            self.dirty();
        }
    }

    fn page_placement(&mut self, ui: &mut egui::Ui) {
        ui.heading("Placement designer");
        ui.horizontal(|ui| {
            ui.label("Map:");
            let maps: Vec<String> = self.placements.maps.keys().cloned().collect();
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
                self.placements.undo(&self.map);
                self.save_placements();
            }
            if ui.button("Clear map").clicked() {
                self.placements.clear(&self.map);
                self.placements.maps.entry(self.map.clone()).or_default();
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
        ui.small("Left click = add spot. Right click = remove nearest. Units fill spots in number order (reserved slot spots first).");
        let spots = self.placements.spots(&self.map).to_vec();
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
            if let Some(pos) = resp.interact_pointer_pos() {
                let p = to_client(pos, rect, scale);
                if resp.clicked() {
                    self.placements.add(&self.map, p, self.place_slot);
                    self.save_placements();
                } else if resp.secondary_clicked() {
                    self.placements.remove_near(&self.map, p, 14);
                    self.save_placements();
                }
            }
        }
        ui.label(format!("{n} spot(s) on {}", self.map));
    }

    fn save_placements(&mut self) {
        if let Err(e) = self.placements.save(&self.placements_path) {
            self.say(format!("saving placements failed: {e}"));
        }
    }

    fn hover_test(&mut self) {
        let Some(info) = self.clients.get(self.sel).cloned() else {
            self.say("no client selected");
            return;
        };
        let spots = self.placements.spots(&self.map).to_vec();
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

    fn page_patterns(&mut self, ui: &mut egui::Ui) {
        ui.heading("Patterns");
        ui.small("Drag a box around button text / an icon on the picture, name it, Save. Close pop-ups first.");
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

    fn update_preview(&mut self, ctx: &egui::Context) {
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

    fn save_pattern(&mut self) {
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

    fn test_patterns(&mut self, only: Option<&str>) {
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

    fn page_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        let s = &mut self.settings;
        let mut changed = false;
        egui::Grid::new("settings")
            .spacing([14.0, 8.0])
            .show(ui, |ui| {
                ui.label("Roblox process");
                changed |= ui.text_edit_singleline(&mut s.process_name).changed();
                ui.end_row();
                ui.label("Max clients");
                changed |= ui
                    .add(DragValue::new(&mut s.max_instances).range(1..=4))
                    .changed();
                ui.end_row();
                ui.label("Client size");
                ui.label(format!(
                    "{} x {} (patterns are captured at this size)",
                    s.client_w, s.client_h
                ));
                ui.end_row();
                ui.label("Mouse settle (ms)");
                changed |= ui
                    .add(DragValue::new(&mut s.mouse_settle_ms).range(0..=500))
                    .changed();
                ui.end_row();
                ui.label("Click hold (ms)");
                changed |= ui
                    .add(DragValue::new(&mut s.click_hold_ms).range(10..=500))
                    .changed();
                ui.end_row();
                ui.label("Max input turn (ms)");
                changed |= ui
                    .add(DragValue::new(&mut s.max_lease_ms).range(100..=2000))
                    .changed();
                ui.end_row();
            });
        if changed {
            self.dirty();
        }
        ui.add_space(8.0);
        ui.small(format!("Settings: {}", self.settings_path.display()));
        ui.small(format!("Patterns: {}", self.lib.dir.display()));
        ui.small(format!("Placements: {}", self.placements_path.display()));
    }

    /// The selected client's picture at 1:1 (or scaled to fit). Returns the
    /// response, the screen rect of the picture and the scale.
    fn client_view(&mut self, ui: &mut egui::Ui) -> Option<(egui::Response, egui::Rect, f32)> {
        ui.horizontal(|ui| {
            let labels: Vec<String> = self
                .clients
                .iter()
                .enumerate()
                .map(|(i, c)| client_label(i, c, &self.settings))
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

fn rgba(f: &Frame) -> Vec<u8> {
    let mut v = f.bgra.clone();
    for p in v.chunks_exact_mut(4) {
        p.swap(0, 2);
        p[3] = 255;
    }
    v
}

fn role_label(r: &Role) -> &'static str {
    match r {
        Role::Independent => "Independent",
        Role::Host => "Party host",
        Role::Follower => "Party follower",
    }
}

fn client_label(i: usize, _c: &WinInfo, s: &Settings) -> String {
    let name = s.instances.get(i).map(|p| p.label.as_str()).unwrap_or("");
    format!("#{} {}", i + 1, name)
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

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        while let Ok(m) = self.log_rx.try_recv() {
            self.say(m);
        }
        if self.live && self.last_cap.elapsed() > Duration::from_millis(300) {
            self.capture(&ctx);
        }
        if self.live {
            ctx.request_repaint_after(Duration::from_millis(300));
        }
        if let Some(t) = self.dirty_at {
            if t.elapsed() > Duration::from_millis(600) {
                self.dirty_at = None;
                if let Err(e) = self.settings.save(&self.settings_path) {
                    self.say(format!("saving settings failed: {e}"));
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(650));
            }
        }

        egui::Panel::left("nav").exact_size(150.0).show(ui, |ui| {
            ui.add_space(8.0);
            ui.heading("AA Macro");
            ui.small(env!("CARGO_PKG_VERSION"));
            ui.add_space(10.0);
            for (p, l) in [
                (Page::Home, "Home"),
                (Page::Units, "Units"),
                (Page::Placement, "Placement"),
                (Page::Patterns, "Patterns"),
                (Page::Settings, "Settings"),
            ] {
                if ui.selectable_label(self.page == p, l).clicked() {
                    self.page = p;
                }
            }
            ui.add_space(12.0);
            ui.separator();
            let n = self.clients.len();
            ui.colored_label(if n > 0 { GOOD } else { WARN }, format!("{n} client(s)"));
        });
        egui::Panel::bottom("log")
            .resizable(true)
            .default_size(110.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for l in &self.log {
                            ui.monospace(l);
                        }
                    });
            });
        egui::CentralPanel::default_margins().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| match self.page {
                Page::Home => self.page_home(ui),
                Page::Units => self.page_units(ui),
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
    let opts = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title("AA Macro")
            .with_inner_size([1060.0, 820.0])
            .with_min_inner_size([820.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "AA Macro",
        opts,
        Box::new(move |_cc| Ok(Box::new(App::new(platform, settings, settings_path)))),
    )
    .map_err(|e| anyhow::anyhow!("window failed: {e}"))
}
