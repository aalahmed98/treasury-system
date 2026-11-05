//! The figures the PMU pages show, computed from the portfolio and the market on a date.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use alm::schedule::add_months;
use chrono::{Datelike, Duration, NaiveDate};
use domain::{
    Accounting, Bond, BondRateType, BookFigure, BookYear, CcyHedge, CollateralBook, DayIncome, Dv01Bucket, HedgeBucket, HedgePair, Headline, Income, Lot,
    MonthFlow, MonthIncome, NamedAmount, NamedIncome, PeriodIncome, PmuBook, PositionRow, QuoteView, Repo, RepoQuote, RepoSummary, RepoView, RowLevel,
    ShockRow, Swap, SwapView, Warning, YieldRow,
};
use uuid::Uuid;

use crate::bond::{self, clean_from_yield, dirty_from_yield, mod_duration, schedule};
use crate::ledger::{LotDay, LotState, float_rate_on, swap_notional, swap_payments, swap_pv, walk_lot, walk_repo, walk_swap};
use crate::market::{Market, curve_rate};

/// The parallel shifts of the rate-shock table, in basis points.
pub const SHOCKS_BP: [f64; 6] = [-200.0, -100.0, -50.0, 50.0, 100.0, 200.0];

/// Remaining-maturity buckets for maturity profiles and DV01, as (label, up to years).
pub const MATURITY_BUCKETS: [(&str, f64); 7] = [("<1Y", 1.0), ("1-2Y", 2.0), ("2-3Y", 3.0), ("3-5Y", 5.0), ("5-7Y", 7.0), ("7-10Y", 10.0), ("10Y+", f64::INFINITY)];

/// Remaining-maturity buckets for repos, as (label, up to days).
pub const REPO_BUCKETS: [(&str, i64); 6] = [("7 days", 7), ("8-30 days", 30), ("1-3 months", 91), ("3-6 months", 182), ("6-12 months", 365), ("Over 1 year", i64::MAX)];

#[derive(Debug, Clone, Default)]
pub struct Portfolio {
    pub books: Vec<PmuBook>,
    pub bonds: BTreeMap<String, Bond>,
    pub lots: Vec<Lot>,
    pub swaps: Vec<Swap>,
    pub repos: Vec<Repo>,
    pub quotes: Vec<RepoQuote>,
}

fn bucket_of(years: f64) -> usize {
    MATURITY_BUCKETS.iter().position(|(_, upto)| years < *upto).unwrap_or(MATURITY_BUCKETS.len() - 1)
}

pub fn month_start(d: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap_or(d)
}

pub fn year_start(d: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(d.year(), 1, 1).unwrap_or(d)
}

pub fn year_end(d: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(d.year(), 12, 31).unwrap_or(d)
}

fn wavg(pairs: impl IntoIterator<Item = (f64, f64)>) -> f64 {
    let (mut num, mut den) = (0.0, 0.0);
    for (v, w) in pairs {
        num += v * w;
        den += w;
    }
    if den.abs() < 1e-12 { 0.0 } else { num / den }
}

// ---------------------------------------------------------------------------------------------
// One lot and one swap on a date
// ---------------------------------------------------------------------------------------------

/// A lot held on a date, valued.
#[derive(Debug, Clone)]
pub struct LotNow {
    pub idx: usize,
    pub state: LotState,
    pub today: LotDay,
    pub spot: f64,
    pub face_bhd: f64,
    pub market_price: f64,
    pub ytm: f64,
    pub accrued_bhd: f64,
    pub mv_bhd: f64,
    pub bv_bhd: f64,
    pub mtm_bhd: f64,
    pub dv01_bhd: f64,
    pub mod_duration: f64,
    /// Local currency.
    pub hedged_face: f64,
    pub pledged_face: f64,
    /// For floaters: years to the next reset.
    reset_years: f64,
}

impl LotNow {
    /// P&L in BHD of the bond (not its hedges) for a parallel move of `bp`.
    pub fn shock_bhd(&self, b: &Bond, sched: &[NaiveDate], on: NaiveDate, bp: f64, face: f64) -> f64 {
        match b.rate_type {
            BondRateType::Fixed => (dirty_from_yield(b, sched, on, self.ytm + bp / 100.0) - dirty_from_yield(b, sched, on, self.ytm)) * face / 100.0 * self.spot,
            BondRateType::Float => -face * self.reset_years * bp / 10_000.0 * self.spot,
        }
    }
}

pub fn held_on(lot: &Lot, b: &Bond, on: NaiveDate) -> bool {
    lot.settle_date <= on && lot.settle_date >= b.issue_date && b.maturity > on && lot.status != "SOLD"
}

/// Local face of a lot hedged on `on`: each linked swap's assigned notional, scaled by how much
/// of the swap is still live, capped at the face.
pub fn hedged_face(p: &Portfolio, lot: &Lot, on: NaiveDate) -> f64 {
    let h: f64 = p
        .swaps
        .iter()
        .filter(|s| s.start <= on && s.maturity > on)
        .flat_map(|s| {
            let live = if s.notional > 0.0 { swap_notional(s, on + Duration::days(1)) / s.notional } else { 0.0 };
            s.hedges.iter().filter(|(l, _)| *l == lot.id).map(move |(_, n)| n * live)
        })
        .sum();
    h.min(lot.face)
}

pub fn pledged_face(p: &Portfolio, lot: &Lot, on: NaiveDate) -> f64 {
    let x: f64 = p.repos.iter().filter(|r| r.start <= on && r.end > on).flat_map(|r| r.collateral.iter().filter(|(l, _)| *l == lot.id).map(|(_, f)| *f)).sum();
    x.min(lot.face)
}

pub fn lot_now(m: &Market, p: &Portfolio, idx: usize, on: NaiveDate) -> Option<LotNow> {
    let lot = &p.lots[idx];
    let b = p.bonds.get(&lot.isin)?;
    if !held_on(lot, b, on) {
        return None;
    }
    let mut today = LotDay::default();
    let state = walk_lot(m, lot, b, on, on, |d| today = *d);
    let sched = schedule(b);
    let spot = m.spot(&b.ccy, on);
    let years = (b.maturity - on).num_days() as f64 / 365.0;
    let (market_price, ytm, mod_dur, dv01, reset_years) = match b.rate_type {
        BondRateType::Fixed => {
            let (clean, ytm) = match m.price(&b.isin, on) {
                Some((c, y)) => (c, y),
                None => {
                    let y = curve_rate(&m.curve(&b.ccy, 0.0), (b.maturity - on).num_days()) + 0.6;
                    (clean_from_yield(b, &sched, on, y), y)
                }
            };
            let dv01 = (dirty_from_yield(b, &sched, on, ytm + 0.01) - dirty_from_yield(b, &sched, on, ytm)) * lot.face / 100.0 * spot;
            (clean, ytm, mod_duration(b, &sched, on, ytm), dv01, 0.0)
        }
        BondRateType::Float => {
            let next = state.period.map(|(_, n)| n).unwrap_or(b.maturity);
            let tau = (next - on).num_days().max(0) as f64 / 365.0;
            let clean = m.price(&b.isin, on).map(|x| x.0).unwrap_or(100.0);
            (clean, state.coupon_rate, tau, -lot.face * tau / 10_000.0 * spot, tau)
        }
    };
    let _ = years;
    let accrued_bhd = state.accrued * spot;
    let mv_bhd = market_price * lot.face / 100.0 * spot + accrued_bhd;
    let bv_bhd = state.book_value * spot;
    Some(LotNow {
        idx,
        state,
        today,
        spot,
        face_bhd: lot.face * spot,
        market_price,
        ytm,
        accrued_bhd,
        mv_bhd,
        bv_bhd,
        mtm_bhd: mv_bhd - accrued_bhd - bv_bhd,
        dv01_bhd: dv01,
        mod_duration: mod_dur,
        hedged_face: hedged_face(p, lot, on),
        pledged_face: pledged_face(p, lot, on),
        reset_years,
    })
}

#[derive(Debug, Clone)]
pub struct SwapNow {
    pub idx: usize,
    pub notional: f64,
    pub closed: f64,
    pub float_rate: f64,
    pub spot: f64,
    pub mtm_bhd: f64,
    pub dv01_bhd: f64,
}

pub fn swap_live(s: &Swap, on: NaiveDate) -> bool {
    s.start <= on && s.maturity > on && swap_notional(s, on + Duration::days(1)) > 0.0
}

pub fn swap_now(m: &Market, p: &Portfolio, idx: usize, on: NaiveDate) -> Option<SwapNow> {
    let s = &p.swaps[idx];
    if !swap_live(s, on) {
        return None;
    }
    let spot = m.spot(&s.ccy, on);
    let pv = swap_pv(m, s, on, 0.0);
    Some(SwapNow {
        idx,
        notional: swap_notional(s, on + Duration::days(1)),
        closed: s.unwinds.iter().filter(|u| u.date <= on).map(|u| u.notional).sum(),
        float_rate: float_rate_on(m, s, on + Duration::days(1)),
        spot,
        mtm_bhd: pv * spot,
        dv01_bhd: (swap_pv(m, s, on, 1.0) - pv) * spot,
    })
}

/// How a swap's figures are shared among the lots it hedges: by assigned notional.
fn shares(s: &Swap) -> Vec<(Uuid, f64)> {
    let total: f64 = s.hedges.iter().map(|(_, n)| n).sum();
    if total <= 0.0 { Vec::new() } else { s.hedges.iter().map(|(l, n)| (*l, n / total)).collect() }
}

fn repo_shares(r: &Repo) -> Vec<(Uuid, f64)> {
    let total: f64 = r.collateral.iter().map(|(_, f)| f).sum();
    if total <= 0.0 { Vec::new() } else { r.collateral.iter().map(|(l, f)| (*l, f / total)).collect() }
}

// ---------------------------------------------------------------------------------------------
// Income over a period
// ---------------------------------------------------------------------------------------------

/// Swap carry or repo interest that has no lot to sit on, kept by book.
pub const UNLINKED: &str = "unlinked";

#[derive(Debug, Clone, Default)]
pub struct IncomeRun {
    pub total: Income,
    pub daily: BTreeMap<NaiveDate, Income>,
    pub by_lot: HashMap<Uuid, Income>,
    /// Swap carry not linked to a lot, by the swap's book; repo interest with no collateral.
    pub unlinked: BTreeMap<String, Income>,
}

impl IncomeRun {
    fn put(&mut self, day: NaiveDate, lot: Option<Uuid>, book: &str, f: impl Fn(&mut Income)) {
        f(self.daily.entry(day).or_default());
        match lot {
            Some(id) => f(self.by_lot.entry(id).or_default()),
            None => f(self.unlinked.entry(book.to_string()).or_default()),
        }
        f(&mut self.total);
    }

    fn finish(mut self) -> IncomeRun {
        self.total = self.total.finish();
        for v in self.daily.values_mut() {
            *v = v.finish();
        }
        for v in self.by_lot.values_mut() {
            *v = v.finish();
        }
        for v in self.unlinked.values_mut() {
            *v = v.finish();
        }
        self
    }

    /// Income per book: its lots plus what is unlinked in it.
    pub fn by_book(&self, p: &Portfolio) -> BTreeMap<String, Income> {
        let mut out: BTreeMap<String, Income> = BTreeMap::new();
        for l in &p.lots {
            if let Some(i) = self.by_lot.get(&l.id) {
                out.entry(l.book.clone()).or_default().add(i);
            }
        }
        for (k, i) in &self.unlinked {
            out.entry(k.clone()).or_default().add(i);
        }
        out
    }
}

/// Income of every lot, swap and repo from `from` to `to` (inclusive), in BHD at each day's spot.
pub fn income(m: &Market, p: &Portfolio, from: NaiveDate, to: NaiveDate) -> IncomeRun {
    let mut run = IncomeRun::default();
    if to < from {
        return run;
    }
    for lot in &p.lots {
        let Some(b) = p.bonds.get(&lot.isin) else { continue };
        if lot.status == "SOLD" || lot.settle_date >= to || lot.settle_date < b.issue_date || b.maturity < from {
            continue;
        }
        walk_lot(m, lot, b, from, to, |d| {
            let fx = m.spot(&b.ccy, d.date);
            run.put(d.date, Some(lot.id), &lot.book, |i| {
                i.coupon += d.coupon * fx;
                i.amortisation += d.amortisation * fx;
                i.ftp += d.ftp * fx;
            });
        });
    }
    for s in &p.swaps {
        if s.maturity < from || s.start > to {
            continue;
        }
        let sh = shares(s);
        walk_swap(m, s, from, to, |d, carry| {
            let v = carry * m.spot(&s.ccy, d);
            if sh.is_empty() {
                run.put(d, None, &s.book, |i| i.irs += v);
            } else {
                for (l, w) in &sh {
                    run.put(d, Some(*l), &s.book, |i| i.irs += v * w);
                }
            }
        });
    }
    for r in &p.repos {
        if r.end < from || r.start > to {
            continue;
        }
        let sh = repo_shares(r);
        walk_repo(r, from, to, |d, cost| {
            let v = cost * m.spot(&r.ccy, d);
            if sh.is_empty() {
                run.put(d, None, UNLINKED, |i| i.repo += v);
            } else {
                for (l, w) in &sh {
                    run.put(d, Some(*l), UNLINKED, |i| i.repo += v * w);
                }
            }
        });
    }
    run.finish()
}

pub fn daily_series(run: &IncomeRun, from: NaiveDate, to: NaiveDate) -> Vec<DayIncome> {
    let mut out = Vec::new();
    let mut d = from;
    while d <= to {
        out.push(DayIncome { date: d, income: run.daily.get(&d).copied().unwrap_or_default() });
        d += Duration::days(1);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// The state on a date
// ---------------------------------------------------------------------------------------------

/// Everything valued on one date: the held lots, the live swaps, and the figures built from them.
pub struct Snapshot<'a> {
    pub m: &'a Market,
    pub p: &'a Portfolio,
    pub on: NaiveDate,
    pub lots: Vec<LotNow>,
    pub swaps: Vec<SwapNow>,
}

impl<'a> Snapshot<'a> {
    pub fn new(m: &'a Market, p: &'a Portfolio, on: NaiveDate) -> Snapshot<'a> {
        let lots = (0..p.lots.len()).filter_map(|i| lot_now(m, p, i, on)).collect();
        let swaps = (0..p.swaps.len()).filter_map(|i| swap_now(m, p, i, on)).collect();
        Snapshot { m, p, on, lots, swaps }
    }

    fn lot(&self, n: &LotNow) -> &Lot {
        &self.p.lots[n.idx]
    }

    fn bond(&self, n: &LotNow) -> &Bond {
        &self.p.bonds[&self.p.lots[n.idx].isin]
    }

    /// Swap MTM and DV01 shared onto the lots they hedge, plus what sits on no held lot, by book.
    #[allow(clippy::type_complexity)]
    pub fn swap_alloc(&self) -> (HashMap<Uuid, (f64, f64)>, BTreeMap<String, (f64, f64)>) {
        let held: BTreeSet<Uuid> = self.lots.iter().map(|n| self.lot(n).id).collect();
        let mut by_lot: HashMap<Uuid, (f64, f64)> = HashMap::new();
        let mut loose: BTreeMap<String, (f64, f64)> = BTreeMap::new();
        for sn in &self.swaps {
            let s = &self.p.swaps[sn.idx];
            let sh: Vec<(Uuid, f64)> = shares(s).into_iter().filter(|(l, _)| held.contains(l)).collect();
            let tot: f64 = sh.iter().map(|x| x.1).sum();
            if tot <= 0.0 {
                let e = loose.entry(s.book.clone()).or_default();
                e.0 += sn.mtm_bhd;
                e.1 += sn.dv01_bhd;
                continue;
            }
            for (l, w) in sh {
                let e = by_lot.entry(l).or_default();
                e.0 += sn.mtm_bhd * w / tot;
                e.1 += sn.dv01_bhd * w / tot;
            }
        }
        (by_lot, loose)
    }

    pub fn headline(&self, mtd: &Income) -> Headline {
        let face: f64 = self.lots.iter().map(|n| n.face_bhd).sum();
        let mv: f64 = self.lots.iter().map(|n| n.mv_bhd).sum();
        let bv: f64 = self.lots.iter().map(|n| n.bv_bhd).sum();
        let bond_mtm: f64 = self.lots.iter().map(|n| n.mtm_bhd).sum();
        let swap_mtm: f64 = self.swaps.iter().map(|s| s.mtm_bhd).sum();
        let dv01: f64 = self.lots.iter().map(|n| n.dv01_bhd).sum::<f64>() + self.swaps.iter().map(|s| s.dv01_bhd).sum::<f64>();
        let lots = self.lots.len() as u32;
        let with_id = self.lots.iter().filter(|n| self.lot(n).kondor_id.is_some()).count();
        Headline {
            portfolio_face_bhd: face,
            market_value_bhd: mv,
            book_value_bhd: bv,
            net_mtm_bhd: bond_mtm + swap_mtm,
            mtd_carry_bhd: mtd.carry,
            net_dv01_bhd: dv01,
            mtd_nii_bhd: mtd.net_nii,
            portfolio_yield: wavg(self.lots.iter().map(|n| (n.state.eir, n.bv_bhd))),
            mod_duration: wavg(self.lots.iter().map(|n| (n.mod_duration, n.mv_bhd))),
            lots,
            kondor_pct: if lots == 0 { 0.0 } else { 100.0 * with_id as f64 / lots as f64 },
            daily_carry_bhd: self.lots.iter().map(|n| (n.today.coupon + n.today.amortisation - n.today.ftp) * n.spot).sum(),
        }
    }

    /// The positions tree: every book, each bond in it, each lot of that bond.
    pub fn rows(&self, inc: &IncomeRun) -> Vec<PositionRow> {
        let (alloc, _) = self.swap_alloc();
        let total_face: f64 = self.lots.iter().map(|n| n.face_bhd).sum::<f64>().max(1e-9);
        let mut by_book: BTreeMap<String, BTreeMap<String, Vec<&LotNow>>> = BTreeMap::new();
        for n in &self.lots {
            let l = self.lot(n);
            by_book.entry(l.book.clone()).or_default().entry(l.isin.clone()).or_default().push(n);
        }
        let mut rows = Vec::new();
        for book in &self.p.books {
            let Some(bonds) = by_book.get(&book.code) else { continue };
            let mut book_rows: Vec<PositionRow> = Vec::new();
            let mut bond_groups: Vec<(PositionRow, Vec<PositionRow>)> = Vec::new();
            for (isin, lots) in bonds {
                let b = &self.p.bonds[isin];
                let lot_rows: Vec<PositionRow> = lots
                    .iter()
                    .map(|n| {
                        let l = self.lot(n);
                        let (sm, sd) = alloc.get(&l.id).copied().unwrap_or_default();
                        let _ = sm;
                        PositionRow {
                            level: RowLevel::Lot,
                            key: l.id.to_string(),
                            parent: Some(format!("{}|{}", l.book, l.isin)),
                            book: l.book.clone(),
                            isin: Some(l.isin.clone()),
                            label: format!("Bought {}", l.trade_date.format("%d %b %Y")),
                            issuer: Some(b.issuer.clone()),
                            ccy: Some(b.ccy.clone()),
                            kondor_id: l.kondor_id.clone(),
                            trade_date: Some(l.trade_date),
                            maturity: Some(b.maturity),
                            lots: 1,
                            face_local: Some(l.face),
                            face_bhd: n.face_bhd,
                            price: l.clean_price,
                            market_price: Some(n.market_price),
                            market_value_bhd: n.mv_bhd,
                            book_value_bhd: n.bv_bhd,
                            mtm_bhd: n.mtm_bhd,
                            eir: n.state.eir,
                            ytm: Some(n.ytm),
                            ftp_rate: n.state.ftp_rate,
                            dv01_bhd: n.dv01_bhd,
                            swap_dv01_bhd: sd,
                            mod_duration: n.mod_duration,
                            hedged_face_bhd: n.hedged_face * n.spot,
                            pledged_face_bhd: n.pledged_face * n.spot,
                            share: n.face_bhd / total_face,
                            income: inc.by_lot.get(&l.id).copied().unwrap_or_default(),
                            daily_carry_bhd: (n.today.coupon + n.today.amortisation - n.today.ftp) * n.spot,
                        }
                    })
                    .collect();
                let mut agg = aggregate(&lot_rows, RowLevel::Bond, format!("{}|{}", book.code, isin), Some(book.code.clone()), b.name.clone());
                agg.isin = Some(isin.clone());
                agg.issuer = Some(b.issuer.clone());
                agg.ccy = Some(b.ccy.clone());
                agg.maturity = Some(b.maturity);
                agg.face_local = Some(lots.iter().map(|n| self.lot(n).face).sum());
                agg.kondor_id = None;
                bond_groups.push((agg, lot_rows));
            }
            bond_groups.sort_by(|a, b| b.0.face_bhd.total_cmp(&a.0.face_bhd));
            let bond_aggs: Vec<PositionRow> = bond_groups.iter().map(|g| g.0.clone()).collect();
            let mut top = aggregate(&bond_aggs, RowLevel::Book, book.code.clone(), None, book.name.clone());
            top.book = book.code.clone();
            top.parent = None;
            let ccys: BTreeSet<&str> = bond_aggs.iter().filter_map(|r| r.ccy.as_deref()).collect();
            if ccys.len() == 1 {
                top.ccy = ccys.into_iter().next().map(str::to_string);
            }
            book_rows.push(top);
            for (agg, lots) in bond_groups {
                book_rows.push(agg);
                book_rows.extend(lots);
            }
            rows.extend(book_rows);
        }
        rows
    }

    pub fn book_figures(&self) -> Vec<BookFigure> {
        let (_, loose) = self.swap_alloc();
        self.p
            .books
            .iter()
            .filter_map(|bk| {
                let ns: Vec<&LotNow> = self.lots.iter().filter(|n| self.lot(n).book == bk.code).collect();
                let swaps: Vec<&SwapNow> = self.swaps.iter().filter(|s| self.p.swaps[s.idx].book == bk.code).collect();
                if ns.is_empty() && swaps.is_empty() {
                    return None;
                }
                let dv01: f64 = ns.iter().map(|n| n.dv01_bhd).sum();
                let _ = &loose;
                Some(BookFigure {
                    book: bk.code.clone(),
                    name: bk.name.clone(),
                    accounting: bk.accounting,
                    face_bhd: ns.iter().map(|n| n.face_bhd).sum(),
                    market_value_bhd: ns.iter().map(|n| n.mv_bhd).sum(),
                    book_value_bhd: ns.iter().map(|n| n.bv_bhd).sum(),
                    mtm_bhd: ns.iter().map(|n| n.mtm_bhd).sum::<f64>() + swaps.iter().map(|s| s.mtm_bhd).sum::<f64>(),
                    dv01_bhd: dv01,
                    net_dv01_bhd: dv01 + swaps.iter().map(|s| s.dv01_bhd).sum::<f64>(),
                    mod_duration: wavg(ns.iter().map(|n| (n.mod_duration, n.mv_bhd))),
                    yield_pct: wavg(ns.iter().map(|n| (n.state.eir, n.bv_bhd))),
                    daily_carry_bhd: ns.iter().map(|n| (n.today.coupon + n.today.amortisation - n.today.ftp) * n.spot).sum(),
                })
            })
            .collect()
    }

    /// Face by remaining maturity measured from `from`, hedged against unhedged. `year_end`: only
    /// what is still held then, hedged by the swaps still live then.
    pub fn maturity_buckets(&self, year_end: bool) -> Vec<HedgeBucket> {
        let refd = if year_end { year_end_of(self.on) } else { self.on };
        let mut out: Vec<HedgeBucket> = MATURITY_BUCKETS.iter().map(|(l, _)| HedgeBucket { label: l.to_string(), hedged_bhd: 0.0, unhedged_bhd: 0.0 }).collect();
        for n in &self.lots {
            let (l, b) = (self.lot(n), self.bond(n));
            if b.maturity <= refd {
                continue;
            }
            let hedged = if year_end { hedged_face(self.p, l, refd) } else { n.hedged_face };
            let i = bucket_of((b.maturity - refd).num_days() as f64 / 365.0);
            out[i].hedged_bhd += hedged * n.spot;
            out[i].unhedged_bhd += (l.face - hedged) * n.spot;
        }
        out
    }

    pub fn year_end_face(&self) -> (f64, f64) {
        let ye = year_end_of(self.on);
        let mut face = 0.0;
        let mut hedged = 0.0;
        for n in &self.lots {
            let (l, b) = (self.lot(n), self.bond(n));
            if b.maturity > ye {
                face += n.face_bhd;
                hedged += hedged_face(self.p, l, ye) * n.spot;
            }
        }
        (face, hedged)
    }

    pub fn hedge_pairs(&self) -> Vec<HedgePair> {
        let (alloc, _) = self.swap_alloc();
        let mut by: BTreeMap<String, HedgePair> = BTreeMap::new();
        for n in &self.lots {
            let (l, b) = (self.lot(n), self.bond(n));
            let (sm, sd) = alloc.get(&l.id).copied().unwrap_or_default();
            let e = by.entry(b.isin.clone()).or_insert_with(|| HedgePair {
                isin: b.isin.clone(),
                name: b.name.clone(),
                ccy: b.ccy.clone(),
                face_bhd: 0.0,
                hedged_face_bhd: 0.0,
                bond_dv01: 0.0,
                irs_dv01: 0.0,
                net_dv01: 0.0,
                bond_mtm: 0.0,
                irs_mtm: 0.0,
                net_mtm: 0.0,
                closed_notional: 0.0,
                swaps: 0,
            });
            e.face_bhd += n.face_bhd;
            e.hedged_face_bhd += n.hedged_face * n.spot;
            e.bond_dv01 += n.dv01_bhd;
            e.irs_dv01 += sd;
            e.bond_mtm += n.mtm_bhd;
            e.irs_mtm += sm;
        }
        // Closed notional and the number of swaps, shared to each bond by assigned notional.
        for s in &self.p.swaps {
            let closed: f64 = s.unwinds.iter().filter(|u| u.date <= self.on).map(|u| u.notional).sum();
            let mut seen = BTreeSet::new();
            for (l, w) in shares(s) {
                let Some(lot) = self.p.lots.iter().find(|x| x.id == l) else { continue };
                if let Some(e) = by.get_mut(&lot.isin) {
                    e.closed_notional += closed * w;
                    if seen.insert(lot.isin.clone()) && swap_live(s, self.on) {
                        e.swaps += 1;
                    }
                }
            }
        }
        let mut out: Vec<HedgePair> = by
            .into_values()
            .filter(|h| h.swaps > 0 || h.closed_notional > 0.0)
            .map(|mut h| {
                h.net_dv01 = h.bond_dv01 + h.irs_dv01;
                h.net_mtm = h.bond_mtm + h.irs_mtm;
                h
            })
            .collect();
        out.sort_by(|a, b| b.face_bhd.total_cmp(&a.face_bhd));
        out
    }

    pub fn dv01_buckets(&self) -> Vec<Dv01Bucket> {
        let mut out: Vec<Dv01Bucket> = MATURITY_BUCKETS.iter().map(|(l, _)| Dv01Bucket { label: l.to_string(), bond: 0.0, irs: 0.0, net: 0.0 }).collect();
        for n in &self.lots {
            out[bucket_of((self.bond(n).maturity - self.on).num_days() as f64 / 365.0)].bond += n.dv01_bhd;
        }
        for s in &self.swaps {
            out[bucket_of((self.p.swaps[s.idx].maturity - self.on).num_days() as f64 / 365.0)].irs += s.dv01_bhd;
        }
        for b in &mut out {
            b.net = b.bond + b.irs;
        }
        out
    }

    pub fn shocks(&self) -> (Vec<String>, Vec<ShockRow>) {
        let books: Vec<String> = self.p.books.iter().map(|b| b.code.clone()).collect();
        let rows = SHOCKS_BP
            .iter()
            .map(|bp| {
                let mut by: Vec<f64> = vec![0.0; books.len()];
                for n in &self.lots {
                    let (l, b) = (self.lot(n), self.bond(n));
                    if let Some(i) = books.iter().position(|x| *x == l.book) {
                        by[i] += n.shock_bhd(b, &schedule(b), self.on, *bp, l.face);
                    }
                }
                for sn in &self.swaps {
                    let s = &self.p.swaps[sn.idx];
                    if let Some(i) = books.iter().position(|x| *x == s.book) {
                        by[i] += (swap_pv(self.m, s, self.on, *bp) - swap_pv(self.m, s, self.on, 0.0)) * sn.spot;
                    }
                }
                ShockRow { shift_bp: *bp, total: by.iter().sum(), by_book: by }
            })
            .collect();
        (books, rows)
    }

    #[allow(clippy::type_complexity)]
    pub fn yield_by_year(&self) -> Vec<YieldRow> {
        // (book or all, year) -> (face, eir*face, unhedged face, eir*unhedged face)
        let mut acc: BTreeMap<(Option<String>, i32), (f64, f64, f64, f64)> = BTreeMap::new();
        for n in &self.lots {
            let (l, b) = (self.lot(n), self.bond(n));
            let un = (l.face - n.hedged_face).max(0.0) * n.spot;
            for key in [None, Some(l.book.clone())] {
                let e = acc.entry((key, b.maturity.year())).or_default();
                e.0 += n.face_bhd;
                e.1 += n.state.eir * n.face_bhd;
                e.2 += un;
                e.3 += n.state.eir * un;
            }
        }
        acc.into_iter()
            .map(|((book, year), (f, ef, u, eu))| YieldRow { book, year, face_bhd: f, yield_all: if f > 0.0 { ef / f } else { 0.0 }, yield_unhedged: (u > 1e-6).then(|| eu / u) })
            .collect()
    }

    pub fn hedged_by_ccy(&self) -> Vec<CcyHedge> {
        let mut m: BTreeMap<String, (f64, f64)> = BTreeMap::new();
        for n in &self.lots {
            let (l, b) = (self.lot(n), self.bond(n));
            let e = m.entry(b.ccy.clone()).or_default();
            e.0 += n.hedged_face * n.spot;
            e.1 += (l.face - n.hedged_face) * n.spot;
        }
        m.into_iter().map(|(ccy, (h, u))| CcyHedge { ccy, hedged_bhd: h, unhedged_bhd: u }).collect()
    }

    pub fn swap_views(&self) -> Vec<SwapView> {
        self.swaps
            .iter()
            .map(|sn| {
                let s = &self.p.swaps[sn.idx];
                let mut isins: Vec<String> = s.hedges.iter().filter_map(|(l, _)| self.p.lots.iter().find(|x| x.id == *l).map(|x| x.isin.clone())).collect();
                isins.sort();
                isins.dedup();
                SwapView {
                    id: s.id,
                    book: s.book.clone(),
                    kondor_id: s.kondor_id.clone(),
                    ccy: s.ccy.clone(),
                    notional: sn.notional,
                    closed_notional: sn.closed,
                    start: s.start,
                    maturity: s.maturity,
                    pay_fixed: s.pay_fixed,
                    fixed_rate: s.fixed_rate,
                    float_index: s.float_index.clone(),
                    float_rate: sn.float_rate,
                    mtm_bhd: sn.mtm_bhd,
                    dv01_bhd: sn.dv01_bhd,
                    isins,
                }
            })
            .collect()
    }

    // ----- funding -----

    pub fn repo_views(&self) -> Vec<RepoView> {
        let mut out: Vec<RepoView> = self
            .p
            .repos
            .iter()
            .filter(|r| r.start <= self.on && r.end > self.on)
            .map(|r| {
                let mut isins = Vec::new();
                let mut coll = 0.0;
                for (l, f) in &r.collateral {
                    if let Some(lot) = self.p.lots.iter().find(|x| x.id == *l) {
                        isins.push(lot.isin.clone());
                        let ccy = self.p.bonds.get(&lot.isin).map(|b| b.ccy.as_str()).unwrap_or("BHD");
                        coll += f * self.m.spot(ccy, self.on);
                    }
                }
                isins.sort();
                isins.dedup();
                RepoView {
                    id: r.id,
                    counterparty: r.counterparty.clone(),
                    kondor_id: r.kondor_id.clone(),
                    ccy: r.ccy.clone(),
                    start: r.start,
                    end: r.end,
                    cash_bhd: r.cash * self.m.spot(&r.ccy, self.on),
                    rate: r.rate,
                    haircut_pct: r.haircut_pct,
                    days_left: (r.end - self.on).num_days(),
                    collateral_face_bhd: coll,
                    isins,
                }
            })
            .collect();
        out.sort_by_key(|r| r.end);
        out
    }

    pub fn repo_summary(&self, receipts_12m: f64) -> RepoSummary {
        let views = self.repo_views();
        let amount: f64 = views.iter().map(|r| r.cash_bhd).sum();
        let pledged: f64 = self.lots.iter().map(|n| n.pledged_face * n.spot).sum();
        let held: f64 = self.lots.iter().map(|n| n.face_bhd).sum();
        RepoSummary {
            amount_bhd: amount,
            available_face_bhd: (held - pledged).max(0.0),
            pledged_face_bhd: pledged,
            due_7d_bhd: views.iter().filter(|r| r.days_left <= 7).map(|r| r.cash_bhd).sum(),
            active: views.len() as u32,
            avg_rate: wavg(views.iter().map(|r| (r.rate, r.cash_bhd))),
            avg_haircut: wavg(views.iter().map(|r| (r.haircut_pct, r.cash_bhd))),
            wtd_maturity_days: wavg(views.iter().map(|r| (r.days_left as f64, r.cash_bhd))),
            receipts_12m_bhd: receipts_12m,
        }
    }

    pub fn repo_by_remaining(&self) -> Vec<NamedAmount> {
        let mut out: Vec<NamedAmount> = REPO_BUCKETS.iter().map(|(l, _)| NamedAmount { label: l.to_string(), amount: 0.0 }).collect();
        for r in self.repo_views() {
            let i = REPO_BUCKETS.iter().position(|(_, d)| r.days_left <= *d).unwrap_or(REPO_BUCKETS.len() - 1);
            out[i].amount += r.cash_bhd;
        }
        out
    }

    pub fn collateral_by_book(&self) -> Vec<CollateralBook> {
        self.p
            .books
            .iter()
            .filter_map(|bk| {
                let ns: Vec<&LotNow> = self.lots.iter().filter(|n| self.lot(n).book == bk.code).collect();
                if ns.is_empty() {
                    return None;
                }
                let pledged: f64 = ns.iter().map(|n| n.pledged_face * n.spot).sum();
                let held: f64 = ns.iter().map(|n| n.face_bhd).sum();
                Some(CollateralBook { book: bk.code.clone(), pledged_bhd: pledged, free_bhd: (held - pledged).max(0.0) })
            })
            .collect()
    }

    /// Repos starting (a) and ending (b) per month, six months back to six ahead.
    pub fn funding_flows(&self) -> Vec<MonthFlow> {
        let first = add_months(month_start(self.on), -6);
        let mut out: Vec<MonthFlow> = (0..13).map(|i| MonthFlow { month: add_months(first, i), a: 0.0, b: 0.0 }).collect();
        for r in &self.p.repos {
            let fx = self.m.spot(&r.ccy, self.on);
            for (d, is_start) in [(r.start, true), (r.end, false)] {
                if let Some(mf) = out.iter_mut().find(|x| month_start(d) == x.month) {
                    if is_start {
                        mf.a += r.cash * fx;
                    } else {
                        mf.b += r.cash * fx;
                    }
                }
            }
        }
        out
    }

    pub fn quotes(&self) -> Vec<QuoteView> {
        let mut latest: BTreeMap<(String, i32), &RepoQuote> = BTreeMap::new();
        for q in &self.p.quotes {
            if q.as_of > self.on {
                continue;
            }
            let k = (q.counterparty.clone(), q.tenor_days);
            if latest.get(&k).is_none_or(|x| x.as_of < q.as_of) {
                latest.insert(k, q);
            }
        }
        latest
            .into_values()
            .map(|q| QuoteView {
                counterparty: q.counterparty.clone(),
                tenor: q.tenor.clone(),
                tenor_days: q.tenor_days,
                spread_bp: q.spread_bp,
                as_of: q.as_of,
                age_days: (self.on - q.as_of).num_days(),
                sofr: self.m.history.contains_key("USD").then(|| self.m.fixing("USD", q.as_of, q.tenor_days as i64)),
            })
            .collect()
    }

    /// Coupons (a) and principal (b) to be received per month over the next twelve months.
    pub fn receipts(&self) -> Vec<MonthFlow> {
        let first = add_months(month_start(self.on), 1);
        let mut out: Vec<MonthFlow> = (0..12).map(|i| MonthFlow { month: add_months(month_start(self.on), i), a: 0.0, b: 0.0 }).collect();
        let _ = first;
        let end = self.on + Duration::days(365);
        for n in &self.lots {
            let (l, b) = (self.lot(n), self.bond(n));
            walk_lot(self.m, l, b, self.on + Duration::days(1), end, |d| {
                if (d.coupon_cash != 0.0 || d.principal != 0.0)
                    && let Some(mf) = out.iter_mut().find(|x| x.month == month_start(d.date))
                {
                    mf.a += d.coupon_cash * n.spot;
                    mf.b += d.principal * n.spot;
                }
            });
        }
        out
    }

    /// Net swap settlements (a) per month, five months back to twelve ahead, and unwind cash (b).
    pub fn hedge_cash(&self) -> Vec<MonthFlow> {
        let first = add_months(month_start(self.on), -5);
        let mut out: Vec<MonthFlow> = (0..18).map(|i| MonthFlow { month: add_months(first, i), a: 0.0, b: 0.0 }).collect();
        let last = add_months(first, 18) - Duration::days(1);
        for s in &self.p.swaps {
            let fx = self.m.spot(&s.ccy, self.on);
            for (d, v) in swap_payments(self.m, s, first, last) {
                if let Some(mf) = out.iter_mut().find(|x| x.month == month_start(d)) {
                    mf.a += v * fx;
                }
            }
            for u in &s.unwinds {
                if let Some(mf) = out.iter_mut().find(|x| x.month == month_start(u.date)) {
                    mf.b += u.settlement * fx;
                }
            }
        }
        out
    }

    pub fn warnings(&self) -> Vec<Warning> {
        let mut out = Vec::new();
        let mut no_curve: BTreeSet<String> = BTreeSet::new();
        for n in &self.lots {
            if n.state.ftp_rate.is_none() {
                no_curve.insert(self.bond(n).ccy.clone());
            }
        }
        for ccy in no_curve {
            out.push(Warning { key: format!("ftp:{ccy}"), message: format!("No FTP curve for {ccy}: those bonds are carried without a funding charge") });
        }
        let missing = self.lots.iter().filter(|n| self.lot(n).kondor_id.is_none()).count();
        if missing > 0 {
            out.push(Warning { key: "kondor".into(), message: format!("{missing} purchase {} without a Kondor deal id", if missing == 1 { "lot" } else { "lots" }) });
        }
        out
    }
}

fn year_end_of(d: NaiveDate) -> NaiveDate {
    year_end(d)
}

/// A parent row that adds up its children (sums, and weighted prices, yields and duration).
fn aggregate(children: &[PositionRow], level: RowLevel, key: String, parent: Option<String>, label: String) -> PositionRow {
    let mut income = Income::default();
    for c in children {
        income.add(&c.income);
    }
    let face: f64 = children.iter().map(|c| c.face_bhd).sum();
    PositionRow {
        level,
        key,
        parent,
        book: children.first().map(|c| c.book.clone()).unwrap_or_default(),
        isin: None,
        label,
        issuer: None,
        ccy: None,
        kondor_id: None,
        trade_date: children.iter().filter_map(|c| c.trade_date).min(),
        maturity: children.iter().filter_map(|c| c.maturity).max(),
        lots: children.iter().map(|c| c.lots).sum(),
        face_local: None,
        face_bhd: face,
        price: wavg(children.iter().map(|c| (c.price, c.face_bhd))),
        market_price: Some(wavg(children.iter().map(|c| (c.market_price.unwrap_or(c.price), c.face_bhd)))),
        market_value_bhd: children.iter().map(|c| c.market_value_bhd).sum(),
        book_value_bhd: children.iter().map(|c| c.book_value_bhd).sum(),
        mtm_bhd: children.iter().map(|c| c.mtm_bhd).sum(),
        eir: wavg(children.iter().map(|c| (c.eir, c.book_value_bhd))),
        ytm: Some(wavg(children.iter().map(|c| (c.ytm.unwrap_or(c.eir), c.market_value_bhd)))),
        ftp_rate: {
            let with: Vec<&PositionRow> = children.iter().filter(|c| c.ftp_rate.is_some()).collect();
            (!with.is_empty()).then(|| wavg(with.iter().map(|c| (c.ftp_rate.unwrap_or(0.0), c.book_value_bhd))))
        },
        dv01_bhd: children.iter().map(|c| c.dv01_bhd).sum(),
        swap_dv01_bhd: children.iter().map(|c| c.swap_dv01_bhd).sum(),
        mod_duration: wavg(children.iter().map(|c| (c.mod_duration, c.market_value_bhd))),
        hedged_face_bhd: children.iter().map(|c| c.hedged_face_bhd).sum(),
        pledged_face_bhd: children.iter().map(|c| c.pledged_face_bhd).sum(),
        share: children.iter().map(|c| c.share).sum(),
        income,
        daily_carry_bhd: children.iter().map(|c| c.daily_carry_bhd).sum(),
    }
}

// ---------------------------------------------------------------------------------------------
// Income by name, and the forecast
// ---------------------------------------------------------------------------------------------

/// Income by bond across books (including bonds since matured), largest first.
pub fn income_by_bond(p: &Portfolio, run: &IncomeRun) -> Vec<NamedIncome> {
    let mut m: BTreeMap<String, Income> = BTreeMap::new();
    for l in &p.lots {
        if let Some(i) = run.by_lot.get(&l.id) {
            m.entry(l.isin.clone()).or_default().add(i);
        }
    }
    let mut out: Vec<NamedIncome> = m
        .into_iter()
        .map(|(isin, income)| NamedIncome { label: p.bonds.get(&isin).map(|b| b.name.clone()).unwrap_or_else(|| isin.clone()), key: isin, income })
        .collect();
    for (book, i) in &run.unlinked {
        out.push(NamedIncome { key: format!("{UNLINKED}:{book}"), label: if book == UNLINKED { "Repo funding not tied to a bond".into() } else { format!("Swaps in {book} not linked to a held bond") }, income: *i });
    }
    out.sort_by(|a, b| b.income.carry.abs().total_cmp(&a.income.carry.abs()));
    out
}

pub fn income_by_book(p: &Portfolio, run: &IncomeRun) -> Vec<NamedIncome> {
    let names: HashMap<&str, &str> = p.books.iter().map(|b| (b.code.as_str(), b.name.as_str())).collect();
    run.by_book(p)
        .into_iter()
        .map(|(k, income)| NamedIncome { label: names.get(k.as_str()).map(|s| s.to_string()).unwrap_or_else(|| if k == UNLINKED { "Repo funding".into() } else { k.clone() }), key: k, income })
        .collect()
}

/// The portfolio extended with what maturing face is reinvested into until `horizon`: the same
/// face, at par, for the original tenor, at today's curve for that tenor plus the bond's spread
/// over the curve today, charged today's FTP rate for the tenor.
pub fn with_reinvestment(m: &Market, p: &Portfolio, on: NaiveDate, horizon: NaiveDate) -> Portfolio {
    let mut q = p.clone();
    let mut queue: Vec<(Lot, Bond)> = p
        .lots
        .iter()
        .filter_map(|l| {
            let b = p.bonds.get(&l.isin)?;
            (held_on(l, b, on) && b.maturity <= horizon).then(|| (l.clone(), b.clone()))
        })
        .collect();
    let mut round = 0;
    while let Some((l, b)) = queue.pop() {
        round += 1;
        if round > 2_000 {
            break;
        }
        let tenor_days = (b.maturity - b.issue_date).num_days().max(365);
        let months = ((tenor_days as f64) / 365.0 * 12.0).round().max(12.0) as i64;
        let curve = m.curve(&b.ccy, 0.0);
        let rem = (b.maturity - on).num_days().max(30);
        let spread = match m.price(&b.isin, on) {
            Some((_, y)) if b.rate_type == BondRateType::Fixed => (y - curve_rate(&curve, rem)).clamp(0.0, 3.0),
            _ => 0.5,
        };
        let coupon = (curve_rate(&curve, tenor_days) + spread).max(0.0);
        let isin = format!("{}~{}", b.isin, round);
        let nb = Bond {
            isin: isin.clone(),
            name: format!("{} (reinvested)", b.name.trim_end_matches(" (reinvested)")),
            issue_date: b.maturity,
            maturity: add_months(b.maturity, months),
            coupon: (coupon * 1e4).round() / 1e4,
            rate_type: BondRateType::Fixed,
            float_index: None,
            float_spread: 0.0,
            ..b.clone()
        };
        let nl = Lot {
            id: Uuid::new_v5(&l.id, format!("reinvest-{round}").as_bytes()),
            isin,
            trade_date: b.maturity,
            settle_date: b.maturity,
            clean_price: 100.0,
            accrued_paid: 0.0,
            kondor_id: None,
            eir: nb.coupon,
            ftp_rate: m.ftp.rate(&b.ccy, on, tenor_days).map(|x| x.0),
            ftp_publication_id: None,
            status: "PROJECTED".into(),
            ..l.clone()
        };
        if nb.maturity <= horizon {
            queue.push((nl.clone(), nb.clone()));
        }
        q.bonds.insert(nb.isin.clone(), nb);
        q.lots.push(nl);
    }
    q
}

pub struct Forecast {
    pub year_to_date: Income,
    pub rest_of_year: Income,
    pub full_year: Income,
    pub monthly: Vec<MonthIncome>,
    pub periods: Vec<PeriodIncome>,
    pub by_book: Vec<BookYear>,
}

/// This year's income, actual to `on` and forecast after it, the following three calendar years,
/// and this year by book. Swaps run to their maturity; repos to their end date.
pub fn forecast(m: &Market, p: &Portfolio, on: NaiveDate, reinvest: bool) -> Forecast {
    let ys = year_start(on);
    let ye = year_end(on);
    let horizon = NaiveDate::from_ymd_opt(on.year() + 3, 12, 31).unwrap_or(ye);
    let ext;
    let q = if reinvest {
        ext = with_reinvestment(m, p, on, horizon);
        &ext
    } else {
        p
    };
    let actual = income(m, p, ys, on);
    let rest_run = income(m, q, on + Duration::days(1), ye);
    let later = income(m, q, ye + Duration::days(1), horizon);
    let mut ahead = rest_run.clone();
    ahead.daily.extend(later.daily);
    let mut monthly: Vec<MonthIncome> = (0..12).map(|i| MonthIncome { month: add_months(ys, i), actual: add_months(ys, i) <= on, income: Income::default() }).collect();
    for (d, i) in actual.daily.iter().chain(ahead.daily.iter().filter(|(d, _)| **d <= ye)) {
        if let Some(mi) = monthly.iter_mut().find(|x| x.month == month_start(*d)) {
            mi.income.add(i);
        }
    }
    let sum = |run: &IncomeRun, a: NaiveDate, b: NaiveDate| {
        let mut t = Income::default();
        for (_, i) in run.daily.range(a..=b) {
            t.add(i);
        }
        t
    };
    let rest = sum(&ahead, on + Duration::days(1), ye);
    let mut full = actual.total;
    full.add(&rest);
    let mut periods = vec![PeriodIncome { label: format!("Rest of {}", on.year()), from: on + Duration::days(1), to: ye, partial: true, income: rest }];
    for y in 1..=3 {
        let (a, b) = (NaiveDate::from_ymd_opt(on.year() + y, 1, 1).unwrap_or(ye), NaiveDate::from_ymd_opt(on.year() + y, 12, 31).unwrap_or(ye));
        periods.push(PeriodIncome { label: (on.year() + y).to_string(), from: a, to: b, partial: false, income: sum(&ahead, a, b) });
    }
    let act_book = actual.by_book(p);
    let fc_book = rest_run.by_book(q);
    let keys: BTreeSet<&String> = act_book.keys().chain(fc_book.keys()).collect();
    let by_book = keys.into_iter().map(|k| BookYear { book: k.clone(), actual: act_book.get(k).copied().unwrap_or_default(), forecast: fc_book.get(k).copied().unwrap_or_default() }).collect();
    Forecast { year_to_date: actual.total, rest_of_year: rest, full_year: full, monthly, periods, by_book }
}

/// Accounting class of a book, for the pages that label books.
pub fn accounting_of(p: &Portfolio, book: &str) -> Option<Accounting> {
    p.books.iter().find(|b| b.code == book).map(|b| b.accounting)
}

/// The bond's own cash flows from all held lots after `on`, and its swaps' settlements, by date.
pub fn bond_cash_flows(m: &Market, p: &Portfolio, isin: &str, on: NaiveDate, until: NaiveDate) -> Vec<(NaiveDate, String, f64, f64)> {
    let mut out = Vec::new();
    let Some(b) = p.bonds.get(isin) else { return out };
    let fx = m.spot(&b.ccy, on);
    let mut by: BTreeMap<(NaiveDate, &str), f64> = BTreeMap::new();
    for l in p.lots.iter().filter(|l| l.isin == isin && held_on(l, b, on)) {
        walk_lot(m, l, b, on + Duration::days(1), until, |d| {
            if d.coupon_cash != 0.0 {
                *by.entry((d.date, "Coupon")).or_default() += d.coupon_cash;
            }
            if d.principal != 0.0 {
                *by.entry((d.date, "Redemption")).or_default() += d.principal;
            }
        });
    }
    let lot_ids: BTreeSet<Uuid> = p.lots.iter().filter(|l| l.isin == isin).map(|l| l.id).collect();
    for s in &p.swaps {
        let share: f64 = shares(s).into_iter().filter(|(l, _)| lot_ids.contains(l)).map(|x| x.1).sum();
        if share <= 0.0 {
            continue;
        }
        for (d, v) in swap_payments(m, s, on + Duration::days(1), until) {
            *by.entry((d, "Swap settlement")).or_default() += v * share;
        }
    }
    for ((d, k), v) in by {
        out.push((d, k.to_string(), v, v * fx));
    }
    out
}

/// The effective rate a lot would carry if bought on `settle` at `clean` (a convenience for the
/// generator and booking, which store it with the lot).
pub fn lot_eir(b: &Bond, settle: NaiveDate, clean: f64, m: &Market) -> f64 {
    let rate = match b.rate_type {
        BondRateType::Fixed => b.coupon,
        BondRateType::Float => {
            let sched = schedule(b);
            match bond::period_of(b.issue_date, &sched, settle) {
                Some((prev, next)) => m.fixing(&b.ccy, prev, (next - prev).num_days()) + b.float_spread,
                None => b.float_spread,
            }
        }
    };
    bond::purchase_eir(b, settle, clean, rate)
}
