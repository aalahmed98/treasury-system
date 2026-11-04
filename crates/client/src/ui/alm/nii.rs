//! Net interest income and margin projection under a rate scenario.

use egui::RichText;

use crate::alm_state::{AlmState, Standing};
use crate::app::App;
use crate::ui::alm::common::{assumptions_header, group, headline, pct, results_panel, scenario_picker, side_panel, table, with_view};
use crate::ui::charts;
use crate::ui::fmt::fmt_money_short;
use crate::ui::theme::{Palette, pal};
use crate::ui::widgets::{card, wrap_row};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    side_panel(ui, "alm_nii_assumptions", 320.0, 260.0, |ui| assumptions(&mut app.alm, ui, &p));
    results_panel(ui, "alm_nii_results", |ui| results(&app.alm, ui, &p));
}

fn assumptions(alm: &mut AlmState, ui: &mut egui::Ui, p: &Palette) {
    assumptions_header(alm, ui, p);
    ui.add_space(4.0);
    group(ui, p, "Rate scenario");
    let (mut scenario, mut bp) = (alm.scenario, alm.custom_bp);
    if scenario_picker(ui, &mut scenario, &mut bp, true) {
        alm.scenario = scenario;
        alm.custom_bp = bp;
        alm.touch();
    }
    let Some(mut params) = alm.working() else { return };
    let mut changed = false;
    ui.add_space(6.0);
    group(ui, p, "Projection");
    egui::Grid::new("nii_params").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
        ui.label("Horizon (months)");
        changed |= ui.add(egui::DragValue::new(&mut params.nii.horizon_months).range(1..=60).speed(1.0)).changed();
        ui.end_row();
        ui.label("Constant balance sheet");
        changed |= ui.checkbox(&mut params.nii.constant_balance_sheet, "").changed();
        ui.end_row();
        ui.label("Default deposit beta");
        let mut beta = params.nii.default_beta * 100.0;
        if pct(ui, &mut beta, 100.0) {
            params.nii.default_beta = beta / 100.0;
            changed = true;
        }
        ui.end_row();
    });
    ui.add_space(6.0);
    group(ui, p, "Rate shocks (bp)");
    egui::Grid::new("nii_shocks").num_columns(4).spacing([8.0, 3.0]).striped(true).show(ui, |ui| {
        ui.label("");
        for h in ["Parallel", "Short", "Long"] {
            ui.label(p.label(h));
        }
        ui.end_row();
        for (ccy, s) in params.stress.shocks.iter_mut() {
            ui.label(ccy.as_str());
            for v in [&mut s.parallel, &mut s.short, &mut s.long] {
                changed |= ui.add(egui::DragValue::new(v).range(0.0..=2000.0).speed(5.0).fixed_decimals(0)).changed();
            }
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    ui.label(RichText::new("Fixed cohorts keep their rate until maturity and roll at the scenario curve with their margin; floating cohorts reprice at each reset; administered deposits move by their beta.").color(p.muted).small());
    if changed {
        alm.overrides = Some(params);
        alm.touch();
    }
}

fn results(alm: &AlmState, ui: &mut egui::Ui, p: &Palette) {
    let Some(v) = with_view(ui, p, &alm.nii, alm.is_pending(), "Net interest income") else { return };
    headline(
        ui,
        p,
        &v.headline,
        Standing::Comfortable,
        &format!("over {} months from {} · {} · in {}", v.raw.horizon_months, v.raw.as_of.format("%d %b %Y"), v.basis, v.raw.reporting_ccy),
        &format!("NIM {} on earning assets of {}{}", v.nim, fmt_money_short(v.raw.earning_assets), v.delta.as_ref().map(|d| format!(" · {d}")).unwrap_or_default()),
    );
    ui.add_space(8.0);
    card(ui, p, "NII by month", 720.0, |ui| charts::bars(ui, &v.month_bars, p.series[0], 170.0, fmt_money_short));
    ui.add_space(8.0);
    wrap_row(ui, |ui| {
        card(ui, p, "Months", 440.0, |ui| table(ui, "nii_rows", &["Month", "Income", "Expense", "NII"], &v.rows, None));
        card(ui, p, "By book", 240.0, |ui| table(ui, "nii_book", &["Book", "NII"], &v.by_book, None));
        card(ui, p, "By currency", 220.0, |ui| table(ui, "nii_ccy", &["Currency", "NII"], &v.by_ccy, None));
    });
}
