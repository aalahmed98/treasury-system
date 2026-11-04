//! Stress testing: the Basel rate scenarios through NII and EVE, the liquidity stress with its
//! survival horizon, and FX shocks.


use crate::alm_state::{AlmState, Standing};
use crate::app::App;
use crate::ui::alm::common::{assumptions_header, bp, group, headline, pct, results_panel, side_panel, table, two_cols, with_view};
use crate::ui::charts;
use crate::ui::fmt::fmt_money_short;
use crate::ui::theme::{Palette, pal};
use crate::ui::widgets::{card, wrap_row};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    side_panel(ui, "alm_stress_assumptions", 330.0, 260.0, |ui| assumptions(&mut app.alm, ui, &p));
    results_panel(ui, "alm_stress_results", |ui| results(&app.alm, ui, &p));
}

fn assumptions(alm: &mut AlmState, ui: &mut egui::Ui, p: &Palette) {
    assumptions_header(alm, ui, p);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label("Custom parallel shock");
        let mut v = alm.custom_bp;
        if bp(ui, &mut v) {
            alm.custom_bp = v;
            alm.touch();
        }
    });
    let Some(mut params) = alm.working() else { return };
    let mut changed = false;
    ui.add_space(6.0);
    group(ui, p, "Limits");
    egui::Grid::new("stress_limits").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
        ui.label("EVE loss (% of Tier 1)");
        changed |= pct(ui, &mut params.gap.eve_limit_pct_tier1, 100.0);
        ui.end_row();
        ui.label("NII sensitivity (%)");
        changed |= pct(ui, &mut params.gap.nii_sensitivity_limit_pct, 100.0);
        ui.end_row();
        ui.label("Tier 1 capital");
        changed |= ui.add(egui::DragValue::new(&mut params.tier1_capital).range(1.0..=1e12).speed(1e6).fixed_decimals(0)).changed();
        ui.end_row();
    });
    ui.add_space(6.0);
    group(ui, p, "Rate shocks (bp)");
    egui::Grid::new("stress_shocks").num_columns(4).spacing([8.0, 3.0]).striped(true).show(ui, |ui| {
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
    egui::Grid::new("stress_floor").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
        ui.label("Post-shock floor (bp)");
        changed |= ui.add(egui::DragValue::new(&mut params.stress.floor_bp).range(-500.0..=0.0).speed(5.0).fixed_decimals(0)).changed();
        ui.end_row();
        ui.label("Floor rises per year (bp)");
        changed |= ui.add(egui::DragValue::new(&mut params.stress.floor_slope_bp_per_year).range(0.0..=50.0).speed(1.0).fixed_decimals(0)).changed();
        ui.end_row();
    });
    ui.add_space(6.0);
    group(ui, p, "Liquidity stress");
    egui::Grid::new("stress_liq").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
        let l = &mut params.stress.liquidity;
        ui.label("Run-off multiplier");
        changed |= ui.add(egui::DragValue::new(&mut l.outflow_multiplier).range(1.0..=5.0).speed(0.05).fixed_decimals(2)).changed();
        ui.end_row();
        ui.label("Extra HQLA haircut");
        changed |= pct(ui, &mut l.extra_hqla_haircut_pct, 100.0);
        ui.end_row();
        ui.label("Inflow multiplier");
        changed |= ui.add(egui::DragValue::new(&mut l.inflow_multiplier).range(0.0..=1.0).speed(0.05).fixed_decimals(2)).changed();
        ui.end_row();
    });
    ui.add_space(6.0);
    group(ui, p, "FX shocks (% fall against BHD)");
    egui::Grid::new("stress_fx").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
        for (ccy, v) in params.stress.fx_shock_pct.iter_mut() {
            ui.label(ccy.as_str());
            changed |= pct(ui, v, 100.0);
            ui.end_row();
        }
    });
    if changed {
        alm.overrides = Some(params);
        alm.touch();
    }
}

fn results(alm: &AlmState, ui: &mut egui::Ui, p: &Palette) {
    let Some(v) = with_view(ui, p, &alm.stress, alm.is_pending(), "Stress testing") else { return };
    let worst = v.raw.worst_eve.and_then(|s| v.raw.rate_scenarios.iter().find(|r| r.scenario == s));
    headline(
        ui,
        p,
        &worst.and_then(|w| w.eve_loss_pct_tier1).map(|x| format!("{x:.1}%")).unwrap_or_else(|| "0%".into()),
        if v.breaches.is_empty() { Standing::Comfortable } else { Standing::Breach },
        &format!("worst EVE loss as a share of Tier 1 ({}){}", fmt_money_short(v.raw.tier1_capital), worst.map(|w| format!(" · {}", w.label)).unwrap_or_default()),
        &format!("{} · {} · base NII {} · base EVE {}", v.raw.as_of.format("%d %b %Y"), v.basis, fmt_money_short(v.raw.base_nii), fmt_money_short(v.raw.base_eve)),
    );
    if !v.breaches.is_empty() {
        ui.colored_label(p.negative, format!("outside a limit: {}", v.breaches.join("; ")));
    }
    ui.add_space(8.0);
    card(ui, p, "Rate scenarios", 900.0, |ui| table(ui, "stress_rows", &["Scenario", "ΔNII 12m", "ΔNII %", "ΔEVE", "EVE loss", "% Tier 1", "Limit"], &v.rows, None));
    ui.add_space(8.0);
    wrap_row(ui, |ui| {
        card(ui, p, "ΔNII by scenario", 440.0, |ui| charts::bars(ui, &v.nii_bars, p.series[0], 160.0, fmt_money_short));
        card(ui, p, "ΔEVE by scenario", 440.0, |ui| charts::bars(ui, &v.eve_bars, p.series[1], 160.0, fmt_money_short));
    });
    ui.add_space(8.0);
    wrap_row(ui, |ui| {
        card(ui, p, "Liquidity stress", 320.0, |ui| two_cols(ui, p, &v.liquidity_rows, false));
        card(ui, p, "Cumulative stressed net outflow against the stock (90 days)", 560.0, |ui| {
            let stock = v.stock;
            let series = vec![("net outflow".to_string(), v.survival.clone()), ("stock".to_string(), vec![[1.0, stock], [90.0, stock]])];
            charts::lines(ui, "stress_survival", &series, 170.0, fmt_money_short, |x| format!("d{x:.0}"));
        });
    });
    ui.add_space(8.0);
    card(ui, p, "FX shocks", 440.0, |ui| table(ui, "stress_fx_t", &["Currency", "Shock", "Net exposure", "Revaluation"], &v.fx_rows, None));
}
