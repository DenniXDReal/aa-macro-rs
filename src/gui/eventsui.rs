//! Settings > Custom Events: your "when this text shows, do that" rules
//! (custom_events.json). Areas can be dragged on the OCR areas page.

use super::*;
use crate::game::events::{self, Do, Match, Rule, When};

impl App {
    /// The rules, loaded the first time they're needed.
    pub(super) fn event_rules(&mut self) -> &mut events::Rules {
        self.event_rules.get_or_insert_with(events::load)
    }

    pub(super) fn save_event_rules(&mut self) {
        if let Some(r) = &self.event_rules {
            if let Err(e) = events::save(r) {
                self.say(format!("custom events not saved: {e:#}"));
            }
        }
    }

    pub(super) fn events_ui(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(egui::RichText::new("CUSTOM EVENTS").size(11.0).color(MUTED));
            ui.label(
                egui::RichText::new("Your own rules: when the text shows in an area, do something. Each rule fires once, then waits for the text to go away (and at most once a second). Saved to custom_events.json as you edit; the running macro picks changes up by itself. Step jumps work with the step machine (Settings > General).")
                    .size(11.0)
                    .color(MUTED),
            );
        });
        ui.add_space(6.0);
        let mut changed = false;
        let mut pick_area: Option<usize> = None;
        let mut delete: Option<usize> = None;
        {
            let rules = self.event_rules();
            ui.horizontal(|ui| {
                ui.label("pause between typed characters");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut rules.type_gap_ms)
                            .range(1..=500)
                            .suffix(" ms"),
                    )
                    .changed();
                if ui.button("+ Add rule").clicked() {
                    rules.rules.push(Rule::default());
                    changed = true;
                }
            });
            for (i, r) in rules.rules.iter_mut().enumerate() {
                ui.add_space(4.0);
                card(ui, |ui| {
                    changed |= rule_ui(ui, i, r, &mut pick_area, &mut delete);
                });
            }
        }
        if let Some(i) = delete {
            self.event_rules().rules.remove(i);
            changed = true;
        }
        if changed {
            self.save_event_rules();
        }
        if let Some(i) = pick_area {
            self.ocr_ui.sel = format!("event:{i}");
            self.page = Page::OcrAreas;
        }
    }
}

fn rule_ui(
    ui: &mut egui::Ui,
    i: usize,
    r: &mut Rule,
    pick_area: &mut Option<usize>,
    delete: &mut Option<usize>,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        changed |= ui.checkbox(&mut r.enabled, "").changed();
        changed |= ui
            .add(egui::TextEdit::singleline(&mut r.name).desired_width(200.0))
            .changed();
        if ui.button("Delete").clicked() {
            *delete = Some(i);
        }
    });
    // trigger
    ui.horizontal(|ui| {
        ui.label("When");
        let mut k = r.when.kind();
        let mut secs = match r.when {
            When::Every(n) => n,
            _ => 5,
        };
        egui::ComboBox::from_id_salt(("ev_when", i))
            .selected_text(When::KINDS[k])
            .show_ui(ui, |ui| {
                for (j, l) in When::KINDS.iter().enumerate() {
                    ui.selectable_value(&mut k, j, *l);
                }
            });
        if k == 4 {
            ui.add(egui::DragValue::new(&mut secs).range(1..=3600).suffix(" s"));
        }
        let w = When::from_kind(k, secs);
        if w != r.when {
            r.when = w;
            changed = true;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Only on maps");
        let mut t = r.maps.join(", ");
        if ui
            .add(
                egui::TextEdit::singleline(&mut t)
                    .desired_width(180.0)
                    .hint_text("all"),
            )
            .changed()
        {
            r.maps = split(&t);
            changed = true;
        }
        ui.label("modes");
        let mut t = r.modes.join(", ");
        if ui
            .add(
                egui::TextEdit::singleline(&mut t)
                    .desired_width(160.0)
                    .hint_text("all (e.g. Summer Event)"),
            )
            .changed()
        {
            r.modes = split(&t);
            changed = true;
        }
    });
    // OCR condition
    ui.horizontal(|ui| {
        ui.label("Look at");
        for (v, l) in r.area.iter_mut().zip(["x", "y", "w", "h"]) {
            changed |= ui
                .add(
                    egui::DragValue::new(v)
                        .range(0..=4000)
                        .prefix(format!("{l} ")),
                )
                .changed();
        }
        if ui
            .button("Drag on the picture")
            .on_hover_text("opens the OCR areas page with this rule's area")
            .clicked()
        {
            *pick_area = Some(i);
        }
    });
    ui.horizontal(|ui| {
        ui.label("for the text");
        changed |= ui
            .add(
                egui::TextEdit::singleline(&mut r.text)
                    .desired_width(180.0)
                    .hint_text("empty = always"),
            )
            .changed();
        for (m, l) in [
            (Match::Exact, "Exact"),
            (Match::Contains, "Contains"),
            (Match::Regex, "Regex"),
        ] {
            changed |= ui.radio_value(&mut r.matching, m, l).changed();
        }
        if r.matching == Match::Regex && regex::Regex::new(&r.text).is_err() {
            ui.colored_label(WARN, "bad regex");
        }
    });
    // action
    ui.horizontal(|ui| {
        ui.label("Then");
        let mut k = r.action.kind();
        egui::ComboBox::from_id_salt(("ev_do", i))
            .selected_text(Do::KINDS[k])
            .show_ui(ui, |ui| {
                for (j, l) in Do::KINDS.iter().enumerate() {
                    ui.selectable_value(&mut k, j, *l);
                }
            });
        if k != r.action.kind() {
            r.action = Do::of_kind(k);
            changed = true;
        }
        match &mut r.action {
            Do::TypeText { text, chat } => {
                changed |= ui
                    .add(egui::TextEdit::singleline(text).desired_width(180.0))
                    .changed();
                changed |= ui
                    .checkbox(chat, "in chat (/ first, Enter after)")
                    .changed();
            }
            Do::PressKey { key } => {
                changed |= ui
                    .add(
                        egui::TextEdit::singleline(key)
                            .desired_width(80.0)
                            .hint_text("e, space, f1..."),
                    )
                    .changed();
                if crate::keys::Key::parse(key).is_none() {
                    ui.colored_label(WARN, "unknown key");
                }
            }
            Do::Click { x, y } => {
                changed |= ui.add(egui::DragValue::new(x).prefix("x ")).changed();
                changed |= ui.add(egui::DragValue::new(y).prefix("y ")).changed();
            }
            Do::ChangeStep { step } => {
                egui::ComboBox::from_id_salt(("ev_step", i))
                    .selected_text(step.as_str())
                    .show_ui(ui, |ui| {
                        for s in [
                            "lobby",
                            "stage_setup",
                            "walk",
                            "vote",
                            "place",
                            "upgrade",
                            "dungeon",
                            "end",
                        ] {
                            changed |= ui.selectable_value(step, s.to_string(), s).changed();
                        }
                    });
            }
        }
    });
    changed |= ui
        .checkbox(
            &mut r.all_clients,
            "look and act on every client (else the main one)",
        )
        .changed();
    changed
}

fn split(t: &str) -> Vec<String> {
    t.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}
