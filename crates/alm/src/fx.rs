//! Market risk of the FX dealing book: every deal valued at a day's market, the book's P&L and its
//! split, net open positions (shorthand method), the forward ladder, counterparty exposure,
//! historical value at risk and the limits they are held to. Pure: the caller loads deals and
//! market data and passes them in.
//!
//! Valuation: a deal is two currency legs on its value date. A leg not yet settled is worth its
//! amount at the day's spot, discounted on its currency's curve to the value date; a settled leg
//! is a currency balance worth its amount at the day's spot. The deal's value is the sum, in BHD,
//! and since a deal is worth nothing to the bank before it is traded, that value is its P&L.

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};
use domain::{
    CcyPosition, CcyVar, CounterpartyExposure, FxDealValue, FxHistoryPoint, FxRiskResult, ForwardLadder, FwdLadderRow, LimitStatus, LimitUse, PnlBy, PnlSummary, RiskLimits,
    VarResult, normalise_counterparty,
};
use uuid::Uuid;

use crate::curve::Curve;
use crate::ird::basis;

/// One FX deal as the risk engine needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct FxDeal {
    pub id: Uuid,
    pub deal_ref: String,
    pub desk_code: String,
    pub trader: String,
    pub counterparty: String,
    pub forward: bool,
    /// +1 buys ccy1, -1 sells it.
    pub sign: f64,
    pub ccy1: String,
    pub ccy2: String,
    pub notional: f64,
    /// Units of ccy2 per ccy1.
    pub rate: f64,
    pub trade_date: NaiveDate,
    pub value_date: NaiveDate,
    /// Booking order within the book, for the intraday path.
    pub seq: i64,
    /// Whether a dealer booked it; deals the reconciler created from a source have none.
    pub has_dealer: bool,
}

impl FxDeal {
    pub fn pair(&self) -> String {
        format!("{}/{}", self.ccy1, self.ccy2)
    }

    /// The two legs: (currency, amount, value date).
    pub fn legs(&self) -> [(&str, f64); 2] {
        [(self.ccy1.as_str(), self.sign * self.notional), (self.ccy2.as_str(), -self.sign * self.notional * self.rate)]
    }
}

/// The market on one day: BHD per unit of each currency, and money-market curves.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FxMarket {
    pub date: NaiveDate,
    pub spots: BTreeMap<String, f64>,
    pub curves: BTreeMap<String, Curve>,
}

impl FxMarket {
    pub fn spot(&self, ccy: &str, base: &str) -> Option<f64> {
        if ccy == base { Some(1.0) } else { self.spots.get(ccy).copied() }
    }

    /// Simple money-market discount factor to a date; 1 for a date today or past, or with no curve.
    pub fn df(&self, ccy: &str, to: NaiveDate) -> f64 {
        let days = (to - self.date).num_days();
        if days <= 0 {
            return 1.0;
        }
        let Some(c) = self.curves.get(ccy) else { return 1.0 };
        let r = c.rate(days as f64 / 365.0) / 100.0;
        1.0 / (1.0 + r * days as f64 / basis(ccy))
    }

    /// BHD value of an amount of a currency due on a date.
    pub fn leg_value(&self, ccy: &str, amount: f64, on: NaiveDate, base: &str) -> Option<f64> {
        Some(amount * self.spot(ccy, base)? * self.df(ccy, on))
    }

    /// Outright rate for a value date: units of ccy2 per ccy1 under covered interest parity.
    pub fn outright(&self, ccy1: &str, ccy2: &str, on: NaiveDate, base: &str) -> Option<f64> {
        let s1 = self.spot(ccy1, base)?;
        let s2 = self.spot(ccy2, base)?;
        Some(s1 / s2 * self.df(ccy1, on) / self.df(ccy2, on))
    }
}

/// A deal's value at a market, BHD.
pub fn value(d: &FxDeal, m: &FxMarket, base: &str) -> Option<f64> {
    let mut v = 0.0;
    for (ccy, amount) in d.legs() {
        v += m.leg_value(ccy, amount, d.value_date, base)?;
    }
    Some(v)
}

fn status(pct: f64, warn_pct: f64) -> LimitStatus {
    if pct > 100.0 + 1e-9 {
        LimitStatus::Breach
    } else if pct >= warn_pct {
        LimitStatus::Watch
    } else {
        LimitStatus::Ok
    }
}

fn limit_use(key: impl Into<String>, label: impl Into<String>, value: f64, limit: f64, warn_pct: f64) -> LimitUse {
    let pct = if limit > 0.0 { value / limit * 100.0 } else { 0.0 };
    LimitUse { key: key.into(), label: label.into(), value, limit, pct, status: status(pct, warn_pct) }
}

/// Net amount per currency of every deal traded on or before `on`.
pub fn positions_at<'a>(deals: impl Iterator<Item = &'a FxDeal>, on: NaiveDate) -> BTreeMap<String, f64> {
    let mut pos: BTreeMap<String, f64> = BTreeMap::new();
    for d in deals.filter(|d| d.trade_date <= on) {
        for (ccy, amount) in d.legs() {
            *pos.entry(ccy.to_string()).or_default() += amount;
        }
    }
    pos
}

/// Shorthand aggregate of positions in BHD (the base currency left out): (longs, shorts as a
/// positive number, the larger of the two).
pub fn shorthand(pos: &BTreeMap<String, f64>, m: &FxMarket, base: &str, only: impl Fn(&str) -> bool) -> (f64, f64, f64) {
    let (mut long, mut short) = (0.0, 0.0);
    for (ccy, amount) in pos {
        if ccy == base || !only(ccy) {
            continue;
        }
        let Some(s) = m.spot(ccy, base) else { continue };
        let v = amount * s;
        if v >= 0.0 { long += v } else { short -= v }
    }
    (long, short, long.max(short))
}

/// The book's value at a market: (all deals traded by the market's day, of which unsettled).
pub fn book_value(deals: &[FxDeal], m: &FxMarket, base: &str) -> (f64, f64) {
    let (mut total, mut open) = (0.0, 0.0);
    for d in deals.iter().filter(|d| d.trade_date <= m.date) {
        if let Some(v) = value(d, m, base) {
            total += v;
            if d.value_date > m.date {
                open += v;
            }
        }
    }
    (total, open)
}

/// Value of the deals traded by `cut` at the market `m`.
fn value_traded_by(deals: &[&FxDeal], m: &FxMarket, cut: NaiveDate, base: &str) -> f64 {
    deals.iter().filter(|d| d.trade_date <= cut).filter_map(|d| value(d, m, base)).sum()
}

/// Day, month and year P&L of a set of deals at the market `today`, given the closes the period
/// starts from (the previous day's, the last day of the previous month and of the previous year).
fn period_pnl(deals: &[&FxDeal], today: &FxMarket, prev: &FxMarket, month_open: &FxMarket, year_open: &FxMarket, base: &str) -> (f64, f64, f64, f64) {
    let now = value_traded_by(deals, today, today.date, base);
    let day = now - value_traded_by(deals, prev, prev.date, base);
    let new_deals: f64 = deals.iter().filter(|d| d.trade_date == today.date).filter_map(|d| value(d, today, base)).sum();
    let mtd = now - value_traded_by(deals, month_open, month_open.date, base);
    let ytd = now - value_traded_by(deals, year_open, year_open.date, base);
    (day, new_deals, mtd, ytd)
}

/// The day before a date, the last day of the previous month, and the last day of the previous year.
pub fn period_opens(today: NaiveDate) -> (NaiveDate, NaiveDate, NaiveDate) {
    let prev = today.pred_opt().unwrap_or(today);
    let month = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).and_then(|d| d.pred_opt()).unwrap_or(prev);
    let year = NaiveDate::from_ymd_opt(today.year(), 1, 1).and_then(|d| d.pred_opt()).unwrap_or(prev);
    (prev, month, year)
}

pub const LADDER_BUCKETS: [(&str, i64); 7] = [("Spot", 3), ("1W", 7), ("1M", 31), ("3M", 92), ("6M", 183), ("1Y", 366), ("Over 1Y", i64::MAX)];

/// Net unsettled amounts per currency by time to value date.
pub fn forward_ladder(deals: &[FxDeal], m: &FxMarket, base: &str) -> ForwardLadder {
    let mut rows: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for d in deals.iter().filter(|d| d.trade_date <= m.date && d.value_date > m.date) {
        let days = (d.value_date - m.date).num_days();
        let b = LADDER_BUCKETS.iter().position(|(_, upto)| days <= *upto).unwrap_or(LADDER_BUCKETS.len() - 1);
        for (ccy, amount) in d.legs() {
            if ccy == base {
                continue;
            }
            rows.entry(ccy.to_string()).or_insert_with(|| vec![0.0; LADDER_BUCKETS.len()])[b] += amount;
        }
    }
    ForwardLadder {
        buckets: LADDER_BUCKETS.iter().map(|(l, _)| (*l).to_string()).collect(),
        rows: rows
            .into_iter()
            .map(|(ccy, amounts)| {
                let s = m.spot(&ccy, base).unwrap_or(0.0);
                let bhd = amounts.iter().map(|a| a * s).collect();
                FwdLadderRow { ccy, amounts, bhd }
            })
            .collect(),
    }
}

/// Basel's current-exposure add-on for FX contracts by remaining life.
pub fn add_on_factor(days: i64) -> f64 {
    if days <= 365 {
        0.01
    } else if days <= 5 * 365 {
        0.05
    } else {
        0.075
    }
}

/// Pre-settlement and settlement exposure per counterparty against its lines, worst first.
pub fn counterparty_exposure(deals: &[FxDeal], m: &FxMarket, limits: &RiskLimits) -> Vec<CounterpartyExposure> {
    let base = limits.base_ccy.as_str();
    struct Acc {
        name: String,
        deals: u32,
        mtm: f64,
        add_on: f64,
        pay: BTreeMap<NaiveDate, f64>,
    }
    let mut by: BTreeMap<String, Acc> = BTreeMap::new();
    for d in deals.iter().filter(|d| d.trade_date <= m.date && d.value_date >= m.date) {
        let key = normalise_counterparty(&d.counterparty);
        let a = by.entry(key.clone()).or_insert(Acc { name: key, deals: 0, mtm: 0.0, add_on: 0.0, pay: BTreeMap::new() });
        a.deals += 1;
        if d.value_date > m.date {
            a.mtm += value(d, m, base).unwrap_or(0.0);
            let notional_bhd = d.notional * m.spot(&d.ccy1, base).unwrap_or(0.0);
            a.add_on += notional_bhd * add_on_factor((d.value_date - m.date).num_days());
        }
        // What the bank pays away on the value date before the other leg arrives.
        for (ccy, amount) in d.legs() {
            if amount < 0.0 {
                *a.pay.entry(d.value_date).or_default() += -amount * m.spot(ccy, base).unwrap_or(0.0);
            }
        }
    }
    let mut out: Vec<CounterpartyExposure> = by
        .into_values()
        .map(|a| {
            let line = limits.line(&a.name);
            let pse = a.mtm.max(0.0) + a.add_on;
            let (peak_date, peak) = a.pay.iter().max_by(|x, y| x.1.partial_cmp(y.1).unwrap_or(std::cmp::Ordering::Equal)).map(|(d, v)| (Some(*d), *v)).unwrap_or((None, 0.0));
            let pse_pct = if line.pre_settlement > 0.0 { pse / line.pre_settlement * 100.0 } else { 0.0 };
            let set_pct = if line.settlement > 0.0 { peak / line.settlement * 100.0 } else { 0.0 };
            CounterpartyExposure {
                name: a.name,
                deals: a.deals,
                mtm: a.mtm,
                add_on: a.add_on,
                pre_settlement: pse,
                pre_settlement_line: line.pre_settlement,
                pre_settlement_pct: pse_pct,
                settlement_peak: peak,
                settlement_peak_date: peak_date,
                settlement_line: line.settlement,
                settlement_pct: set_pct,
                status: status(pse_pct.max(set_pct), limits.warn_pct),
            }
        })
        .collect();
    out.sort_by(|a, b| b.pre_settlement_pct.max(b.settlement_pct).partial_cmp(&a.pre_settlement_pct.max(a.settlement_pct)).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// Spot history: BHD per unit of each currency per day, ascending by day.
pub type SpotHistory = BTreeMap<NaiveDate, BTreeMap<String, f64>>;

/// One-day returns of each currency against BHD over the `n` days ending `on`.
fn returns(history: &SpotHistory, on: NaiveDate, n: usize) -> Vec<BTreeMap<String, f64>> {
    let days: Vec<(&NaiveDate, &BTreeMap<String, f64>)> = history.range(..=on).collect();
    let mut out = Vec::new();
    for w in days.windows(2).rev().take(n) {
        let (prev, cur) = (w[0].1, w[1].1);
        out.push(cur.iter().filter_map(|(c, v)| prev.get(c).filter(|p| **p > 0.0).map(|p| (c.clone(), v / p - 1.0))).collect());
    }
    out
}

/// The loss at the 99th percentile and the average loss beyond it, of a set of scenario P&Ls.
pub fn var_es(mut pnl: Vec<f64>) -> (f64, f64) {
    if pnl.is_empty() {
        return (0.0, 0.0);
    }
    pnl.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let k = ((pnl.len() as f64) * 0.01).floor() as usize;
    let k = k.min(pnl.len() - 1);
    let var = (-pnl[k]).max(0.0);
    let tail = &pnl[..=k];
    let es = (-(tail.iter().sum::<f64>() / tail.len() as f64)).max(0.0);
    (var, es)
}

pub const VAR_WINDOW: usize = 250;

/// Historical value at risk of BHD positions per currency over the window ending `on`.
pub fn var_of(positions_bhd: &BTreeMap<String, f64>, history: &SpotHistory, on: NaiveDate) -> (f64, f64, u32) {
    let rs = returns(history, on, VAR_WINDOW);
    let pnl: Vec<f64> = rs.iter().map(|r| positions_bhd.iter().map(|(c, p)| p * r.get(c).copied().unwrap_or(0.0)).sum()).collect();
    let (v, es) = var_es(pnl);
    (v, es, rs.len() as u32)
}

pub fn value_at_risk(positions_bhd: &BTreeMap<String, f64>, history: &SpotHistory, on: NaiveDate) -> VarResult {
    let (var99, es99, observations) = var_of(positions_bhd, history, on);
    // Stressed: today's positions over every full window the history holds; the worst one.
    let mut stressed = (var99, Some(on));
    let days: Vec<NaiveDate> = history.range(..=on).map(|(d, _)| *d).collect();
    for end in days.iter().skip(VAR_WINDOW) {
        let (v, _, n) = var_of(positions_bhd, history, *end);
        if n as usize >= VAR_WINDOW && v > stressed.0 {
            stressed = (v, Some(*end));
        }
    }
    let by_ccy = positions_bhd
        .iter()
        .filter(|(_, p)| p.abs() > 0.5)
        .map(|(c, p)| {
            let one: BTreeMap<String, f64> = [(c.clone(), *p)].into_iter().collect();
            CcyVar { ccy: c.clone(), position: *p, var99: var_of(&one, history, on).0 }
        })
        .collect();
    VarResult { var99, es99, stressed_var99: stressed.0, stressed_window_end: stressed.1, observations, by_ccy }
}

/// Distance of a deal's rate from the mid for its value date on its trade date, basis points.
pub fn off_market_bp(d: &FxDeal, trade_market: &FxMarket, base: &str) -> Option<(f64, f64)> {
    let mid = trade_market.outright(&d.ccy1, &d.ccy2, d.value_date, base)?;
    Some((mid, (d.rate / mid - 1.0) * 10_000.0))
}

/// Everything the risk page shows for one day.
pub struct RiskInputs<'a> {
    pub deals: &'a [FxDeal],
    /// The valuation day's market and the closes the day, month and year start from.
    pub today: &'a FxMarket,
    pub prev: &'a FxMarket,
    pub month_open: &'a FxMarket,
    pub year_open: &'a FxMarket,
    /// The market on each trade date, for the off-market check and the day-one value.
    pub trade_markets: &'a BTreeMap<NaiveDate, FxMarket>,
    pub history: &'a SpotHistory,
    pub limits: &'a RiskLimits,
    pub limits_version: Option<i32>,
    pub cancelled_30d: u32,
}

pub fn risk(inp: &RiskInputs) -> FxRiskResult {
    let limits = inp.limits;
    let base = limits.base_ccy.as_str();
    let m = inp.today;
    let deals: Vec<&FxDeal> = inp.deals.iter().filter(|d| d.trade_date <= m.date).collect();

    // Positions.
    let pos = positions_at(deals.iter().copied(), m.date);
    let (long, short, total) = shorthand(&pos, m, base, |_| true);
    let (_, _, floating) = shorthand(&pos, m, base, |c| !limits.is_pegged(c));
    let (_, _, pegged) = shorthand(&pos, m, base, |c| limits.is_pegged(c));
    let mut unpriced = Vec::new();
    let positions: Vec<CcyPosition> = pos
        .iter()
        .filter(|(c, a)| c.as_str() != base && a.abs() > 0.005)
        .map(|(ccy, amount)| {
            let spot = m.spot(ccy, base);
            if spot.is_none() {
                unpriced.push(ccy.clone());
            }
            let bhd = spot.map(|s| amount * s);
            let limit = limits.nop_limit(ccy);
            let pct = bhd.map(|b| if limit > 0.0 { b.abs() / limit * 100.0 } else { 0.0 }).unwrap_or(0.0);
            CcyPosition { ccy: ccy.clone(), amount: *amount, bhd, pegged: limits.is_pegged(ccy), spot, limit, pct, status: status(pct, limits.warn_pct) }
        })
        .collect();
    let positions_bhd: BTreeMap<String, f64> = positions.iter().filter_map(|p| p.bhd.map(|b| (p.ccy.clone(), b))).collect();

    // The intraday path: yesterday's close, then the day's deals in booking order.
    let mut path = positions_at(deals.iter().copied(), inp.prev.date);
    let mut today_deals: Vec<&FxDeal> = deals.iter().copied().filter(|d| d.trade_date == m.date).collect();
    today_deals.sort_by_key(|d| d.seq);
    let mut peak = shorthand(&path, m, base, |_| true).2;
    for d in today_deals {
        for (ccy, amount) in d.legs() {
            *path.entry(ccy.to_string()).or_default() += amount;
        }
        peak = peak.max(shorthand(&path, m, base, |_| true).2);
    }

    // P&L.
    let (day, new_deals, mtd, ytd) = period_pnl(&deals, m, inp.prev, inp.month_open, inp.year_open, base);
    let (book, open) = book_value(inp.deals, m, base);
    let pnl = PnlSummary { today: day, new_deals, market_move: day - new_deals, mtd, ytd, total: book, open_mtm: open, settled: book - open };
    let group = |key: &dyn Fn(&FxDeal) -> String| -> Vec<PnlBy> {
        let mut keys: BTreeMap<String, Vec<&FxDeal>> = BTreeMap::new();
        for d in &deals {
            keys.entry(key(d)).or_default().push(d);
        }
        let mut out: Vec<PnlBy> = keys
            .into_iter()
            .map(|(k, ds)| {
                let (today, _, mtd, ytd) = period_pnl(&ds, m, inp.prev, inp.month_open, inp.year_open, base);
                let p = positions_at(ds.iter().copied(), m.date);
                PnlBy { key: k, today, mtd, ytd, deals: ds.len() as u32, nop: shorthand(&p, m, base, |_| true).2 }
            })
            .collect();
        out.sort_by(|a, b| b.mtd.partial_cmp(&a.mtd).unwrap_or(std::cmp::Ordering::Equal));
        out
    };
    let by_trader = group(&|d: &FxDeal| d.trader.clone());
    let by_pair = group(&|d: &FxDeal| d.pair());

    let ladder = forward_ladder(inp.deals, m, base);
    let counterparties = counterparty_exposure(inp.deals, m, limits);
    let var = value_at_risk(&positions_bhd, inp.history, m.date);

    // Off-market deals: every deal checked against the mid for its value date on its trade date.
    let mut off_market: Vec<FxDealValue> = deals
        .iter()
        .filter_map(|d| {
            let tm = inp.trade_markets.get(&d.trade_date)?;
            let (mid, bp) = off_market_bp(d, tm, base)?;
            let tolerance = if d.forward { limits.off_market_bp_forward } else { limits.off_market_bp_spot };
            (bp.abs() > tolerance).then(|| deal_value(d, m, Some(tm), base, Some((mid, bp)), true))
        })
        .collect();
    off_market.sort_by(|a, b| b.off_market_bp.unwrap_or(0.0).abs().partial_cmp(&a.off_market_bp.unwrap_or(0.0).abs()).unwrap_or(std::cmp::Ordering::Equal));

    // Limits.
    let w = limits.warn_pct;
    let mut uses = vec![
        limit_use("nop.total", "Net open position, overnight", total, limits.nop_overnight_total, w),
        limit_use("nop.intraday", "Net open position, intraday peak", peak, limits.nop_intraday_total, w),
    ];
    for p in &positions {
        if let Some(b) = p.bhd {
            uses.push(limit_use(format!("nop.ccy.{}", p.ccy), format!("{} open position", p.ccy), b.abs(), p.limit, w));
        }
    }
    let dealers: std::collections::BTreeSet<&str> = deals.iter().filter(|d| d.has_dealer).map(|d| d.trader.as_str()).collect();
    for t in by_trader.iter().filter(|t| dealers.contains(t.key.as_str())) {
        uses.push(limit_use(format!("nop.dealer.{}", t.key), format!("{} open position", t.key), t.nop, limits.dealer_nop, w));
    }
    uses.push(limit_use("stop.daily", "Stop-loss, today", (-day).max(0.0), limits.stop_loss_daily, w));
    uses.push(limit_use("stop.monthly", "Stop-loss, month to date", (-mtd).max(0.0), limits.stop_loss_monthly, w));
    uses.push(limit_use("var", "Value at risk, 1 day 99%", var.var99, limits.var_limit, w));
    for c in &counterparties {
        uses.push(limit_use(format!("cpty.pse.{}", c.name), format!("{} pre-settlement", c.name), c.pre_settlement, c.pre_settlement_line, w));
        uses.push(limit_use(format!("cpty.set.{}", c.name), format!("{} settlement", c.name), c.settlement_peak, c.settlement_line, w));
    }

    FxRiskResult {
        as_of: m.date,
        base_ccy: base.to_string(),
        limits_version: inp.limits_version,
        positions,
        nop_long: long,
        nop_short: short,
        nop_total: total,
        nop_floating: floating,
        nop_pegged: pegged,
        nop_intraday_peak: peak,
        ladder,
        pnl,
        by_trader,
        by_pair,
        counterparties,
        var,
        limits: uses,
        off_market,
        unpriced,
        cancelled_30d: inp.cancelled_30d,
    }
}

/// One deal valued at a market, with its trade-date check if known.
pub fn deal_value(d: &FxDeal, m: &FxMarket, trade_market: Option<&FxMarket>, base: &str, check: Option<(f64, f64)>, flagged: bool) -> FxDealValue {
    FxDealValue {
        deal_id: d.id,
        deal_ref: d.deal_ref.clone(),
        desk_code: d.desk_code.clone(),
        trader: d.trader.clone(),
        counterparty: d.counterparty.clone(),
        pair: d.pair(),
        side: if d.sign > 0.0 { "BUY".into() } else { "SELL".into() },
        notional: d.notional,
        rate: d.rate,
        trade_date: d.trade_date,
        value_date: d.value_date,
        market_rate: m.outright(&d.ccy1, &d.ccy2, d.value_date.max(m.date), base),
        mtm: value(d, m, base),
        settled: d.value_date <= m.date,
        trade_mid: check.map(|c| c.0),
        off_market_bp: check.map(|c| c.1),
        off_market: flagged,
        day_one: trade_market.and_then(|tm| value(d, tm, base)),
    }
}

/// The book's figures on each day of a range: day and total P&L, the aggregate and floating open
/// positions, and VaR. `markets` must hold every day of the range and the day before it.
pub fn history(deals: &[FxDeal], markets: &BTreeMap<NaiveDate, FxMarket>, spots: &SpotHistory, limits: &RiskLimits, days: &[NaiveDate]) -> Vec<FxHistoryPoint> {
    let base = limits.base_ccy.as_str();
    let all: Vec<&FxDeal> = deals.iter().collect();
    days.iter()
        .filter_map(|day| {
            let m = markets.get(day)?;
            let prev = markets.range(..*day).next_back().map(|(_, p)| p)?;
            let total = value_traded_by(&all, m, *day, base);
            let pnl_day = total - value_traded_by(&all, prev, prev.date, base);
            let pos = positions_at(deals.iter(), *day);
            let (_, _, nop_total) = shorthand(&pos, m, base, |_| true);
            let (_, _, nop_floating) = shorthand(&pos, m, base, |c| !limits.is_pegged(c));
            let bhd: BTreeMap<String, f64> = pos.iter().filter(|(c, _)| c.as_str() != base).filter_map(|(c, a)| m.spot(c, base).map(|s| (c.clone(), a * s))).collect();
            let (var99, _, _) = var_of(&bhd, spots, *day);
            Some(FxHistoryPoint { date: *day, pnl_day, pnl_total: total, nop_total, nop_floating, var99 })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn market(on: NaiveDate, eur: f64) -> FxMarket {
        FxMarket { date: on, spots: [("USD".to_string(), 0.376), ("EUR".to_string(), eur)].into_iter().collect(), curves: BTreeMap::new() }
    }

    fn deal(n: i64, sign: f64, c1: &str, c2: &str, notional: f64, rate: f64, trade: NaiveDate, value: NaiveDate) -> FxDeal {
        FxDeal {
            id: Uuid::from_u128(n as u128),
            deal_ref: format!("FX-{n:06}"),
            desk_code: "FX".into(),
            trader: "fx1".into(),
            counterparty: "Citibank N.A.".into(),
            forward: value > trade + chrono::Duration::days(3),
            sign,
            ccy1: c1.into(),
            ccy2: c2.into(),
            notional,
            rate,
            trade_date: trade,
            value_date: value,
            seq: n,
            has_dealer: true,
        }
    }

    #[test]
    fn a_bought_euro_gains_when_the_euro_rises() {
        // Buy 1m EUR at 0.4080 BHD; the euro moves to 0.4100: +2,000 BHD, no discounting with no curve.
        let d = deal(1, 1.0, "EUR", "BHD", 1_000_000.0, 0.408, day(2026, 11, 2), day(2026, 11, 4));
        let v = value(&d, &market(day(2026, 11, 3), 0.410), "BHD").unwrap();
        assert!((v - 2_000.0).abs() < 1e-6, "{v}");
        // A cross: buy 1m EUR against USD at 1.0851; EUR/USD at 0.410/0.376 = 1.090426 gives 5,326.6 USD, 2,002.8 BHD.
        let x = deal(2, 1.0, "EUR", "USD", 1_000_000.0, 1.0851, day(2026, 11, 2), day(2026, 11, 4));
        let v = value(&x, &market(day(2026, 11, 3), 0.410), "BHD").unwrap();
        assert!((v - (0.410 - 1.0851 * 0.376) * 1_000_000.0).abs() < 1e-6, "{v}");
    }

    #[test]
    fn discounting_and_the_outright_follow_covered_interest_parity() {
        let on = day(2026, 11, 4);
        let mut m = market(on, 0.408);
        m.curves.insert("EUR".into(), Curve::flat("EUR", 2.0));
        m.curves.insert("BHD".into(), Curve::flat("BHD", 6.0));
        let vd = on + chrono::Duration::days(90);
        let f = m.outright("EUR", "BHD", vd, "BHD").unwrap();
        let expect = 0.408 * (1.0 + 0.06 * 90.0 / 360.0) / (1.0 + 0.02 * 90.0 / 360.0);
        assert!((f - expect).abs() < 1e-12, "{f} {expect}");
        // A forward done at the outright is worth nothing.
        let d = deal(3, 1.0, "EUR", "BHD", 1_000_000.0, f, on, vd);
        assert!(value(&d, &m, "BHD").unwrap().abs() < 1e-6);
    }

    #[test]
    fn shorthand_takes_the_larger_side_and_ignores_the_base() {
        let m = market(day(2026, 11, 4), 0.4);
        let pos: BTreeMap<String, f64> = [("EUR".to_string(), 1_000_000.0), ("USD".to_string(), -2_000_000.0), ("BHD".to_string(), 9e9)].into_iter().collect();
        let (long, short, total) = shorthand(&pos, &m, "BHD", |_| true);
        assert!((long - 400_000.0).abs() < 1e-6 && (short - 752_000.0).abs() < 1e-6 && (total - 752_000.0).abs() < 1e-6);
    }

    #[test]
    fn pnl_splits_into_new_deals_and_market_move() {
        let d1 = day(2026, 11, 2);
        let d2 = day(2026, 11, 3);
        let deals = vec![
            deal(1, 1.0, "EUR", "BHD", 1_000_000.0, 0.408, d1, day(2026, 11, 4)),
            // Today, bought 0.0005 under the mid: +500 BHD on day one.
            deal(2, 1.0, "EUR", "BHD", 1_000_000.0, 0.4095, d2, day(2026, 11, 5)),
        ];
        let (m1, m2) = (market(d1, 0.408), market(d2, 0.410));
        let refs: Vec<&FxDeal> = deals.iter().collect();
        let (dayp, new_deals, _, _) = period_pnl(&refs, &m2, &m1, &m1, &m1, "BHD");
        assert!((new_deals - 500.0).abs() < 1e-6, "{new_deals}");
        // The first deal moved from 0 to +2,000 overnight.
        assert!((dayp - 2_500.0).abs() < 1e-6, "{dayp}");
    }

    #[test]
    fn var_takes_the_third_worst_of_250_and_es_averages_the_tail() {
        let pnl: Vec<f64> = (0..250).map(|i| i as f64 - 10.0).collect();
        let (var, es) = var_es(pnl);
        assert_eq!(var, 8.0);
        assert!((es - 9.0).abs() < 1e-12);
    }

    #[test]
    fn pegged_positions_carry_no_var() {
        let mut h: SpotHistory = BTreeMap::new();
        let start = day(2025, 1, 1);
        for i in 0..300 {
            let eur = 0.408 * (1.0 + 0.003 * ((i as f64) * 1.7).sin());
            h.insert(start + chrono::Duration::days(i), [("USD".to_string(), 0.376), ("EUR".to_string(), eur)].into_iter().collect());
        }
        let on = start + chrono::Duration::days(299);
        let usd: BTreeMap<String, f64> = [("USD".to_string(), 10_000_000.0)].into_iter().collect();
        assert_eq!(var_of(&usd, &h, on).0, 0.0);
        let eur: BTreeMap<String, f64> = [("EUR".to_string(), 10_000_000.0)].into_iter().collect();
        let r = value_at_risk(&eur, &h, on);
        assert!(r.var99 > 0.0 && r.stressed_var99 >= r.var99 && r.observations == 250);
    }

    #[test]
    fn counterparty_exposure_adds_the_add_on_and_finds_the_settlement_peak() {
        let on = day(2026, 11, 4);
        let limits = RiskLimits::default();
        let deals = vec![
            deal(1, 1.0, "EUR", "BHD", 1_000_000.0, 0.400, day(2026, 11, 3), day(2026, 11, 6)),
            deal(2, -1.0, "USD", "BHD", 2_000_000.0, 0.376, day(2026, 11, 3), day(2026, 11, 6)),
        ];
        let x = counterparty_exposure(&deals, &market(on, 0.408), &limits);
        assert_eq!(x.len(), 1);
        let c = &x[0];
        assert_eq!(c.name, "CITIBANK N.A.");
        // MTM +8,000 on the euro deal, 0 on the dollar; add-on 1% of 408,000 + 1% of 752,000.
        assert!((c.mtm - 8_000.0).abs() < 1e-6, "{}", c.mtm);
        assert!((c.pre_settlement - (8_000.0 + 4_080.0 + 7_520.0)).abs() < 1e-6, "{}", c.pre_settlement);
        // On the 6th the bank pays 400,000 BHD for the euros and 2m USD (752,000 BHD).
        assert!((c.settlement_peak - 1_152_000.0).abs() < 1e-6, "{}", c.settlement_peak);
        assert_eq!(c.settlement_peak_date, Some(day(2026, 11, 6)));
    }

    #[test]
    fn an_off_market_rate_is_measured_against_the_trade_date_mid() {
        let d = deal(1, 1.0, "EUR", "BHD", 1_000_000.0, 0.4121, day(2026, 11, 2), day(2026, 11, 4));
        let (mid, bp) = off_market_bp(&d, &market(day(2026, 11, 2), 0.408), "BHD").unwrap();
        assert!((mid - 0.408).abs() < 1e-12);
        assert!((bp - 100.49).abs() < 0.01, "{bp}");
    }

    #[test]
    fn the_forward_ladder_buckets_unsettled_legs() {
        let on = day(2026, 11, 4);
        let deals = vec![
            deal(1, 1.0, "EUR", "USD", 1_000_000.0, 1.085, day(2026, 11, 3), day(2026, 11, 6)),
            deal(2, -1.0, "EUR", "USD", 500_000.0, 1.09, day(2026, 11, 3), day(2026, 12, 20)),
            deal(3, 1.0, "EUR", "USD", 700_000.0, 1.08, day(2026, 10, 1), day(2026, 10, 3)),
        ];
        let l = forward_ladder(&deals, &market(on, 0.408), "BHD");
        let eur = l.rows.iter().find(|r| r.ccy == "EUR").unwrap();
        assert_eq!(eur.amounts[0], 1_000_000.0);
        assert_eq!(eur.amounts[3], -500_000.0);
        assert_eq!(eur.amounts.iter().sum::<f64>(), 500_000.0);
    }
}
