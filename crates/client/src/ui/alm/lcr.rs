//! Liquidity coverage ratio: assumptions on the left (run-off rates, haircuts, caps and the bank's
//! thresholds, editable as an unsaved what-if), the ratio against its thresholds and the full
//! composition on the right.

use egui::{Align, DragValue, Layout, Panel, RichText};
use egui_extras::{Column, TableBuilder};

use crate::alm_state::{AlmState, Standing};
use crate::app::App;
use crate::ui::charts;
use crate::ui::theme::{Palette, pal};
use crate::ui::widgets::{card, wrap_row};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    Panel::left("alm_lcr_assumptions").resizable(true).default_size(340.0).min_size(260.0).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("alm_lcr_assumptions_scroll").show(ui, |ui| {
            assumptions(&mut app.alm, ui, &p);
        });
    });
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("alm_lcr_results").show(ui, |ui| results(&app.alm, ui, &p));
    });
}

fn pct(ui: &mut egui::Ui, v: &mut f64, max: f64) -> bool {
    ui.add(DragValue::new(v).range(0.0..=max).speed(0.5).suffix("%").fixed_decimals(1)).changed()
}

fn assumptions(alm: &mut AlmState, ui: &mut egui::Ui, p: &Palette) {
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
    // Edit a copy; it becomes the what-if only when a value really changed.
    let Some(mut params) = alm.working() else { return };
    let mut changed = false;
    let lcr = &mut params.lcr;

    ui.add_space(4.0);
    ui.label(RichText::new("Thresholds").color(p.muted).small());
    egui::Grid::new("lcr_thresholds").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
        ui.label("Regulatory minimum");
        changed |= pct(ui, &mut lcr.minimum, 500.0);
        ui.end_row();
        ui.label("Internal minimum");
        changed |= pct(ui, &mut lcr.internal_minimum, 500.0);
        ui.end_row();
        ui.label("Early warning");
        changed |= pct(ui, &mut lcr.early_warning, 500.0);
        ui.end_row();
    });

    ui.add_space(6.0);
    ui.label(RichText::new("Liquid assets").color(p.muted).small());
    egui::Grid::new("lcr_hqla").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
        for level in ["L1", "L2A", "L2B"] {
            ui.label(format!("{level} haircut"));
            let v = lcr.haircuts.entry(level.to_string()).or_insert(0.0);
            changed |= pct(ui, v, 100.0);
            ui.end_row();
        }
        ui.label("Level 2 cap");
        changed |= pct(ui, &mut lcr.cap_l2_total_pct, 99.0);
        ui.end_row();
        ui.label("Level 2B cap");
        changed |= pct(ui, &mut lcr.cap_l2b_pct, 99.0);
        ui.end_row();
        ui.label("Inflow cap");
        changed |= pct(ui, &mut lcr.inflow_cap_pct, 100.0);
        ui.end_row();
    });

    ui.add_space(6.0);
    ui.label(RichText::new("Run-off rates").color(p.muted).small());
    egui::Grid::new("lcr_outflows").num_columns(2).spacing([12.0, 3.0]).striped(true).show(ui, |ui| {
        for (class, v) in lcr.outflow_rates.iter_mut() {
            ui.label(class.replace('_', " ").to_lowercase());
            changed |= pct(ui, v, 100.0);
            ui.end_row();
        }
    });

    ui.add_space(6.0);
    ui.label(RichText::new("Inflow rates").color(p.muted).small());
    egui::Grid::new("lcr_inflows").num_columns(2).spacing([12.0, 3.0]).striped(true).show(ui, |ui| {
        for (class, v) in lcr.inflow_rates.iter_mut() {
            ui.label(class.replace('_', " ").to_lowercase());
            changed |= pct(ui, v, 100.0);
            ui.end_row();
        }
    });
    ui.add_space(8.0);
    ui.label(RichText::new("Basel III defaults until the CBB LM-11 calibration and the bank's limits are confirmed. Edits here are a what-if; Setup saves a new version.").color(p.muted).small());

    if changed {
        alm.overrides = Some(params);
        alm.mark_dirty();
    }
}

fn results(alm: &AlmState, ui: &mut egui::Ui, p: &Palette) {
    ui.horizontal(|ui| {
        ui.strong("Liquidity coverage ratio");
        if alm.lcr.loading || alm.is_dirty() {
            ui.spinner();
        }
        if let Some(e) = &alm.lcr.error {
            ui.colored_label(p.negative, e);
        }
    });
    let Some(v) = &alm.lcr.view else {
        ui.label(RichText::new("computing…").color(p.muted));
        return;
    };
    let tone = match v.standing {
        Standing::Comfortable => p.positive,
        Standing::Watch => p.warning,
        Standing::Breach => p.negative,
        Standing::Unknown => p.muted,
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(&v.headline).size(34.0).strong().color(tone));
        ui.vertical(|ui| {
            ui.label(RichText::new(format!("{} · {} · in {}", v.raw.as_of.format("%d %b %Y"), v.basis, v.raw.reporting_ccy)).color(p.muted).small());
            ui.label(
                RichText::new(format!("minimum {:.0}% · internal {:.0}% · early warning {:.0}%", v.raw.minimum, v.raw.internal_minimum, v.raw.early_warning))
                    .color(p.muted)
                    .small(),
            );
            let value = v.raw.total.ratio.unwrap_or(0.0);
            let max = value.max(v.raw.early_warning) * 1.15;
            charts::limit_gauge(ui, value, v.raw.internal_minimum.max(v.raw.minimum), Some(v.raw.early_warning), max, p, 320.0);
        });
    });
    ui.add_space(8.0);

    wrap_row(ui, |ui| {
        card(ui, p, "Stock of liquid assets", 320.0, |ui| two_cols(ui, p, &v.hqla_rows));
        card(ui, p, "Net cash outflows (30 days)", 320.0, |ui| two_cols(ui, p, &v.totals));
        card(ui, p, "By currency", 300.0, |ui| table(ui, "lcr_ccy", &["Currency", "Stock", "Net outflows", "LCR"], &v.ccy_rows));
    });
    ui.add_space(8.0);
    wrap_row(ui, |ui| {
        card(ui, p, "Outflows", 480.0, |ui| table(ui, "lcr_out", &["Class", "Amount", "Run-off", "Outflow"], &v.outflow_rows));
        card(ui, p, "Inflows", 480.0, |ui| table(ui, "lcr_in", &["Class", "Due in 30 days", "Rate", "Inflow"], &v.inflow_rows));
    });
}

fn two_cols(ui: &mut egui::Ui, p: &Palette, rows: &[(String, String)]) {
    egui::Grid::new(ui.next_auto_id()).num_columns(2).spacing([16.0, 3.0]).show(ui, |ui| {
        for (i, (k, v)) in rows.iter().enumerate() {
            let last = i + 1 == rows.len();
            ui.label(if last { RichText::new(k).strong() } else { RichText::new(k).color(p.muted) });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.monospace(if last { RichText::new(v).strong() } else { RichText::new(v) });
            });
            ui.end_row();
        }
    });
}

fn table(ui: &mut egui::Ui, id: &str, headers: &[&str], rows: &[[String; 4]]) {
    if rows.is_empty() {
        ui.label(RichText::new("none").color(pal(ui).muted));
        return;
    }
    ui.push_id(id, |ui| {
        let mut t = TableBuilder::new(ui).id_salt(id).striped(true).cell_layout(Layout::left_to_right(Align::Center)).column(Column::remainder().at_least(120.0));
        for _ in 1..headers.len() {
            t = t.column(Column::auto().at_least(80.0));
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
