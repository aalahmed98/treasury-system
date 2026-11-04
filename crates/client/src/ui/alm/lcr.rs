//! Liquidity coverage ratio: assumptions on the left (run-off rates, haircuts, caps and the bank's
//! thresholds, editable as an unsaved what-if), the ratio against its thresholds and the full
//! composition on the right.

use egui::RichText;

use crate::alm_state::AlmState;
use crate::app::App;
use crate::ui::alm::common::{assumptions_header, group, headline, pct, results_panel, side_panel, table, two_cols, with_view};
use crate::ui::charts;
use crate::ui::theme::{Palette, pal};
use crate::ui::widgets::{card, wrap_row};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    side_panel(ui, "alm_lcr_assumptions", 340.0, 260.0, |ui| assumptions(&mut app.alm, ui, &p));
    results_panel(ui, "alm_lcr_results", |ui| results(&app.alm, ui, &p));
}

fn assumptions(alm: &mut AlmState, ui: &mut egui::Ui, p: &Palette) {
    assumptions_header(alm, ui, p);
    let Some(mut params) = alm.working() else { return };
    let mut changed = false;
    let lcr = &mut params.lcr;

    ui.add_space(4.0);
    group(ui, p, "Thresholds");
    egui::Grid::new("lcr_thresholds").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
        for (label, v) in [("Regulatory minimum", &mut lcr.minimum), ("Internal minimum", &mut lcr.internal_minimum), ("Early warning", &mut lcr.early_warning)] {
            ui.label(label);
            changed |= pct(ui, v, 500.0);
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    group(ui, p, "Liquid assets");
    egui::Grid::new("lcr_hqla").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
        for level in ["L1", "L2A", "L2B"] {
            ui.label(format!("{level} haircut"));
            changed |= pct(ui, lcr.haircuts.entry(level.to_string()).or_insert(0.0), 100.0);
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
    group(ui, p, "Run-off rates");
    egui::Grid::new("lcr_outflows").num_columns(2).spacing([12.0, 3.0]).striped(true).show(ui, |ui| {
        for (class, v) in lcr.outflow_rates.iter_mut() {
            ui.label(class.replace('_', " ").to_lowercase());
            changed |= pct(ui, v, 100.0);
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    group(ui, p, "Inflow rates");
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
        alm.touch();
    }
}

fn results(alm: &AlmState, ui: &mut egui::Ui, p: &Palette) {
    let Some(v) = with_view(ui, p, &alm.lcr, alm.is_pending(), "Liquidity coverage ratio") else { return };
    headline(
        ui,
        p,
        &v.headline,
        v.standing,
        &format!("{} · {} · in {}", v.raw.as_of.format("%d %b %Y"), v.basis, v.raw.reporting_ccy),
        &format!("minimum {:.0}% · internal {:.0}% · early warning {:.0}%", v.raw.minimum, v.raw.internal_minimum, v.raw.early_warning),
    );
    let value = v.raw.total.ratio.unwrap_or(0.0);
    charts::limit_gauge(ui, value, v.raw.internal_minimum.max(v.raw.minimum), Some(v.raw.early_warning), value.max(v.raw.early_warning) * 1.15, true, p, 320.0);
    ui.add_space(8.0);
    wrap_row(ui, |ui| {
        card(ui, p, "Stock of liquid assets", 320.0, |ui| two_cols(ui, p, &v.hqla_rows, true));
        card(ui, p, "Net cash outflows (30 days)", 320.0, |ui| two_cols(ui, p, &v.totals, true));
        card(ui, p, "By currency", 300.0, |ui| table(ui, "lcr_ccy", &["Currency", "Stock", "Net outflows", "LCR"], &v.ccy_rows, None));
    });
    ui.add_space(8.0);
    wrap_row(ui, |ui| {
        card(ui, p, "Outflows", 480.0, |ui| table(ui, "lcr_out", &["Class", "Amount", "Run-off", "Outflow"], &v.outflow_rows, None));
        card(ui, p, "Inflows", 480.0, |ui| table(ui, "lcr_in", &["Class", "Due in 30 days", "Rate", "Inflow"], &v.inflow_rows, None));
    });
}
