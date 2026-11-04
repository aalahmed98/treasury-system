//! Pieces every ALM tool shares: the panel pair, the assumptions header with the what-if state,
//! the headline with its standing, two-column figures, tables and the scenario chips.

use domain::RateScenario;
use egui::{Align, CentralPanel, Layout, Panel, RichText, ScrollArea};
use egui_extras::{Column, TableBuilder};

use crate::alm_state::{AlmState, Standing};
use crate::ui::theme::{Palette, medium, pal, semibold};
use crate::ui::widgets::{Tone, badge, canvas_frame, chip, secondary, side_frame, status_badge, th};

pub fn standing_color(p: &Palette, s: Standing) -> egui::Color32 {
    match s {
        Standing::Comfortable => p.good,
        Standing::Watch => p.warn,
        Standing::Breach => p.bad,
        Standing::Unknown => p.muted,
    }
}

pub fn standing_tone(s: Standing) -> Tone {
    match s {
        Standing::Comfortable => Tone::Good,
        Standing::Watch => Tone::Warn,
        Standing::Breach => Tone::Bad,
        Standing::Unknown => Tone::Neutral,
    }
}

pub fn standing_label(s: Standing) -> &'static str {
    match s {
        Standing::Comfortable => "within limits",
        Standing::Watch => "watch",
        Standing::Breach => "breach",
        Standing::Unknown => "no data",
    }
}

/// The assumptions panel on the left of a tool.
pub fn side_panel(ui: &mut egui::Ui, id: &str, default: f32, min: f32, body: impl FnOnce(&mut egui::Ui)) {
    let p = pal(ui);
    Panel::left(id).resizable(true).default_size(default).min_size(min).frame(side_frame(&p)).show(ui, |ui| {
        ScrollArea::vertical().id_salt(format!("{id}_scroll")).show(ui, body);
    });
}

/// The results on the right of a tool.
pub fn results_panel(ui: &mut egui::Ui, id: &str, body: impl FnOnce(&mut egui::Ui)) {
    let p = pal(ui);
    CentralPanel::default().frame(canvas_frame(&p)).show(ui, |ui| {
        ScrollArea::vertical().id_salt(id).show(ui, body);
    });
}

/// "Assumptions" with the what-if badge and the reset button.
pub fn assumptions_header(alm: &mut AlmState, ui: &mut egui::Ui, p: &Palette) {
    ui.horizontal(|ui| {
        ui.label(p.heading("Assumptions"));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if alm.overrides.is_some() {
                if secondary(ui, p, true, "Reset").on_hover_text("back to the bank's saved parameters").clicked() {
                    alm.reset_overrides();
                }
                badge(ui, p, "what-if", Tone::Warn).on_hover_text("unsaved edits; Setup saves a version");
            } else {
                badge(ui, p, &format!("bank v{}", alm.bank_version), Tone::Neutral).on_hover_text("the bank's active parameter set");
            }
        });
    });
    if alm.bank_changed {
        ui.label(RichText::new("the bank's parameters changed while you were editing; reset to pick them up").small().color(p.warn));
    }
    ui.add_space(2.0);
}

/// A group label inside the assumptions.
pub fn group(ui: &mut egui::Ui, p: &Palette, title: &str) {
    ui.add_space(8.0);
    ui.label(p.eyebrow(title));
    ui.add_space(2.0);
}

pub fn pct(ui: &mut egui::Ui, v: &mut f64, max: f64) -> bool {
    ui.add(egui::DragValue::new(v).range(0.0..=max).speed(0.5).suffix("%").fixed_decimals(1)).changed()
}

pub fn bp(ui: &mut egui::Ui, v: &mut f64) -> bool {
    ui.add(egui::DragValue::new(v).range(-2000.0..=2000.0).speed(5.0).suffix(" bp").fixed_decimals(0)).changed()
}

/// The one number a tool is about, its standing beside it, and two lines of context.
pub fn headline(ui: &mut egui::Ui, p: &Palette, value: &str, standing: Standing, line1: &str, line2: &str) {
    ui.horizontal(|ui| {
        ui.label(p.hero(value));
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.add_space(12.0);
            status_badge(ui, p, standing_label(standing), standing_tone(standing));
        });
    });
    ui.label(p.small(line1));
    ui.label(p.small(line2));
}

pub fn two_cols(ui: &mut egui::Ui, p: &Palette, rows: &[(String, String)], bold_last: bool) {
    egui::Grid::new(ui.next_auto_id()).num_columns(2).spacing([16.0, 5.0]).show(ui, |ui| {
        for (i, (k, v)) in rows.iter().enumerate() {
            let last = bold_last && i + 1 == rows.len();
            ui.label(if last { RichText::new(k).family(medium()).color(p.text) } else { p.secondary(k) });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(if last { RichText::new(v).family(semibold()).color(p.text) } else { RichText::new(v).color(p.text) });
            });
            ui.end_row();
        }
    });
}

/// A striped table: the first column is text, the rest right-aligned figures.
pub fn table<const N: usize>(ui: &mut egui::Ui, id: &str, headers: &[&str; N], rows: &[[String; N]], height: Option<f32>) {
    if rows.is_empty() {
        ui.label(pal(ui).muted("none"));
        return;
    }
    ui.push_id(id, |ui| {
        if let Some(h) = height {
            ui.set_height(h);
        }
        let mut t = TableBuilder::new(ui).id_salt(id).striped(true).vscroll(height.is_some()).cell_layout(Layout::left_to_right(Align::Center)).column(Column::remainder().at_least(110.0));
        for _ in 1..N {
            t = t.column(Column::auto().at_least(70.0));
        }
        t.header(24.0, |mut h| {
            for (i, s) in headers.iter().enumerate() {
                h.col(|ui| {
                    if i == 0 {
                        th(ui, s);
                    } else {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            th(ui, s);
                        });
                    }
                });
            }
        })
        .body(|body| {
            body.rows(22.0, rows.len(), |mut row| {
                let r = &rows[row.index()];
                row.col(|ui| {
                    ui.label(&r[0]);
                });
                for cell in &r[1..] {
                    row.col(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(cell);
                        });
                    });
                }
            });
        });
    });
}

/// The rate scenario chips; returns true when the choice changed.
pub fn scenario_picker(ui: &mut egui::Ui, scenario: &mut RateScenario, custom_bp: &mut f64, include_base: bool) -> bool {
    let p = pal(ui);
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        let mut all: Vec<RateScenario> = Vec::new();
        if include_base {
            all.push(RateScenario::Base);
        }
        all.extend(RateScenario::BASEL);
        all.push(RateScenario::Custom);
        for s in all {
            if chip(ui, &p, *scenario == s, s.label()).clicked() && *scenario != s {
                *scenario = s;
                changed = true;
            }
        }
        if *scenario == RateScenario::Custom {
            changed |= bp(ui, custom_bp);
        }
    });
    changed
}

pub fn results_header(ui: &mut egui::Ui, p: &Palette, title: &str, loading: bool, error: Option<&String>) {
    ui.horizontal(|ui| {
        ui.label(p.title(title));
        if loading {
            ui.add(egui::Spinner::new().size(14.0));
        }
        if let Some(e) = error {
            ui.label(RichText::new(e).color(p.bad));
        }
    });
    ui.add_space(6.0);
}

/// The results pane of a computed tool: the slot's view, or a placeholder.
pub fn with_view<'a, V>(ui: &mut egui::Ui, p: &Palette, slot: &'a crate::alm_state::ToolSlot<V>, pending: bool, title: &str) -> Option<&'a V> {
    results_header(ui, p, title, slot.result.loading || pending, slot.result.error.as_ref());
    if slot.result.view.is_none() {
        ui.label(p.muted("computing…"));
    }
    slot.result.view.as_ref()
}
