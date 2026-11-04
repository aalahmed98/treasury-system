use egui::{Align, Layout, RichText, Sense};
use egui_extras::{Column, TableBuilder};
use uuid::Uuid;

use crate::app::{App, DetailTab};
use crate::state::{SortKey, fmt_amount, fmt_local};
use crate::ui::widgets::{chip, chip_toggle, primary, th, th_sort};
use crate::ui::{break_color, match_color, pal, status_color, ticket};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    toolbar(app, ui);
    ui.separator();

    let avail = ui.available_height();
    let detail_h = if app.selected.is_some() { 210.0 } else { 0.0 };

    let (clicked, sort_clicked) = table(app, ui, avail - detail_h);

    if let Some(id) = clicked {
        let next = if app.selected == Some(id) { None } else { Some(id) };
        app.select_deal(next);
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
        ui.label(pal(ui).title("Deal blotter"));
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

        ui.add(egui::TextEdit::singleline(&mut app.search).hint_text("search ref, pair, counterparty, match…").desired_width(240.0));
        if !app.search.is_empty() && ui.small_button("✕").clicked() {
            app.search.clear();
        }
        chip_toggle(ui, &pal(ui), &mut app.only_breaks, "breaks only").on_hover_text("deals with an open break");
        chip_toggle(ui, &pal(ui), &mut app.only_provisional, "provisional only").on_hover_text("not yet confirmed by the back office");
        let bookable = app.bookable_desks();
        if primary(ui, &pal(ui), !bookable.is_empty() && app.ticket.is_none(), "Book deal").on_hover_text("open a booking ticket").clicked() {
            let d = &bookable[0];
            app.ticket = Some(ticket::TicketForm::new(d.desk_id, &d.code));
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(format!("{} of {} deals", app.filtered.len(), app.store.len())).color(pal(ui).muted));
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
        if th_sort(ui, on, &format!("{text}{arrow}")).clicked() { Some(key) } else { None }
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
            .column(Column::initial(72.0).at_least(60.0))      // match
            .column(Column::initial(30.0).at_least(26.0))      // confirmed
            .column(Column::initial(44.0).at_least(40.0))      // sources
            .column(Column::initial(64.0).at_least(56.0))      // product
            .column(Column::initial(56.0).at_least(50.0))      // side
            .column(Column::initial(74.0).at_least(60.0))      // pair
            .column(Column::initial(110.0).at_least(80.0))     // notional
            .column(Column::initial(80.0).at_least(60.0))      // rate
            .column(Column::initial(76.0).at_least(70.0))      // trade
            .column(Column::initial(76.0).at_least(70.0))      // value
            .column(Column::initial(160.0).at_least(100.0))    // counterparty
            .column(Column::initial(84.0).at_least(70.0))      // status
            .column(Column::remainder().at_least(110.0))       // updated
            .header(24.0, |mut h| {
                h.col(|_| {});
                h.col(|ui| sort_clicked = header_btn(ui, SortKey::Ref, "Ref").or(sort_clicked));
                h.col(|ui| sort_clicked = header_btn(ui, SortKey::Desk, "Desk").or(sort_clicked));
                h.col(|ui| sort_clicked = header_btn(ui, SortKey::Match, "Match").or(sort_clicked));
                h.col(|ui| { th(ui, "Cf").on_hover_text("confirmed by the back office"); });
                h.col(|ui| { th(ui, "Src").on_hover_text("K Kondor · B Bloomberg · Q Quantum · I internal booking"); });
                h.col(|ui| sort_clicked = header_btn(ui, SortKey::Product, "Product").or(sort_clicked));
                h.col(|ui| { th(ui, "Side"); });
                h.col(|ui| sort_clicked = header_btn(ui, SortKey::Pair, "Pair").or(sort_clicked));
                h.col(|ui| sort_clicked = header_btn(ui, SortKey::Notional, "Notional").or(sort_clicked));
                h.col(|ui| { th(ui, "Rate"); });
                h.col(|ui| sort_clicked = header_btn(ui, SortKey::TradeDate, "Trade").or(sort_clicked));
                h.col(|ui| sort_clicked = header_btn(ui, SortKey::ValueDate, "Value").or(sort_clicked));
                h.col(|ui| sort_clicked = header_btn(ui, SortKey::Counterparty, "Counterparty").or(sort_clicked));
                h.col(|ui| sort_clicked = header_btn(ui, SortKey::Status, "Status").or(sort_clicked));
                h.col(|ui| sort_clicked = header_btn(ui, SortKey::Updated, "Updated").or(sort_clicked));
            })
            .body(|body| {
                body.rows(22.0, filtered.len(), |mut row| {
                    let r = &rows[filtered[row.index()]];
                    let id = r.deal.id;
                    row.set_selected(selected == Some(id));
                    row.col(|ui| {
                        if flash.contains_key(&id) {
                            ui.colored_label(pal(ui).positive, "●");
                        }
                    });
                    row.col(|ui| { ui.monospace(&r.deal_ref); });
                    row.col(|ui| { ui.label(&r.desk); });
                    row.col(|ui| { ui.colored_label(match_color(&pal(ui), r.deal.match_status), &r.match_status); });
                    row.col(|ui| { if r.deal.confirmed { ui.colored_label(pal(ui).positive, "✔"); } else { ui.colored_label(pal(ui).muted, "·"); } });
                    row.col(|ui| { ui.monospace(RichText::new(&r.sources).small()); });
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
                    row.col(|ui| { ui.label(&r.counterparty); });
                    row.col(|ui| { ui.colored_label(status_color(&pal(ui), r.deal.status), &r.status); });
                    row.col(|ui| { ui.label(RichText::new(&r.updated).color(pal(ui).muted)); });
                    if row.response().clicked() {
                        clicked = Some(id);
                    }
                });
            });
    });

    (clicked, sort_clicked)
}

fn detail(app: &mut App, ui: &mut egui::Ui, id: Uuid) {
    let Some(r) = app.store.get(id) else { return };
    let d = r.deal.clone();
    let (deal_ref, product, side, pair, notional, rate, status, trader, counterparty, trade_date, value_date, maturity, updated) = (
        r.deal_ref.clone(), r.product.clone(), r.side.clone(), r.pair.clone(), r.notional.clone(), r.rate.clone(), r.status.clone(),
        r.trader.clone(), r.counterparty.clone(), r.trade_date.clone(), r.value_date.clone(), r.maturity.clone(), r.updated.clone(),
    );

    ui.horizontal(|ui| {
        ui.monospace(RichText::new(&deal_ref).family(crate::ui::theme::medium()));
        ui.label(format!("{product} {side} {pair}"));
        ui.monospace(&notional);
        ui.label("@");
        ui.monospace(&rate);
        ui.separator();
        ui.colored_label(status_color(&pal(ui), d.status), &status);
        ui.colored_label(match_color(&pal(ui), d.match_status), crate::state::match_label(d.match_status));
        if d.confirmed {
            ui.colored_label(pal(ui).positive, "confirmed");
        } else {
            ui.colored_label(pal(ui).warning, "provisional");
        }
        ui.label(RichText::new(format!("origin {}", d.origin.as_str())).color(pal(ui).muted));
        ui.label(RichText::new(format!("v{}", d.version)).color(pal(ui).muted));
        ui.separator();
        for (tab, name) in [(DetailTab::Summary, "Summary"), (DetailTab::Sources, "Sources"), (DetailTab::History, "History")] {
            if chip(ui, &pal(ui), app.detail_tab == tab, name).clicked() {
                app.open_detail_tab(tab);
            }
        }
    });

    match app.detail_tab {
        DetailTab::Summary => {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Trader").color(pal(ui).muted));
                ui.label(if trader.is_empty() { "—" } else { &trader });
                ui.label(RichText::new("Counterparty").color(pal(ui).muted));
                ui.label(&counterparty);
                ui.label(RichText::new("Trade / value / maturity").color(pal(ui).muted));
                ui.label(format!("{} / {} / {}", trade_date, value_date, if maturity.is_empty() { "—" } else { &maturity }));
                ui.label(RichText::new("Created").color(pal(ui).muted));
                ui.label(fmt_local(d.created_at));
                ui.label(RichText::new("Updated").color(pal(ui).muted));
                ui.label(&updated);
                ui.label(RichText::new("Id").color(pal(ui).muted));
                ui.monospace(RichText::new(d.id.to_string()).small());
            });
            ticket::amend_panel(app, ui, &d);
        }
        DetailTab::Sources => sources(app, ui),
        DetailTab::History => history(app, ui, &d),
    }
}

/// Every version of the deal with what changed and who changed it.
fn history(app: &App, ui: &mut egui::Ui, d: &domain::Deal) {
    let Some((id, entries)) = &app.history else {
        ui.label(RichText::new("loading history…").color(pal(ui).muted));
        return;
    };
    if *id != d.id {
        ui.label(RichText::new("loading history…").color(pal(ui).muted));
        return;
    }
    egui::ScrollArea::vertical().max_height(150.0).id_salt("history").show(ui, |ui| {
        for e in entries.iter().rev() {
            ui.horizontal_wrapped(|ui| {
                ui.monospace(RichText::new(format!("v{}", e.version)).family(crate::ui::theme::medium()));
                ui.label(RichText::new(e.op.to_lowercase()).color(pal(ui).muted));
                ui.label(RichText::new(fmt_local(e.changed_at)).color(pal(ui).muted));
                match &e.changed_by {
                    Some(u) => { ui.label(u); }
                    None => { ui.label(RichText::new(role_label(&e.changed_by_role)).color(pal(ui).muted)); }
                }
                if e.changes.is_empty() {
                    ui.label(RichText::new(if e.op == "INSERT" { "booked" } else { "no tracked field changed" }).color(pal(ui).muted).small());
                }
                for c in &e.changes {
                    ui.label(RichText::new(format!("{}: {} → {}", c.field, c.from.as_deref().unwrap_or("—"), c.to.as_deref().unwrap_or("—"))).small());
                }
            });
        }
    });
}

fn role_label(role: &str) -> &'static str {
    match role {
        "treasury_recon" => "reconciliation",
        "treasury_app" => "a user",
        "treasury_owner" => "administrator",
        _ => "system",
    }
}

/// What every source says about the selected deal, with disagreements against the golden record
/// highlighted, followed by the deal's breaks.
fn sources(app: &App, ui: &mut egui::Ui) {
    let Some(p) = &app.provenance else {
        ui.label(RichText::new("loading provenance…").color(pal(ui).muted));
        return;
    };
    let g = &p.deal;
    egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
        TableBuilder::new(ui)
            .id_salt("provenance")
            .striped(true)
            .column(Column::auto().at_least(80.0))
            .column(Column::auto().at_least(100.0))
            .column(Column::auto().at_least(70.0))
            .column(Column::auto().at_least(60.0))
            .column(Column::auto().at_least(90.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::auto().at_least(74.0))
            .column(Column::auto().at_least(74.0))
            .column(Column::remainder().at_least(140.0))
            .header(24.0, |mut h| {
                for t in ["Source", "Ref", "Lifecycle", "Link", "Notional", "Rate", "Value", "Seen", "Counterparty (as sent → canonical)"] {
                    h.col(|ui| { th(ui, t); });
                }
            })
            .body(|mut body| {
                for s in &p.sources {
                    let notional_ok = (s.notional - g.notional).abs() <= g.notional.abs() * rust_decimal::Decimal::new(1, 4);
                    let rate_ok = (s.rate - g.rate).abs() <= if g.product.is_fx() { rust_decimal::Decimal::new(5, 5) } else { rust_decimal::Decimal::new(1, 2) };
                    let value_ok = s.value_date == g.value_date;
                    let cancelled_disagree = (s.lifecycle == domain::DealStatus::Cancelled) != (g.status == domain::DealStatus::Cancelled);
                    body.row(22.0, |mut row| {
                        row.col(|ui| { ui.label(crate::ui::theme::strong(s.source.as_str())); });
                        row.col(|ui| { ui.monospace(&s.source_ref); });
                        row.col(|ui| { ui.colored_label(if cancelled_disagree { pal(ui).negative } else { status_color(&pal(ui), s.lifecycle) }, s.lifecycle.as_str()); });
                        row.col(|ui| { ui.label(RichText::new(s.link_kind.map(|k| k.as_str()).unwrap_or("—")).color(pal(ui).muted)); });
                        row.col(|ui| { ui.colored_label(if notional_ok { ui.visuals().text_color() } else { pal(ui).negative }, fmt_amount(s.notional)); });
                        row.col(|ui| {
                            let txt = if s.product.is_fx() { format!("{:.5}", s.rate) } else { format!("{:.4}%", s.rate) };
                            ui.colored_label(if rate_ok { ui.visuals().text_color() } else { pal(ui).negative }, txt);
                        });
                        row.col(|ui| { ui.colored_label(if value_ok { ui.visuals().text_color() } else { pal(ui).negative }, s.value_date.format("%d-%b-%y").to_string()); });
                        row.col(|ui| { ui.label(RichText::new(fmt_local(s.first_seen_at)).color(pal(ui).muted)); });
                        row.col(|ui| {
                            let same = s.counterparty == app_canonical(&g.counterparty);
                            ui.colored_label(if same { ui.visuals().text_color() } else { pal(ui).warning }, format!("{} → {}", s.counterparty_raw, s.counterparty));
                        });
                    });
                }
            });
        if p.breaks.is_empty() {
            ui.label(RichText::new("no breaks on this deal").color(pal(ui).muted));
        } else {
            ui.add_space(4.0);
            for b in &p.breaks {
                ui.horizontal(|ui| {
                    ui.colored_label(break_color(&pal(ui), b.kind), b.kind.as_str());
                    ui.label(RichText::new(format!("#{}", b.id)).color(pal(ui).muted));
                    if let Some(s) = b.source {
                        ui.label(format!("{} {}", s.as_str(), b.source_ref.clone().unwrap_or_default()));
                    }
                    if let Some(f) = &b.field {
                        ui.label(format!("{f}: expected {} observed {}", b.details["expected"].as_str().unwrap_or("?"), b.details["observed"].as_str().unwrap_or("?")));
                    }
                    ui.label(RichText::new(b.status.as_str().to_lowercase()).color(pal(ui).muted));
                    if let Some(r) = b.resolution {
                        ui.label(RichText::new(r.as_str().to_lowercase()).color(pal(ui).muted));
                    }
                });
            }
        }
    });
}

/// The client has no alias table; compare on the server's canonical form when available.
fn app_canonical(raw: &str) -> String {
    raw.to_uppercase()
}
