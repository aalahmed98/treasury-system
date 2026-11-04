//! Liquidity ladder: expected cash per value date for one currency, with nostro actuals where the
//! user may see them, and a drill-down into the flows and movements behind a date.

use chrono::Utc;
use egui::{Align, Layout, RichText, Sense};
use egui_extras::{Column, TableBuilder};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::app::App;
use crate::state::{fmt_amount, fmt_local};
use crate::ui::fmt::right;
use crate::ui::{match_color, pal};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.heading("Liquidity ladder");
        ui.separator();
        let ccys: Vec<String> = app.ladder.as_ref().map(|l| l.currencies.clone()).unwrap_or_default();
        let current = app.ladder_ccy.clone();
        let mut pick: Option<String> = None;
        for c in &ccys {
            if ui.selectable_label(current.as_deref() == Some(c.as_str()), c).clicked() {
                pick = Some(c.clone());
            }
        }
        if let Some(c) = pick {
            app.select_ladder_ccy(c);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Refresh").clicked() {
                app.ladder_dirty = true;
            }
            if let Some(l) = &app.ladder {
                ui.label(RichText::new(format!("as of {}", fmt_local(l.generated_at))).color(pal(ui).muted));
            }
        });
    });

    let Some(l) = app.ladder.clone() else {
        ui.label(RichText::new("loading…").color(pal(ui).muted));
        return;
    };
    ui.horizontal_wrapped(|ui| {
        match &l.opening {
            Some(o) => {
                ui.label(RichText::new("Opening").color(pal(ui).muted));
                ui.monospace(fmt_amount(o.balance));
                ui.label(RichText::new(format!("{} {} of {} ({}, {})", l.ccy, o.kind, o.statement_date.format("%d-%b"), o.bank, o.account_no)).color(pal(ui).muted).small());
            }
            None if l.liquidity_access => {
                ui.label(RichText::new("No nostro statement on file for this currency; cumulative starts at zero.").color(pal(ui).muted).small());
            }
            None => {
                ui.label(RichText::new("Expected flows from your desks' deals. Nostro balances and actual movements need ALM or treasury-head access.").color(pal(ui).muted).small());
            }
        }
    });
    ui.separator();

    let avail = ui.available_height();
    let detail_h = if app.ladder_date.is_some() { 170.0 } else { 0.0 };
    let clicked = table(app, &l, ui, avail - detail_h);
    if let Some(d) = clicked {
        let next = if app.ladder_date == Some(d) { None } else { Some(d) };
        app.select_ladder_date(next);
    }
    if app.ladder_date.is_some() {
        ui.separator();
        detail(app, ui);
    }
}

fn table(app: &App, l: &domain::LadderResponse, ui: &mut egui::Ui, height: f32) -> Option<chrono::NaiveDate> {
    let today = Utc::now().date_naive();
    let selected = app.ladder_date;
    let max_abs = l.rows.iter().map(|r| r.net_expected.abs()).max().unwrap_or(Decimal::ONE).max(Decimal::ONE);
    let mut clicked = None;
    let show_actuals = l.liquidity_access;

    ui.push_id("ladder_scroll", |ui| {
        ui.set_height(height.max(120.0));
        let mut t = TableBuilder::new(ui)
            .id_salt("ladder")
            .striped(true)
            .resizable(true)
            .sense(Sense::click())
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::initial(90.0).at_least(80.0))   // date
            .column(Column::initial(120.0).at_least(90.0))  // confirmed in
            .column(Column::initial(120.0).at_least(90.0))  // confirmed out
            .column(Column::initial(110.0).at_least(80.0))  // provisional in
            .column(Column::initial(110.0).at_least(80.0))  // provisional out
            .column(Column::initial(130.0).at_least(90.0))  // net
            .column(Column::initial(120.0).at_least(80.0))  // bar
            .column(Column::initial(150.0).at_least(100.0)); // cumulative
        if show_actuals {
            t = t.column(Column::initial(130.0).at_least(90.0)).column(Column::initial(110.0).at_least(80.0));
        }
        t.column(Column::remainder().at_least(50.0))
            .header(22.0, |mut h| {
                for s in ["Value date", "Confirmed in", "Confirmed out", "Provisional in", "Provisional out", "Net expected", "", "Cumulative"] {
                    h.col(|ui| { ui.strong(s); });
                }
                if show_actuals {
                    h.col(|ui| { ui.strong("Actual (nostro)"); });
                    h.col(|ui| { ui.strong("Difference"); });
                }
                h.col(|ui| { ui.strong("Deals"); });
            })
            .body(|body| {
                body.rows(19.0, l.rows.len(), |mut row| {
                    let r = &l.rows[row.index()];
                    row.set_selected(selected == Some(r.date));
                    let quiet = r.deals == 0 && r.actual_net.is_none();
                    let dim = |ui: &mut egui::Ui, s: String| {
                        if quiet { ui.label(RichText::new(s).color(pal(ui).muted)); } else { ui.monospace(s); }
                    };
                    row.col(|ui| {
                        let label = r.date.format("%a %d %b").to_string();
                        if r.date == today {
                            ui.label(RichText::new(label).strong().color(pal(ui).warning)).on_hover_text("today");
                        } else if r.date < today {
                            ui.label(RichText::new(label).color(pal(ui).muted));
                        } else {
                            ui.label(label);
                        }
                    });
                    row.col(|ui| { right(ui, |ui| dim(ui, fmt_amount(r.confirmed_in))); });
                    row.col(|ui| { right(ui, |ui| dim(ui, fmt_amount(r.confirmed_out))); });
                    row.col(|ui| { right(ui, |ui| { if r.provisional_in.is_zero() { dim(ui, "0".into()) } else { ui.monospace(RichText::new(fmt_amount(r.provisional_in)).color(pal(ui).warning)); } }); });
                    row.col(|ui| { right(ui, |ui| { if r.provisional_out.is_zero() { dim(ui, "0".into()) } else { ui.monospace(RichText::new(fmt_amount(r.provisional_out)).color(pal(ui).warning)); } }); });
                    row.col(|ui| {
                        right(ui, |ui| {
                            let color = if r.net_expected.is_sign_negative() { pal(ui).negative } else if r.net_expected.is_zero() { pal(ui).muted } else { pal(ui).positive };
                            ui.monospace(RichText::new(fmt_amount(r.net_expected)).color(color).strong());
                        });
                    });
                    row.col(|ui| bar(ui, r.net_expected, max_abs));
                    row.col(|ui| {
                        right(ui, |ui| {
                            let color = if r.cumulative_expected.is_sign_negative() { pal(ui).negative } else { ui.visuals().text_color() };
                            ui.monospace(RichText::new(fmt_amount(r.cumulative_expected)).color(color));
                        });
                    });
                    if show_actuals {
                        row.col(|ui| { right(ui, |ui| match r.actual_net { Some(a) => { ui.monospace(fmt_amount(a)); } None => { ui.label(RichText::new("—").color(pal(ui).muted)); } }); });
                        row.col(|ui| {
                            right(ui, |ui| match r.difference {
                                Some(d) if d.is_zero() => { ui.monospace(RichText::new("0").color(pal(ui).positive)); }
                                Some(d) => { ui.monospace(RichText::new(fmt_amount(d)).color(if d.abs() > Decimal::from(1000) { pal(ui).negative } else { pal(ui).warning })); }
                                None => { ui.label(RichText::new("—").color(pal(ui).muted)); }
                            });
                        });
                    }
                    row.col(|ui| { dim(ui, r.deals.to_string()); });
                    if row.response().clicked() {
                        clicked = Some(r.date);
                    }
                });
            });
    });
    clicked
}

/// A small signed bar, scaled to the largest net on the page.
fn bar(ui: &mut egui::Ui, value: Decimal, max_abs: Decimal) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().max(40.0), 12.0), Sense::hover());
    let painter = ui.painter();
    let mid = rect.center().x;
    let half = rect.width() / 2.0 - 2.0;
    let frac = (value / max_abs).to_f64().unwrap_or(0.0).clamp(-1.0, 1.0) as f32;
    painter.line_segment([egui::pos2(mid, rect.top()), egui::pos2(mid, rect.bottom())], egui::Stroke::new(1.0, pal(ui).muted));
    if frac != 0.0 {
        let w = half * frac.abs();
        let (x0, x1) = if frac > 0.0 { (mid, mid + w) } else { (mid - w, mid) };
        let r = egui::Rect::from_min_max(egui::pos2(x0, rect.top() + 2.0), egui::pos2(x1, rect.bottom() - 2.0));
        painter.rect_filled(r, 1.0, if frac > 0.0 { pal(ui).positive } else { pal(ui).negative });
    }
}

fn detail(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    let Some(date) = app.ladder_date else { return };
    let ccy = app.ladder_ccy.clone().unwrap_or_default();
    ui.horizontal(|ui| {
        ui.strong(format!("{ccy} on {}", date.format("%A %d %B %Y")));
        ui.label(RichText::new(format!("{} expected flows · {} nostro movements", app.ladder_flows.len(), app.ladder_movements.len())).color(p.muted));
        if ui.small_button("✕").clicked() {
            app.select_ladder_date(None);
        }
    });
    let mut open_deal = None;
    egui::ScrollArea::vertical().max_height(130.0).id_salt("ladder_detail").show(ui, |ui| {
        ui.columns(2, |cols| {
            cols[0].label(RichText::new("Expected flows").strong().small());
            for f in &app.ladder_flows {
                cols[0].horizontal(|ui| {
                    if ui.link(RichText::new(&f.deal_ref).monospace()).clicked() {
                        open_deal = Some(f.deal_id);
                    }
                    ui.label(RichText::new(&f.desk_code).color(p.muted).small());
                    ui.label(RichText::new(leg_label(&f.leg)).color(p.muted).small());
                    ui.monospace(RichText::new(fmt_amount(f.amount)).color(if f.amount.is_sign_negative() { p.negative } else { p.positive }));
                    ui.label(RichText::new(if f.confirmed { "confirmed" } else { "provisional" }).color(if f.confirmed { p.positive } else { p.warning }).small());
                    ui.colored_label(match_color(&p, f.match_status), RichText::new(crate::state::match_label(f.match_status)).small());
                });
            }
            cols[1].label(RichText::new("Nostro movements").strong().small());
            if app.ladder_movements.is_empty() {
                cols[1].label(RichText::new(if app.ladder.as_ref().is_some_and(|l| l.liquidity_access) { "none reported" } else { "not visible to this user" }).color(p.muted).small());
            }
            for m in &app.ladder_movements {
                cols[1].horizontal(|ui| {
                    ui.monospace(RichText::new(&m.reference).small());
                    ui.monospace(RichText::new(fmt_amount(m.amount)).color(if m.amount.is_sign_negative() { p.negative } else { p.positive }));
                    match &m.deal_ref_hint {
                        Some(h) => { ui.label(RichText::new(format!("for {h}")).color(p.muted).small()); }
                        None => { ui.label(RichText::new(&m.source).color(p.warning).small()); }
                    }
                });
            }
        });
    });
    if let Some(id) = open_deal {
        app.open_tab(crate::app::Tab::Blotter);
        app.select_deal(Some(id));
    }
}

fn leg_label(leg: &str) -> &'static str {
    match leg {
        "FX_CCY1" => "fx leg 1",
        "FX_CCY2" => "fx leg 2",
        "MM_START" => "principal",
        "MM_END" => "principal + interest",
        _ => "flow",
    }
}
