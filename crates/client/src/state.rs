//! Client-side deal store: rows are formatted once on arrival, never per frame.

use std::collections::HashMap;
use std::time::Instant;

use chrono::{DateTime, Local, Utc};
use domain::{Deal, DealStatus, Product};
use rust_decimal::Decimal;
use uuid::Uuid;

pub struct DealRow {
    pub deal: Deal,
    pub deal_ref: String,
    pub desk: String,
    pub product: String,
    pub side: String,
    pub pair: String,
    pub notional: String,
    pub rate: String,
    pub trade_date: String,
    pub value_date: String,
    pub maturity: String,
    pub counterparty: String,
    pub trader: String,
    pub status: String,
    pub updated: String,
    /// Lower-cased haystack for the search box.
    pub search: String,
}

impl DealRow {
    pub fn new(deal: Deal) -> Self {
        let pair = match &deal.ccy2 {
            Some(c2) => format!("{}/{}", deal.ccy1, c2),
            None => deal.ccy1.clone(),
        };
        let rate = if deal.product.is_fx() { format!("{:.5}", deal.rate) } else { format!("{:.4}%", deal.rate) };
        let search = format!(
            "{} {} {} {} {} {} {}",
            deal.deal_ref, deal.desk_code, deal.product, deal.side, pair, deal.counterparty, deal.trader_name
        )
        .to_lowercase();
        Self {
            deal_ref: deal.deal_ref.clone(),
            desk: deal.desk_code.clone(),
            product: product_label(deal.product).to_string(),
            side: deal.side.as_str().to_string(),
            pair,
            notional: fmt_amount(deal.notional),
            rate,
            trade_date: deal.trade_date.format("%d-%b-%y").to_string(),
            value_date: deal.value_date.format("%d-%b-%y").to_string(),
            maturity: deal.maturity_date.map(|d| d.format("%d-%b-%y").to_string()).unwrap_or_default(),
            counterparty: deal.counterparty.clone(),
            trader: deal.trader_name.clone(),
            status: deal.status.as_str().to_string(),
            updated: fmt_local(deal.updated_at),
            search,
            deal,
        }
    }
}

pub fn product_label(p: Product) -> &'static str {
    match p {
        Product::FxSpot => "FX Spot",
        Product::FxFwd => "FX Fwd",
        Product::MmDepo => "MM Depo",
        Product::MmLoan => "MM Loan",
    }
}

pub fn fmt_local(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%d-%b %H:%M:%S").to_string()
}

/// 1234567.5 -> "1,234,567.5"
pub fn fmt_amount(d: Decimal) -> String {
    let d = d.normalize();
    let s = d.abs().to_string();
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s.as_str(), None),
    };
    let mut out = String::with_capacity(s.len() + s.len() / 3 + 1);
    if d.is_sign_negative() {
        out.push('-');
    }
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if let Some(f) = frac {
        out.push('.');
        out.push_str(f);
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Updated,
    Ref,
    Desk,
    Product,
    Pair,
    Notional,
    TradeDate,
    ValueDate,
    Counterparty,
    Status,
}

#[derive(Default)]
pub struct DealStore {
    rows: Vec<DealRow>,
    index: HashMap<Uuid, usize>,
    pub as_of: Option<DateTime<Utc>>,
    pub sort: Option<(SortKey, bool)>,
    /// Bumped on every change so cached filters know to recompute.
    pub version: u64,
    /// Rows updated recently, for the flash marker.
    pub flash: HashMap<Uuid, Instant>,
}

impl DealStore {
    pub fn rows(&self) -> &[DealRow] {
        &self.rows
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn get(&self, id: Uuid) -> Option<&DealRow> {
        self.index.get(&id).map(|&i| &self.rows[i])
    }

    pub fn replace(&mut self, deals: Vec<Deal>, as_of: DateTime<Utc>) {
        self.rows = deals.into_iter().map(DealRow::new).collect();
        self.as_of = Some(as_of);
        self.resort();
    }

    /// Insert or update one deal. Returns true if it was new.
    pub fn upsert(&mut self, deal: Deal) -> bool {
        let id = deal.id;
        let is_new = match self.index.get(&id) {
            Some(&i) => {
                if self.rows[i].deal.version >= deal.version {
                    return false; // stale or duplicate notification
                }
                self.rows[i] = DealRow::new(deal);
                false
            }
            None => {
                self.rows.push(DealRow::new(deal));
                true
            }
        };
        self.flash.insert(id, Instant::now());
        self.resort();
        is_new
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn set_sort(&mut self, key: SortKey) {
        self.sort = match self.sort {
            Some((k, asc)) if k == key => Some((key, !asc)),
            _ => Some((key, true)),
        };
        self.resort();
    }

    fn resort(&mut self) {
        let (key, asc) = self.sort.unwrap_or((SortKey::Updated, false));
        self.rows.sort_by(|a, b| {
            let (a, b) = (&a.deal, &b.deal);
            let ord = match key {
                SortKey::Updated => a.updated_at.cmp(&b.updated_at),
                SortKey::Ref => a.deal_ref.cmp(&b.deal_ref),
                SortKey::Desk => a.desk_code.cmp(&b.desk_code),
                SortKey::Product => a.product.as_str().cmp(b.product.as_str()),
                SortKey::Pair => (&a.ccy1, &a.ccy2).cmp(&(&b.ccy1, &b.ccy2)),
                SortKey::Notional => a.notional.cmp(&b.notional),
                SortKey::TradeDate => a.trade_date.cmp(&b.trade_date),
                SortKey::ValueDate => a.value_date.cmp(&b.value_date),
                SortKey::Counterparty => a.counterparty.cmp(&b.counterparty),
                SortKey::Status => status_rank(a.status).cmp(&status_rank(b.status)),
            };
            let ord = ord.then_with(|| b.updated_at.cmp(&a.updated_at));
            if asc { ord } else { ord.reverse() }
        });
        self.index = self.rows.iter().enumerate().map(|(i, r)| (r.deal.id, i)).collect();
        self.version += 1;
    }

    /// Drop expired flashes; returns true if any are still live (caller keeps repainting).
    pub fn tick_flash(&mut self, ttl_secs: u64) -> bool {
        self.flash.retain(|_, t| t.elapsed().as_secs() < ttl_secs);
        !self.flash.is_empty()
    }
}

fn status_rank(s: DealStatus) -> u8 {
    match s {
        DealStatus::Executed => 0,
        DealStatus::Booked => 1,
        DealStatus::Confirmed => 2,
        DealStatus::Settled => 3,
        DealStatus::Cancelled => 4,
    }
}
