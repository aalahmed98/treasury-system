//! Repricing gap: behavioural lives and cumulative limits on the left, the buckets on the right.


use crate::alm_state::{AlmState, Standing};
use crate::app::App;
use crate::ui::alm::common::{assumptions_header, group, headline, pct, results_panel, side_panel, table, with_view};
use crate::ui::charts;
use crate::ui::fmt::fmt_money_short;
use crate::ui::theme::{Palette, pal};
use crate::ui::widgets::{card, wrap_row};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    side_panel(ui, "alm_gap_assumptions", 320.0, 260.0, |ui| assumptions(&mut app.alm, ui, &p));
    results_panel(ui, "alm_gap_results", |ui| results(&app.alm, ui, &p));
}

fn assumptions(alm: &mut AlmState, ui: &mut egui::Ui, p: &Palette) {
    assumptions_header(alm, ui, p);
    let Some(mut params) = alm.working() else { return };
    let mut changed = false;
    let g = &mut params.gap;
    ui.add_space(4.0);
    group(ui, p, "Behavioural life (months)");
    egui::Grid::new("gap_nmd").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
        for (product, v) in g.nmd_behavioural_months.iter_mut() {
            ui.label(product.replace('_', " ").to_lowercase());
            changed |= ui.add(egui::DragValue::new(v).range(1.0..=240.0).speed(1.0).fixed_decimals(0)).changed();
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    group(ui, p, "Cumulative gap limits (% assets)");
    egui::Grid::new("gap_limits").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
        // In bucket order, not alphabetical.
        let mut keys: Vec<String> = g.cumulative_gap_limit_pct_assets.keys().cloned().collect();
        keys.sort_by_key(|k| alm::bucket::GAP_BUCKETS.iter().position(|(l, _)| l == k).unwrap_or(usize::MAX));
        for bucket in keys {
            if let Some(v) = g.cumulative_gap_limit_pct_assets.get_mut(&bucket) {
                ui.label(bucket.as_str());
                changed |= pct(ui, v, 100.0);
                ui.end_row();
            }
        }
    });
    if changed {
        alm.overrides = Some(params);
        alm.touch();
    }
}

fn results(alm: &AlmState, ui: &mut egui::Ui, p: &Palette) {
    let Some(v) = with_view(ui, p, &alm.gap, alm.is_pending(), "Repricing gap") else { return };
    let y1 = v.raw.buckets.iter().find(|b| b.label == "1Y");
    let standing = if v.breaches.is_empty() { Standing::Comfortable } else { Standing::Breach };
    headline(
        ui,
        p,
        &y1.map(|b| format!("{:+.1}%", b.cumulative_pct_assets)).unwrap_or_else(|| "—".into()),
        standing,
        &format!("cumulative gap at one year, as a share of total assets of {}", fmt_money_short(v.raw.total_assets)),
        &format!("{} · {} · in {}", v.raw.as_of.format("%d %b %Y"), v.basis, v.raw.reporting_ccy),
    );
    if !v.breaches.is_empty() {
        ui.colored_label(p.negative, format!("outside the limit: {}", v.breaches.join("; ")));
    }
    ui.add_space(8.0);
    wrap_row(ui, |ui| {
        card(ui, p, "Gap per bucket (assets less liabilities)", 460.0, |ui| charts::bars(ui, &v.gap_bars, p.series[0], 170.0, fmt_money_short));
        card(ui, p, "Cumulative gap", 460.0, |ui| charts::bars(ui, &v.cumulative_bars, p.series[2], 170.0, fmt_money_short));
    });
    ui.add_space(8.0);
    wrap_row(ui, |ui| {
        card(ui, p, "Buckets", 680.0, |ui| table(ui, "gap_rows", &["Bucket", "Assets", "Liabilities", "Gap", "Cumulative", "Cumulative % (limit)"], &v.rows, None));
        card(ui, p, "By currency at one year", 260.0, |ui| table(ui, "gap_ccy", &["Currency", "Cumulative", "% assets"], &v.ccy_rows, None));
    });
}
