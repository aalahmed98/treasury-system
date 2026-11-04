//! Booking ticket (a floating window) and the amendment form used in the deal detail pane.

use chrono::{Duration, NaiveDate, Utc};
use domain::{AmendDeal, DealStatus, NewDeal, Product, Side};
use egui::{RichText, TextEdit};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::app::App;
use crate::net::Command;
use crate::ui::pal;

pub struct TicketForm {
    pub desk_id: i16,
    pub product: Product,
    pub side: Side,
    pub ccy1: String,
    pub ccy2: String,
    pub notional: String,
    pub rate: String,
    pub trade_date: String,
    pub value_date: String,
    pub maturity_date: String,
    pub counterparty: String,
    pub error: Option<String>,
    pub submitting: bool,
}

impl TicketForm {
    pub fn new(desk_id: i16, desk_code: &str) -> Self {
        let today = Utc::now().date_naive();
        let mm = desk_code == "MM";
        Self {
            desk_id,
            product: if mm { Product::MmDepo } else { Product::FxSpot },
            side: if mm { Side::Lend } else { Side::Buy },
            ccy1: if mm { "BHD".into() } else { "USD".into() },
            ccy2: if mm { String::new() } else { "BHD".into() },
            notional: "1000000".into(),
            rate: if mm { "5.25".into() } else { "0.37600".into() },
            trade_date: today.to_string(),
            value_date: (today + Duration::days(2)).to_string(),
            maturity_date: if mm { (today + Duration::days(32)).to_string() } else { String::new() },
            counterparty: String::new(),
            error: None,
            submitting: false,
        }
    }

    pub fn to_new_deal(&self) -> Result<NewDeal, String> {
        let ccy = |s: &str, name: &str| -> Result<String, String> {
            let c = s.trim().to_uppercase();
            if c.len() == 3 && c.bytes().all(|b| b.is_ascii_uppercase()) { Ok(c) } else { Err(format!("{name} must be a 3-letter code")) }
        };
        let dec = |s: &str, name: &str| -> Result<Decimal, String> {
            s.trim().replace(',', "").parse::<Decimal>().map_err(|_| format!("{name} is not a number"))
        };
        let date = |s: &str, name: &str| -> Result<NaiveDate, String> {
            NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").map_err(|_| format!("{name} must be YYYY-MM-DD"))
        };
        let fx = self.product.is_fx();
        let notional = dec(&self.notional, "notional")?;
        if notional <= Decimal::ZERO {
            return Err("notional must be positive".into());
        }
        let trade_date = date(&self.trade_date, "trade date")?;
        let value_date = date(&self.value_date, "value date")?;
        if value_date < trade_date {
            return Err("value date is before the trade date".into());
        }
        let maturity_date = if fx {
            None
        } else {
            let m = date(&self.maturity_date, "maturity date")?;
            if m < value_date {
                return Err("maturity date is before the value date".into());
            }
            Some(m)
        };
        if self.counterparty.trim().is_empty() {
            return Err("counterparty is required".into());
        }
        Ok(NewDeal {
            desk_id: self.desk_id,
            product: self.product,
            side: self.side,
            ccy1: ccy(&self.ccy1, "ccy1")?,
            ccy2: if fx { Some(ccy(&self.ccy2, "ccy2")?) } else { None },
            notional,
            rate: dec(&self.rate, "rate")?,
            trade_date,
            value_date,
            maturity_date,
            counterparty: self.counterparty.trim().to_string(),
        })
    }
}

pub struct AmendForm {
    pub id: Uuid,
    pub counterparty: String,
    pub rate: String,
    pub notional: String,
    pub value_date: String,
    pub maturity_date: String,
    pub confirm_cancel: bool,
}

impl AmendForm {
    pub fn from_deal(d: &domain::Deal) -> Self {
        Self {
            id: d.id,
            counterparty: d.counterparty.clone(),
            rate: d.rate.normalize().to_string(),
            notional: d.notional.normalize().to_string(),
            value_date: d.value_date.to_string(),
            maturity_date: d.maturity_date.map(|m| m.to_string()).unwrap_or_default(),
            confirm_cancel: false,
        }
    }

    /// Only the fields that differ from the deal are sent.
    pub fn diff(&self, d: &domain::Deal) -> Result<AmendDeal, String> {
        let mut out = AmendDeal::default();
        if self.counterparty.trim() != d.counterparty {
            out.counterparty = Some(self.counterparty.trim().to_string());
        }
        let rate: Decimal = self.rate.trim().parse().map_err(|_| "rate is not a number".to_string())?;
        if rate != d.rate {
            out.rate = Some(rate);
        }
        let notional: Decimal = self.notional.trim().replace(',', "").parse().map_err(|_| "notional is not a number".to_string())?;
        if notional != d.notional {
            out.notional = Some(notional);
        }
        let value_date = NaiveDate::parse_from_str(self.value_date.trim(), "%Y-%m-%d").map_err(|_| "value date must be YYYY-MM-DD".to_string())?;
        if value_date != d.value_date {
            out.value_date = Some(value_date);
        }
        if !self.maturity_date.trim().is_empty() {
            let m = NaiveDate::parse_from_str(self.maturity_date.trim(), "%Y-%m-%d").map_err(|_| "maturity date must be YYYY-MM-DD".to_string())?;
            if Some(m) != d.maturity_date {
                out.maturity_date = Some(m);
            }
        }
        Ok(out)
    }
}

/// The floating booking window, drawn over whichever tab is open.
pub fn window(app: &mut App, ctx: &egui::Context) {
    if app.ticket.is_none() {
        return;
    }
    let desks = app.bookable_desks();
    let mut open = true;
    let mut submit = false;
    let mut close = false;
    egui::Window::new("Book a deal").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
        let Some(t) = &mut app.ticket else { return };
        egui::Grid::new("ticket_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Desk");
            let current = desks.iter().find(|d| d.desk_id == t.desk_id).map(|d| d.code.clone()).unwrap_or_default();
            egui::ComboBox::from_id_salt("ticket_desk").selected_text(current).show_ui(ui, |ui| {
                for d in &desks {
                    ui.selectable_value(&mut t.desk_id, d.desk_id, &d.code);
                }
            });
            ui.end_row();

            ui.label("Product");
            egui::ComboBox::from_id_salt("ticket_product").selected_text(t.product.as_str()).show_ui(ui, |ui| {
                for p in Product::ALL {
                    if ui.selectable_value(&mut t.product, *p, p.as_str()).clicked() {
                        t.side = if p.is_fx() { Side::Buy } else { Side::Lend };
                        if !p.is_fx() {
                            t.ccy2.clear();
                        }
                    }
                }
            });
            ui.end_row();

            ui.label("Side");
            let sides: &[Side] = if t.product.is_fx() { &[Side::Buy, Side::Sell] } else { &[Side::Lend, Side::Borrow] };
            ui.horizontal(|ui| {
                for s in sides {
                    ui.selectable_value(&mut t.side, *s, s.as_str());
                }
            });
            ui.end_row();

            ui.label(if t.product.is_fx() { "Pair" } else { "Currency" });
            ui.horizontal(|ui| {
                ui.add(TextEdit::singleline(&mut t.ccy1).desired_width(44.0).char_limit(3));
                if t.product.is_fx() {
                    ui.label("/");
                    ui.add(TextEdit::singleline(&mut t.ccy2).desired_width(44.0).char_limit(3));
                }
            });
            ui.end_row();

            ui.label(if t.product.is_fx() { "Notional (ccy1)" } else { "Principal" });
            ui.add(TextEdit::singleline(&mut t.notional).desired_width(160.0));
            ui.end_row();

            ui.label(if t.product.is_fx() { "Rate" } else { "Rate (%)" });
            ui.add(TextEdit::singleline(&mut t.rate).desired_width(160.0));
            ui.end_row();

            ui.label("Trade date");
            ui.add(TextEdit::singleline(&mut t.trade_date).desired_width(160.0));
            ui.end_row();
            ui.label("Value date");
            ui.add(TextEdit::singleline(&mut t.value_date).desired_width(160.0));
            ui.end_row();
            if !t.product.is_fx() {
                ui.label("Maturity date");
                ui.add(TextEdit::singleline(&mut t.maturity_date).desired_width(160.0));
                ui.end_row();
            }

            ui.label("Counterparty");
            ui.add(TextEdit::singleline(&mut t.counterparty).desired_width(220.0));
            ui.end_row();
        });

        if let Some(e) = &t.error {
            ui.add_space(4.0);
            ui.label(RichText::new(e).color(pal(ui).negative));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(!t.submitting, egui::Button::new("Book")).clicked() {
                submit = true;
            }
            if t.submitting {
                ui.spinner();
            }
            if ui.button("Discard").clicked() {
                close = true;
            }
            ui.label(RichText::new("Booked deals enter the reconciliation log immediately.").color(pal(ui).muted).small());
        });
    });
    if submit {
        if let Some(t) = &mut app.ticket {
            match t.to_new_deal() {
                Ok(deal) => {
                    t.error = None;
                    t.submitting = true;
                    app.net.send(Command::BookDeal(deal));
                }
                Err(e) => t.error = Some(e),
            }
        }
    }
    if close || !open {
        app.ticket = None;
    }
}

/// Amend and cancel controls for an internal booking the user may book on.
pub fn amend_panel(app: &mut App, ui: &mut egui::Ui, deal: &domain::Deal) {
    let editable = deal.origin == domain::Source::Internal && app.can_book(deal.desk_id) && !deal.status.is_terminal();
    if !editable {
        if deal.origin != domain::Source::Internal {
            ui.label(RichText::new("created by reconciliation from an external source; its economics follow that source").color(pal(ui).muted).small());
        } else if deal.status.is_terminal() {
            ui.label(RichText::new(format!("{} deals cannot be changed", deal.status.as_str().to_lowercase())).color(pal(ui).muted).small());
        }
        return;
    }
    if app.amend.as_ref().is_none_or(|a| a.id != deal.id) {
        if ui.button("Amend").clicked() {
            app.amend = Some(AmendForm::from_deal(deal));
        }
        return;
    }
    let mut save = false;
    let mut discard = false;
    let mut cancel_deal = false;
    let mut err: Option<String> = None;
    if let Some(a) = &mut app.amend {
        ui.horizontal_wrapped(|ui| {
            ui.label("Counterparty");
            ui.add(TextEdit::singleline(&mut a.counterparty).desired_width(200.0));
            ui.label("Rate");
            ui.add(TextEdit::singleline(&mut a.rate).desired_width(90.0));
            ui.label("Notional");
            ui.add(TextEdit::singleline(&mut a.notional).desired_width(110.0));
            ui.label("Value date");
            ui.add(TextEdit::singleline(&mut a.value_date).desired_width(96.0));
            if !deal.product.is_fx() {
                ui.label("Maturity");
                ui.add(TextEdit::singleline(&mut a.maturity_date).desired_width(96.0));
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Save amendment").clicked() {
                save = true;
            }
            if ui.button("Discard").clicked() {
                discard = true;
            }
            ui.separator();
            ui.checkbox(&mut a.confirm_cancel, "I want to cancel this deal");
            if ui.add_enabled(a.confirm_cancel, egui::Button::new(RichText::new("Cancel deal").color(pal(ui).negative))).clicked() {
                cancel_deal = true;
            }
        });
        if save {
            match a.diff(deal) {
                Ok(req) => {
                    if req.counterparty.is_none() && req.rate.is_none() && req.notional.is_none() && req.value_date.is_none() && req.maturity_date.is_none() {
                        err = Some("nothing changed".into());
                    } else {
                        app.net.send(Command::AmendDeal { id: deal.id, req });
                        discard = true;
                    }
                }
                Err(e) => err = Some(e),
            }
        }
        if cancel_deal {
            app.net.send(Command::CancelDeal(deal.id));
            discard = true;
        }
    }
    if let Some(e) = err {
        ui.label(RichText::new(e).color(pal(ui).negative));
    }
    if discard {
        app.amend = None;
    }
    let _ = DealStatus::Booked;
}
