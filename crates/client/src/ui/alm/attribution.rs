//! NII variance attribution: one month against the prior month or the budget, as a waterfall and
//! a table per line.

use chrono::Datelike;
use domain::CompareTo;
use egui::{Panel, RichText};

use crate::alm_state::{AlmState, Standing};
use crate::app::App;
use crate::ui::alm::common::{headline, table, two_cols, with_view};
use crate::ui::charts;
use crate::ui::theme::{Palette, pal};
use crate::ui::widgets::{card, wrap_row};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    Panel::left("alm_attr_inputs").resizable(true).default_size(300.0).min_size(240.0).show(ui, |ui| inputs(&mut app.alm, ui, &p));
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("alm_attr_results").show(ui, |ui| results(&app.alm, ui, &p));
    });
}

fn inputs(alm: &mut AlmState, ui: &mut egui::Ui, p: &Palette) {
    ui.strong("Comparison");
    ui.add_space(4.0);
    let current = alm.compare_to.unwrap_or(CompareTo::Prior);
    let mut pick = None;
    ui.horizontal(|ui| {
        if ui.selectable_label(current == CompareTo::Prior, "Against the prior month").clicked() {
            pick = Some(CompareTo::Prior);
        }
        if ui.selectable_label(current == CompareTo::Budget, "Against the budget").clicked() {
            pick = Some(CompareTo::Budget);
        }
    });
    if let Some(c) = pick {
        if Some(c) != alm.compare_to {
            alm.compare_to = Some(c);
            alm.touch();
        }
    }
    ui.add_space(6.0);
    ui.label(RichText::new("Month").color(p.muted).small());
    let shown = alm.period.or(alm.attribution.result.view.as_ref().map(|v| v.raw.period_b));
    ui.horizontal(|ui| {
        if ui.small_button("◀").clicked() {
            if let Some(d) = shown {
                alm.period = Some(alm::schedule::add_months(d, -1));
                alm.touch();
            }
        }
        ui.monospace(shown.map(|d| format!("{} {}", d.format("%B"), d.year())).unwrap_or_else(|| "latest".into()));
        if ui.small_button("▶").clicked() {
            if let Some(d) = shown {
                alm.period = Some(alm::schedule::add_months(d, 1));
                alm.touch();
            }
        }
        if alm.period.is_some() && ui.small_button("latest").clicked() {
            alm.period = None;
            alm.touch();
        }
    });
    ui.add_space(8.0);
    ui.label(RichText::new("Each line's change splits into the effect of the number of days, of volume at the old rate, of rate on the old volume and the joint residual. Volume is split at portfolio level into pure growth and mix. Assets and liabilities are separate portfolios and every currency is converted at one spot, so the effects add up to the change exactly.").color(p.muted).small());
}

fn results(alm: &AlmState, ui: &mut egui::Ui, p: &Palette) {
    let Some(v) = with_view(ui, p, &alm.attribution, alm.is_pending(), "NII variance attribution") else { return };
    headline(ui, p, &v.headline, if v.raw.delta >= 0.0 { Standing::Comfortable } else { Standing::Watch }, &v.subtitle, &format!("from {} to {}", crate::ui::fmt::fmt_money_short(v.raw.nii_a), crate::ui::fmt::fmt_money_short(v.raw.nii_b)));
    ui.add_space(8.0);
    wrap_row(ui, |ui| {
        card(ui, p, "Waterfall", 620.0, |ui| charts::waterfall(ui, &v.steps, p, 200.0));
        card(ui, p, "Effects", 260.0, |ui| two_cols(ui, p, &v.effect_rows, true));
    });
    ui.add_space(8.0);
    card(ui, p, "By line", 900.0, |ui| table(ui, "attr_rows", &["Book", "Side", "Ccy", "NII before", "NII after", "Rate before", "Rate after", "Change"], &v.rows, None));
}
