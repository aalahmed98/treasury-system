use egui::RichText;
use egui_extras::{Column, TableBuilder};

use crate::app::App;
use crate::state::fmt_amount;
use crate::ui::pal;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ui.heading("Positions");
    ui.label(RichText::new("Net notional per desk and currency, from the server's entitlement-filtered view.").color(pal(ui).muted).small());
    ui.add_space(4.0);

    let rows = &app.positions;
    TableBuilder::new(ui)
        .id_salt("positions")
        .striped(true)
        .column(Column::auto().at_least(50.0))
        .column(Column::auto().at_least(44.0))
        .column(Column::remainder().at_least(120.0))
        .column(Column::auto().at_least(44.0))
        .header(20.0, |mut h| {
            h.col(|ui| { ui.strong("Desk"); });
            h.col(|ui| { ui.strong("Ccy"); });
            h.col(|ui| { ui.strong("Net"); });
            h.col(|ui| { ui.strong("Deals"); });
        })
        .body(|body| {
            body.rows(18.0, rows.len(), |mut row| {
                let p = &rows[row.index()];
                row.col(|ui| { ui.label(&p.desk_code); });
                row.col(|ui| { ui.monospace(&p.ccy); });
                row.col(|ui| {
                    let color = if p.net_notional.is_sign_negative() { pal(ui).negative } else { pal(ui).positive };
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.monospace(RichText::new(fmt_amount(p.net_notional)).color(color));
                    });
                });
                row.col(|ui| { ui.monospace(p.deal_count.to_string()); });
            });
        });
}
