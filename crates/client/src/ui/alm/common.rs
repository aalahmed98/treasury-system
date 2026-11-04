//! Pieces every ALM tool shares: the assumptions header with the what-if state, headline blocks,
//! two-column figures, simple tables and the scenario picker.

use domain::RateScenario;
use egui::{Align, Layout, RichText};
use egui_extras::{Column, TableBuilder};

use crate::alm_state::{AlmState, Standing};
use crate::ui::theme::{Palette, pal};

pub fn standing_color(p: &Palette, s: Standing) -> egui::Color32 {
    match s {
        Standing::Comfortable => p.positive,
        Standing::Watch => p.warning,
        Standing::Breach => p.negative,
        Standing::Unknown => p.muted,
    }
}

/// "Assumptions" with the what-if badge and the reset button; returns true when reset.
pub fn assumptions_header(alm: &mut AlmState, ui: &mut egui::Ui, p: &Palette) {
    ui.horizontal(|ui| {
        ui.strong("Assumptions");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if alm.overrides.is_some() {
                if ui.small_button("Reset to bank parameters").clicked() {
                    alm.reset_overrides();
                }
                ui.label(RichText::new("what-if").color(p.warning).small());
            } else {
                ui.label(RichText::new(format!("bank v{}", alm.bank_version)).color(p.muted).small());
            }
        });
    });
    if alm.bank_changed {
        ui.colored_label(p.warning, "the bank's parameters changed while you were editing; reset to pick them up");
    }
}

pub fn pct(ui: &mut egui::Ui, v: &mut f64, max: f64) -> bool {
    ui.add(egui::DragValue::new(v).range(0.0..=max).speed(0.5).suffix("%").fixed_decimals(1)).changed()
}

pub fn bp(ui: &mut egui::Ui, v: &mut f64) -> bool {
    ui.add(egui::DragValue::new(v).range(-2000.0..=2000.0).speed(5.0).suffix(" bp").fixed_decimals(0)).changed()
}

/// A big number with its standing colour and two lines of context.
pub fn headline(ui: &mut egui::Ui, p: &Palette, value: &str, standing: Standing, line1: &str, line2: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(value).size(34.0).strong().color(standing_color(p, standing)));
        ui.vertical(|ui| {
            ui.label(RichText::new(line1).color(p.muted).small());
            ui.label(RichText::new(line2).color(p.muted).small());
        });
    });
}

pub fn two_cols(ui: &mut egui::Ui, p: &Palette, rows: &[(String, String)], bold_last: bool) {
    egui::Grid::new(ui.next_auto_id()).num_columns(2).spacing([16.0, 3.0]).show(ui, |ui| {
        for (i, (k, v)) in rows.iter().enumerate() {
            let last = bold_last && i + 1 == rows.len();
            ui.label(if last { RichText::new(k).strong() } else { RichText::new(k).color(p.muted) });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.monospace(if last { RichText::new(v).strong() } else { RichText::new(v) });
            });
            ui.end_row();
        }
    });
}

/// A striped table: the first column is text, the rest right-aligned monospace.
pub fn table<const N: usize>(ui: &mut egui::Ui, id: &str, headers: &[&str; N], rows: &[[String; N]], height: Option<f32>) {
    if rows.is_empty() {
        ui.label(RichText::new("none").color(pal(ui).muted));
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
        t.header(20.0, |mut h| {
            for s in headers {
                h.col(|ui| {
                    ui.strong(*s);
                });
            }
        })
        .body(|body| {
            body.rows(18.0, rows.len(), |mut row| {
                let r = &rows[row.index()];
                row.col(|ui| {
                    ui.label(&r[0]);
                });
                for cell in &r[1..] {
                    row.col(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.monospace(cell);
                        });
                    });
                }
            });
        });
    });
}

/// The rate scenario chips; returns true when the choice changed.
pub fn scenario_picker(ui: &mut egui::Ui, scenario: &mut RateScenario, custom_bp: &mut f64, include_base: bool) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        let mut all: Vec<RateScenario> = Vec::new();
        if include_base {
            all.push(RateScenario::Base);
        }
        all.extend(RateScenario::BASEL);
        all.push(RateScenario::Custom);
        for s in all {
            if ui.selectable_label(*scenario == s, s.label()).clicked() && *scenario != s {
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
        ui.strong(title);
        if loading {
            ui.spinner();
        }
        if let Some(e) = error {
            ui.colored_label(p.negative, e);
        }
    });
}

/// The results pane of a computed tool: the slot's view, or a placeholder.
pub fn with_view<'a, V>(ui: &mut egui::Ui, p: &Palette, slot: &'a crate::alm_state::ToolSlot<V>, pending: bool, title: &str) -> Option<&'a V> {
    results_header(ui, p, title, slot.result.loading || pending, slot.result.error.as_ref());
    if slot.result.view.is_none() {
        ui.label(RichText::new("computing…").color(p.muted));
    }
    slot.result.view.as_ref()
}
