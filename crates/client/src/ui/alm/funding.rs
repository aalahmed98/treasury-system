//! Funding concentration: the largest depositor groups and the wholesale share against limits.

use egui::{Panel, RichText};

use crate::alm_state::{AlmState, Standing};
use crate::app::App;
use crate::ui::alm::common::{assumptions_header, headline, pct, table, two_cols, with_view};
use crate::ui::charts;
use crate::ui::theme::{Palette, pal};
use crate::ui::widgets::{card, wrap_row};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    Panel::left("alm_funding_assumptions").resizable(true).default_size(300.0).min_size(240.0).show(ui, |ui| assumptions(&mut app.alm, ui, &p));
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("alm_funding_results").show(ui, |ui| results(&app.alm, ui, &p));
    });
}

fn assumptions(alm: &mut AlmState, ui: &mut egui::Ui, p: &Palette) {
    assumptions_header(alm, ui, p);
    let Some(mut params) = alm.working() else { return };
    let mut changed = false;
    ui.add_space(4.0);
    ui.label(RichText::new("Limits").color(p.muted).small());
    egui::Grid::new("funding_limits").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
        ui.label("Largest depositor (% of deposits)");
        changed |= pct(ui, &mut params.funding.top1_limit_pct, 100.0);
        ui.end_row();
        ui.label("Ten largest depositors");
        changed |= pct(ui, &mut params.funding.top10_limit_pct, 100.0);
        ui.end_row();
        ui.label("Wholesale funding (% of funding)");
        changed |= pct(ui, &mut params.funding.wholesale_share_limit_pct, 100.0);
        ui.end_row();
    });
    if changed {
        alm.overrides = Some(params);
        alm.touch();
    }
}

fn results(alm: &AlmState, ui: &mut egui::Ui, p: &Palette) {
    let Some(v) = with_view(ui, p, &alm.funding, alm.is_pending(), "Funding concentration") else { return };
    headline(
        ui,
        p,
        &format!("{:.1}%", v.raw.top10_pct),
        if v.raw.breaches.is_empty() { Standing::Comfortable } else { Standing::Breach },
        &format!("ten largest depositor groups as a share of deposits (limit {:.0}%)", v.raw.top10_limit_pct),
        &format!("{} · {} · in {}", v.raw.as_of.format("%d %b %Y"), v.basis, v.raw.reporting_ccy),
    );
    if !v.raw.breaches.is_empty() {
        ui.colored_label(p.negative, v.raw.breaches.join("; "));
    }
    ui.add_space(8.0);
    wrap_row(ui, |ui| {
        card(ui, p, "Measures", 340.0, |ui| two_cols(ui, p, &v.headline_rows, false));
        card(ui, p, "Largest groups (% of deposits)", 560.0, |ui| charts::bars(ui, &v.group_bars, p.series[3], 170.0, |x| format!("{x:.1}%")));
    });
    ui.add_space(8.0);
    wrap_row(ui, |ui| {
        card(ui, p, "Depositor groups", 560.0, |ui| table(ui, "funding_groups", &["Group", "Segment", "Balance", "Share"], &v.group_rows, None));
        card(ui, p, "Deposits by segment", 300.0, |ui| table(ui, "funding_segments", &["Segment", "Deposits"], &v.segment_rows, None));
    });
}
