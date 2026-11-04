//! Interest rate differential calculator: covered interest parity on the latest curves, run here
//! without a server round trip.

use egui::{Panel, RichText};

use crate::alm_state::IrdState;
use crate::app::App;
use crate::ui::alm::common::{table, two_cols};
use crate::ui::fmt::fmt_amount;
use crate::ui::theme::{Palette, pal};
use crate::ui::widgets::{card, wrap_row};

const CCYS: [&str; 6] = ["BHD", "USD", "EUR", "GBP", "SAR", "AED"];
const TENORS: [(&str, i64); 6] = [("1W", 7), ("1M", 30), ("3M", 91), ("6M", 182), ("1Y", 365), ("2Y", 730)];

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    Panel::left("alm_ird_inputs").resizable(true).default_size(320.0).min_size(260.0).show(ui, |ui| inputs(&mut app.alm.ird, ui, &p));
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("alm_ird_results").show(ui, |ui| results(&app.alm.ird, ui, &p));
    });
}

fn ccy_combo(ui: &mut egui::Ui, id: &str, value: &mut String) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id).selected_text(value.as_str()).width(70.0).show_ui(ui, |ui| {
        for c in CCYS {
            if ui.selectable_label(value == c, c).clicked() && value != c {
                *value = c.into();
                changed = true;
            }
        }
    });
    changed
}

fn inputs(ird: &mut IrdState, ui: &mut egui::Ui, p: &Palette) {
    ui.strong("Pair and tenor");
    ui.add_space(4.0);
    let mut market = false;
    ui.horizontal(|ui| {
        market |= ccy_combo(ui, "ird_base", &mut ird.base_ccy);
        ui.label("/");
        market |= ccy_combo(ui, "ird_quote", &mut ird.quote_ccy);
        ui.label(RichText::new("quote per base").color(p.muted).small());
    });
    egui::Grid::new("ird_inputs").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
        ui.label("Days");
        market |= ui.add(egui::DragValue::new(&mut ird.days).range(1..=3650).speed(1.0)).changed();
        ui.end_row();
        ui.label("Follow the market");
        if ui.checkbox(&mut ird.follow_market, "").changed() {
            market = true;
        }
        ui.end_row();
        ui.label("Spot");
        if ui.add(egui::DragValue::new(&mut ird.spot).range(0.0001..=10000.0).speed(0.0001).fixed_decimals(5)).changed() {
            ird.follow_market = false;
        }
        ui.end_row();
        ui.label(format!("{} rate", ird.base_ccy));
        if ui.add(egui::DragValue::new(&mut ird.base_rate).range(-5.0..=50.0).speed(0.01).suffix("%").fixed_decimals(3)).changed() {
            ird.follow_market = false;
        }
        ui.end_row();
        ui.label(format!("{} rate", ird.quote_ccy));
        if ui.add(egui::DragValue::new(&mut ird.quote_rate).range(-5.0..=50.0).speed(0.01).suffix("%").fixed_decimals(3)).changed() {
            ird.follow_market = false;
        }
        ui.end_row();
        ui.label(format!("Notional ({})", ird.base_ccy));
        ui.add(egui::DragValue::new(&mut ird.notional).range(0.0..=1e12).speed(100_000.0).fixed_decimals(0));
        ui.end_row();
        ui.label("Forward to solve for");
        ui.add(egui::DragValue::new(&mut ird.forward_input).range(0.0..=10000.0).speed(0.0001).fixed_decimals(5));
        ui.end_row();
    });
    if market {
        ird.refresh_from_market();
    }
    ui.add_space(6.0);
    ui.label(RichText::new(if ird.rates_as_of.is_empty() { "no market curves loaded".into() } else { format!("curves and spots as of {}", ird.rates_as_of) }).color(p.muted).small());
    ui.label(RichText::new("Simple interest on each currency's basis (ACT/365 for GBP, ACT/360 otherwise). Typing a spot or rate stops following the market.").color(p.muted).small());
}

fn results(ird: &IrdState, ui: &mut egui::Ui, p: &Palette) {
    let bb = alm::ird::basis(&ird.base_ccy);
    let qb = alm::ird::basis(&ird.quote_ccy);
    let pip = alm::ird::pip_factor(&ird.quote_ccy);
    let f = alm::ird::forward(ird.spot, ird.days, ird.base_rate, bb, ird.quote_rate, qb);
    let points = (f - ird.spot) * pip;
    let carry = alm::ird::carry_per_day(ird.notional, ird.spot, ird.base_rate, bb, ird.quote_rate, qb);
    ui.strong(format!("{}/{} {} days", ird.base_ccy, ird.quote_ccy, ird.days));
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{f:.5}")).size(34.0).strong().color(if points >= 0.0 { p.positive } else { p.negative }));
        ui.vertical(|ui| {
            ui.label(RichText::new(format!("forward outright · {points:+.1} pips")).color(p.muted).small());
            ui.label(RichText::new(format!("differential {:+.3}% ({} over {})", ird.quote_rate - ird.base_rate, ird.quote_ccy, ird.base_ccy)).color(p.muted).small());
        });
    });
    ui.add_space(8.0);
    let mut rows = vec![
        ("Spot".into(), format!("{:.5}", ird.spot)),
        ("Forward points".into(), format!("{points:+.1}")),
        (format!("Carry per day ({})", ird.quote_ccy), fmt_amount(rust_decimal::Decimal::from_f64_retain(carry).unwrap_or_default().round_dp(2))),
        (format!("Carry over the tenor ({})", ird.quote_ccy), fmt_amount(rust_decimal::Decimal::from_f64_retain(carry * ird.days as f64).unwrap_or_default().round_dp(2))),
        ("Break-even spot move".into(), format!("{:+.1} pips", points)),
    ];
    if ird.forward_input > 0.0 {
        if let Some(r) = alm::ird::implied_quote_rate(ird.spot, ird.forward_input, ird.days, ird.base_rate, bb, qb) {
            rows.push((format!("Implied {} rate for forward {:.5}", ird.quote_ccy, ird.forward_input), format!("{r:.3}%")));
        }
        if let Some(r) = alm::ird::implied_base_rate(ird.spot, ird.forward_input, ird.days, ird.quote_rate, qb, bb) {
            rows.push((format!("Implied {} rate for forward {:.5}", ird.base_ccy, ird.forward_input), format!("{r:.3}%")));
        }
    }
    let tenor_rows: Vec<[String; 5]> = TENORS
        .iter()
        .map(|(t, d)| {
            let (br, qr) = match &ird.rates.view {
                Some(m) if ird.follow_market => (ird.market_rate(m, &ird.base_ccy, *d).unwrap_or(ird.base_rate), ird.market_rate(m, &ird.quote_ccy, *d).unwrap_or(ird.quote_rate)),
                _ => (ird.base_rate, ird.quote_rate),
            };
            let q = alm::ird::quote(t, *d, ird.spot, br, &ird.base_ccy, qr, &ird.quote_ccy);
            [t.to_string(), format!("{br:.3}%"), format!("{qr:.3}%"), format!("{:.5}", q.forward), format!("{:+.1}", q.points)]
        })
        .collect();
    wrap_row(ui, |ui| {
        card(ui, p, "This tenor", 360.0, |ui| two_cols(ui, p, &rows, false));
        card(ui, p, "Standard tenors from the curves", 480.0, |ui| {
            table(ui, "ird_tenors", &["Tenor", &format!("{} rate", ird.base_ccy), &format!("{} rate", ird.quote_ccy), "Forward", "Points"], &tenor_rows, None)
        });
    });
}
