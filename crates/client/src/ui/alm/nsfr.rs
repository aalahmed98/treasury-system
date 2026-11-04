//! Net stable funding ratio: factors per class and band on the left, the ratio and its lines on
//! the right.

use egui::{Panel, RichText};

use crate::alm_state::AlmState;
use crate::app::App;
use crate::ui::alm::common::{assumptions_header, headline, pct, table, two_cols, with_view};
use crate::ui::charts;
use crate::ui::theme::{Palette, pal};
use crate::ui::widgets::{card, wrap_row};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    Panel::left("alm_nsfr_assumptions").resizable(true).default_size(360.0).min_size(280.0).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("alm_nsfr_assumptions_scroll").show(ui, |ui| assumptions(&mut app.alm, ui, &p));
    });
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("alm_nsfr_results").show(ui, |ui| results(&app.alm, ui, &p));
    });
}

fn factor_grid(ui: &mut egui::Ui, id: &str, map: &mut std::collections::BTreeMap<String, [f64; 3]>) -> bool {
    let mut changed = false;
    egui::Grid::new(id).num_columns(4).spacing([10.0, 3.0]).striped(true).show(ui, |ui| {
        ui.label("");
        for h in ["< 6M", "6M-1Y", "≥ 1Y"] {
            ui.label(RichText::new(h).small());
        }
        ui.end_row();
        for (class, f) in map.iter_mut() {
            ui.label(class.replace('_', " ").to_lowercase());
            for v in f.iter_mut() {
                changed |= pct(ui, v, 100.0);
            }
            ui.end_row();
        }
    });
    changed
}

fn assumptions(alm: &mut AlmState, ui: &mut egui::Ui, p: &Palette) {
    assumptions_header(alm, ui, p);
    let Some(mut params) = alm.working() else { return };
    let mut changed = false;
    let n = &mut params.nsfr;
    ui.add_space(4.0);
    egui::Grid::new("nsfr_thresholds").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
        ui.label("Regulatory minimum");
        changed |= pct(ui, &mut n.minimum, 500.0);
        ui.end_row();
        ui.label("Internal minimum");
        changed |= pct(ui, &mut n.internal_minimum, 500.0);
        ui.end_row();
    });
    ui.add_space(6.0);
    ui.label(RichText::new("Available stable funding factors").color(p.muted).small());
    changed |= factor_grid(ui, "nsfr_asf", &mut n.asf);
    ui.add_space(6.0);
    ui.label(RichText::new("Required stable funding factors").color(p.muted).small());
    changed |= factor_grid(ui, "nsfr_rsf", &mut n.rsf);
    egui::Grid::new("nsfr_other").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
        for level in ["L1", "L2A", "L2B"] {
            ui.label(format!("HQLA {level}"));
            changed |= pct(ui, n.hqla_rsf.entry(level.to_string()).or_insert(0.0), 100.0);
            ui.end_row();
        }
        ui.label("Off-balance commitments");
        changed |= pct(ui, &mut n.off_balance_rsf_pct, 100.0);
        ui.end_row();
        ui.label("Encumbered a year or more");
        changed |= pct(ui, &mut n.encumbered_rsf_pct, 100.0);
        ui.end_row();
    });
    if changed {
        alm.overrides = Some(params);
        alm.touch();
    }
}

fn results(alm: &AlmState, ui: &mut egui::Ui, p: &Palette) {
    let Some(v) = with_view(ui, p, &alm.nsfr, alm.is_pending(), "Net stable funding ratio") else { return };
    headline(
        ui,
        p,
        &v.headline,
        v.standing,
        &format!("{} · {} · in {}", v.raw.as_of.format("%d %b %Y"), v.basis, v.raw.reporting_ccy),
        &format!("minimum {:.0}% · internal {:.0}%", v.raw.minimum, v.raw.internal_minimum),
    );
    let value = v.raw.ratio.unwrap_or(0.0);
    charts::limit_gauge(ui, value, v.raw.internal_minimum.max(v.raw.minimum), None, value.max(v.raw.internal_minimum) * 1.15, true, p, 320.0);
    ui.add_space(8.0);
    card(ui, p, "Totals", 320.0, |ui| two_cols(ui, p, &v.totals, false));
    ui.add_space(8.0);
    wrap_row(ui, |ui| {
        card(ui, p, "Available stable funding", 480.0, |ui| table(ui, "nsfr_asf_t", &["Class · band", "Amount", "Factor", "Weighted"], &v.asf_rows, None));
        card(ui, p, "Required stable funding", 480.0, |ui| table(ui, "nsfr_rsf_t", &["Class · band", "Amount", "Factor", "Weighted"], &v.rsf_rows, None));
    });
}
