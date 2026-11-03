use egui::{Align, Layout, RichText, Sense};
use egui_extras::{Column, TableBuilder};
use uuid::Uuid;

use crate::app::App;
use crate::state::{SortKey, fmt_local};
use crate::ui::{GREEN, MUTED, status_color};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    toolbar(app, ui);
    ui.separator();

    let avail = ui.available_height();
    let detail_h = if app.selected.is_some() { 74.0 } else { 0.0 };

    let (clicked, sort_clicked) = table(app, ui, avail - detail_h);

    if let Some(id) = clicked {
        app.selected = if app.selected == Some(id) { None } else { Some(id) };
    }
    if let Some(k) = sort_clicked {
        app.store.set_sort(k);
    }
    if let Some(id) = app.selected {
        ui.separator();
        detail(app, ui, id);
    }
}

fn toolbar(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.heading("Deal blotter");
        ui.separator();

        let label = match app.desk_filter.and_then(|id| app.desks.iter().find(|d| d.desk_id == id)) {
            Some(d) => d.code.clone(),
            None => "All desks".to_string(),
        };
        egui::ComboBox::from_id_salt("desk_filter").selected_text(label).show_ui(ui, |ui| {
            ui.selectable_value(&mut app.desk_filter, None, "All desks");
            for d in &app.desks {
                ui.selectable_value(&mut app.desk_filter, Some(d.desk_id), &d.code);
            }
        });

        ui.add(egui::TextEdit::singleline(&mut app.search).hint_text("search ref, pair, counterparty…").desired_width(240.0));
        if !app.search.is_empty() && ui.small_button("✕").clicked() {
            app.search.clear();
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(format!("{} of {} deals", app.filtered.len(), app.store.len())).color(MUTED));
        });
    });
}

/// Returns (row clicked, sort header clicked).
fn table(app: &App, ui: &mut egui::Ui, height: f32) -> (Option<Uuid>, Option<SortKey>) {
    let rows = app.store.rows();
    let filtered = &app.filtered;
    let flash = &app.store.flash;
    let selected = app.selected;
    let sort = app.store.sort;
    let mut clicked = None;
    let mut sort_clicked = None;

    let header_btn = |ui: &mut egui::Ui, key: SortKey, text: &str| {
        let (on, arrow) = match sort {
            Some((k, asc)) if k == key => (true, if asc { " ▲" } else { " ▼" }),
            _ => (false, ""),
        };
        if ui.selectable_label(on, format!("{text}{arrow}")).clicked() {
            Some(key)
        } else {
            None
        }
    };

    ui.push_id("blotter_scroll", |ui| {
        ui.set_height(height.max(100.0));
        TableBuilder::new(ui)
            .id_salt("blotter")
            .striped(true)
            .resizable(true)
            .sense(Sense::click())
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::exact(14.0))                        // flash marker
            .column(Column::initial(92.0).at_least(70.0))      // ref
            .column(Column::initial(44.0).at_least(40.0))      // desk
            .column(Column::initial(64.0).at_least(56.0))      // product
            .column(Column::initial(60.0).at_least(50.0))      // side
            .column(Column::initial(74.0).at_least(60.0))      // pair
            .column(Column::initial(110.0).at_least(80.0))     // notional
            .column(Column::initial(80.0).at_least(60.0))      // rate
            .column(Column::initial(76.0).at_least(70.0))      // trade
            .column(Column::initial(76.0).at_least(70.0))      // value
            .column(Column::initial(76.0).at_least(70.0))      // maturity
            .column(Column::initial(170.0).at_least(100.0))    // counterparty
            .column(Column::initial(84.0).at_least(70.0))      // status
            .column(Column::remainder().at_least(110.0))       // updated
            .header(22.0, |mut h| {
                h.col(|_| {});
                for (key, text) in [
                    (SortKey::Ref, "Ref"),
                    (SortKey::Desk, "Desk"),
                    (SortKey::Product, "Product"),
                ] {
                    h.col(|ui| sort_clicked = header_btn(ui, key, text).or(sort_clicked));
                }
                h.col(|ui| { ui.strong("Side"); });
                h.col(|ui| sort_clicked = header_btn(ui, SortKey::Pair, "Pair").or(sort_clicked));
                h.col(|ui| sort_clicked = header_btn(ui, SortKey::Notional, "Notional").or(sort_clicked));
                h.col(|ui| { ui.strong("Rate"); });
                h.col(|ui| sort_clicked = header_btn(ui, SortKey::TradeDate, "Trade").or(sort_clicked));
                h.col(|ui| sort_clicked = header_btn(ui, SortKey::ValueDate, "Value").or(sort_clicked));
                h.col(|ui| { ui.strong("Maturity"); });
                h.col(|ui| sort_clicked = header_btn(ui, SortKey::Counterparty, "Counterparty").or(sort_clicked));
                h.col(|ui| sort_clicked = header_btn(ui, SortKey::Status, "Status").or(sort_clicked));
                h.col(|ui| sort_clicked = header_btn(ui, SortKey::Updated, "Updated").or(sort_clicked));
            })
            .body(|body| {
                body.rows(19.0, filtered.len(), |mut row| {
                    let r = &rows[filtered[row.index()]];
                    let id = r.deal.id;
                    row.set_selected(selected == Some(id));
                    row.col(|ui| {
                        if flash.contains_key(&id) {
                            ui.colored_label(GREEN, "●");
                        }
                    });
                    row.col(|ui| { ui.monospace(&r.deal_ref); });
                    row.col(|ui| { ui.label(&r.desk); });
                    row.col(|ui| { ui.label(&r.product); });
                    row.col(|ui| { ui.label(&r.side); });
                    row.col(|ui| { ui.monospace(&r.pair); });
                    row.col(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| { ui.monospace(&r.notional); });
                    });
                    row.col(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| { ui.monospace(&r.rate); });
                    });
                    row.col(|ui| { ui.label(&r.trade_date); });
                    row.col(|ui| { ui.label(&r.value_date); });
                    row.col(|ui| { ui.label(&r.maturity); });
                    row.col(|ui| { ui.label(&r.counterparty); });
                    row.col(|ui| { ui.colored_label(status_color(r.deal.status), &r.status); });
                    row.col(|ui| { ui.label(RichText::new(&r.updated).color(MUTED)); });
                    if row.response().clicked() {
                        clicked = Some(id);
                    }
                });
            });
    });

    (clicked, sort_clicked)
}

fn detail(app: &App, ui: &mut egui::Ui, id: Uuid) {
    let Some(r) = app.store.get(id) else { return };
    let d = &r.deal;
    ui.horizontal_wrapped(|ui| {
        ui.monospace(RichText::new(&r.deal_ref).strong());
        ui.label(format!("{} {} {}", r.product, r.side, r.pair));
        ui.monospace(&r.notional);
        ui.label("@");
        ui.monospace(&r.rate);
        ui.separator();
        ui.colored_label(status_color(d.status), &r.status);
        ui.label(RichText::new(format!("v{}", d.version)).color(MUTED));
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Trader").color(MUTED));
        ui.label(&r.trader);
        ui.label(RichText::new("Counterparty").color(MUTED));
        ui.label(&r.counterparty);
        ui.label(RichText::new("Trade / value / maturity").color(MUTED));
        ui.label(format!("{} / {} / {}", r.trade_date, r.value_date, if r.maturity.is_empty() { "—" } else { &r.maturity }));
        ui.label(RichText::new("Created").color(MUTED));
        ui.label(fmt_local(d.created_at));
        ui.label(RichText::new("Updated").color(MUTED));
        ui.label(&r.updated);
        ui.label(RichText::new("Id").color(MUTED));
        ui.monospace(RichText::new(d.id.to_string()).small());
    });
}
