//! The daily accrual of a purchase lot at its effective interest rate, its FTP charge, and the
//! daily carry and settlements of a swap and the interest of a repo. All in local currency.
//!
//! A lot's amortised cost starts at what was paid (clean price plus accrued) and each day grows at
//! the effective rate, falling by every coupon and the redemption, so it reaches zero exactly at
//! maturity. The day's income splits into the coupon accrual (the change in accrued interest plus
//! any coupon paid) and the amortisation (the rest). The FTP charge is the book value (amortised
//! cost less accrued) at the locked rate on the money-market basis. A floating bond re-fixes its
//! coupon, re-locks its FTP rate and re-solves its effective rate at each reset.

use chrono::{Duration, NaiveDate};
use domain::{Bond, BondRateType, Lot, Repo, Swap};

use crate::bond::{self, accrued, coupon_cash, dates_back, flows_after, leg_fraction, period_of, schedule};
use crate::market::Market;

/// Money-market basis of a currency.
pub fn mm_basis(ccy: &str) -> f64 {
    if ccy == "GBP" { 365.0 } else { 360.0 }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LotDay {
    pub date: NaiveDate,
    pub coupon: f64,
    pub amortisation: f64,
    pub ftp: f64,
    /// Amortised cost less accrued, at the end of the day.
    pub book_value: f64,
    pub accrued: f64,
    pub coupon_cash: f64,
    pub principal: f64,
}

/// Where a lot stands at the end of a walk.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LotState {
    pub date: NaiveDate,
    pub ac_dirty: f64,
    pub accrued: f64,
    pub book_value: f64,
    pub eir: f64,
    pub coupon_rate: f64,
    pub ftp_rate: Option<f64>,
    pub matured: bool,
    /// The coupon period the date falls in.
    pub period: Option<(NaiveDate, NaiveDate)>,
}

fn period_rate(m: &Market, b: &Bond, prev: NaiveDate, next: NaiveDate) -> f64 {
    match b.rate_type {
        BondRateType::Fixed => b.coupon,
        BondRateType::Float => m.fixing(&b.ccy, prev, (next - prev).num_days()) + b.float_spread,
    }
}

/// Walks a lot from its settlement to `to` (or its maturity), calling `f` for each day from
/// `from` on. Days before `from` are walked to build the amortised cost but not reported.
pub fn walk_lot(m: &Market, lot: &Lot, b: &Bond, from: NaiveDate, to: NaiveDate, mut f: impl FnMut(&LotDay)) -> LotState {
    let sched = schedule(b);
    let s = lot.settle_date;
    let face = lot.face / 100.0;
    let Some((mut prev, mut next)) = period_of(b.issue_date, &sched, s) else {
        return LotState { date: s, matured: true, ..Default::default() };
    };
    let mut rate = period_rate(m, b, prev, next);
    let mut acc = accrued(b, &sched, s, rate) * face;
    let mut ac = lot.clean_price * face + acc;
    let mut eir = bond::eir_of(s, ac / face, &flows_after(b, &sched, s, rate));
    let mut ftp = lot.ftp_rate;
    let basis = mm_basis(&b.ccy);
    let daily = |e: f64| (1.0 + e / 100.0).powf(1.0 / 365.0) - 1.0;
    let mut factor = daily(eir);
    let mut d = s;
    while d < to && d < b.maturity {
        let nd = d + Duration::days(1);
        let income = ac * factor;
        let ftp_amt = ftp.map(|r| (ac - acc) * r / 100.0 / basis).unwrap_or(0.0);
        ac += income;
        let (mut cash, mut principal) = (0.0, 0.0);
        if nd == next {
            cash = coupon_cash(b.day_count, rate, b.coupon_freq, prev, next) * face;
            if nd == b.maturity {
                principal = lot.face;
            }
            ac -= cash + principal;
            if nd < b.maturity
                && let Some((p, n)) = period_of(b.issue_date, &sched, nd)
            {
                prev = p;
                next = n;
                if b.rate_type == BondRateType::Float {
                    rate = period_rate(m, b, prev, next);
                    if let Some((r, _)) = m.ftp.rate(&b.ccy, nd, (next - prev).num_days()) {
                        ftp = Some(r);
                    }
                    eir = bond::eir_of(nd, ac / face, &flows_after(b, &sched, nd, rate));
                    factor = daily(eir);
                }
            }
        }
        let acc_new = if nd >= b.maturity { 0.0 } else { accrued(b, &sched, nd, rate) * face };
        let coupon = acc_new - acc + cash;
        let mut amortisation = income - coupon;
        if nd >= b.maturity {
            // What is left of the amortised cost after the redemption is solver residue: close it.
            amortisation -= ac;
            ac = 0.0;
        }
        acc = acc_new;
        if nd >= from {
            f(&LotDay { date: nd, coupon, amortisation, ftp: ftp_amt, book_value: ac - acc, accrued: acc, coupon_cash: cash, principal });
        }
        d = nd;
    }
    LotState {
        date: d,
        ac_dirty: ac,
        accrued: acc,
        book_value: ac - acc,
        eir,
        coupon_rate: rate,
        ftp_rate: ftp,
        matured: d >= b.maturity,
        period: period_of(b.issue_date, &sched, d),
    }
}

/// The swap's live notional for accruals on `d`: unwinds take effect the day after their date.
pub fn swap_notional(s: &Swap, d: NaiveDate) -> f64 {
    (s.notional - s.unwinds.iter().filter(|u| u.date < d).map(|u| u.notional).sum::<f64>()).max(0.0)
}

pub fn float_periods(s: &Swap) -> Vec<(NaiveDate, NaiveDate)> {
    let ends = dates_back(s.start, s.maturity, s.float_tenor_months as i64);
    let mut out = Vec::with_capacity(ends.len());
    let mut p = s.start;
    for e in ends {
        out.push((p, e));
        p = e;
    }
    out
}

pub fn fixed_periods(s: &Swap) -> Vec<(NaiveDate, NaiveDate)> {
    let ends = dates_back(s.start, s.maturity, bond::step_months(s.fixed_freq));
    let mut out = Vec::with_capacity(ends.len());
    let mut p = s.start;
    for e in ends {
        out.push((p, e));
        p = e;
    }
    out
}

/// The floating rate set for the period holding `d`, spread included.
pub fn float_rate_on(m: &Market, s: &Swap, d: NaiveDate) -> f64 {
    let periods = float_periods(s);
    let (p0, p1) = periods.iter().find(|(a, b)| *a < d && d <= *b).or(periods.last()).copied().unwrap_or((s.start, s.maturity));
    m.fixing(&s.ccy, p0, (p1 - p0).num_days()) + s.float_spread
}

/// A swap's daily carry from `from` to `to`: receive leg less pay leg, for the bank.
pub fn walk_swap(m: &Market, s: &Swap, from: NaiveDate, to: NaiveDate, mut f: impl FnMut(NaiveDate, f64)) {
    let periods = float_periods(s);
    let fixed_day = match s.fixed_day_count {
        domain::BondDayCount::Act360 | domain::BondDayCount::Thirty360 => 1.0 / 360.0,
        _ => 1.0 / 365.0,
    };
    let start = from.max(s.start + Duration::days(1));
    let end = to.min(s.maturity);
    let mut d = start;
    let mut pi = 0;
    let mut cur: Option<(NaiveDate, f64)> = None;
    while d <= end {
        while pi < periods.len() && periods[pi].1 < d {
            pi += 1;
        }
        let Some(&(p0, p1)) = periods.get(pi) else { break };
        let float = match cur {
            Some((k, r)) if k == p0 => r,
            _ => {
                let r = m.fixing(&s.ccy, p0, (p1 - p0).num_days()) + s.float_spread;
                cur = Some((p0, r));
                r
            }
        };
        let n = swap_notional(s, d);
        if n > 0.0 {
            let fixed_amt = n * s.fixed_rate / 100.0 * fixed_day;
            let float_amt = n * float / 100.0 / 360.0;
            f(d, if s.pay_fixed { float_amt - fixed_amt } else { fixed_amt - float_amt });
        }
        d += Duration::days(1);
    }
}

/// The swap's net settlements on its payment dates in `from..=to`, for the bank.
pub fn swap_payments(m: &Market, s: &Swap, from: NaiveDate, to: NaiveDate) -> Vec<(NaiveDate, f64)> {
    let mut out: std::collections::BTreeMap<NaiveDate, f64> = std::collections::BTreeMap::new();
    let sign = if s.pay_fixed { 1.0 } else { -1.0 };
    for (p0, p1) in fixed_periods(s) {
        if p1 >= from && p1 <= to {
            let n = swap_notional(s, p1);
            *out.entry(p1).or_default() -= sign * n * s.fixed_rate / 100.0 * leg_fraction(s.fixed_day_count, p0, p1);
        }
    }
    for (p0, p1) in float_periods(s) {
        if p1 >= from && p1 <= to {
            let n = swap_notional(s, p1);
            let r = m.fixing(&s.ccy, p0, (p1 - p0).num_days()) + s.float_spread;
            *out.entry(p1).or_default() += sign * n * r / 100.0 * (p1 - p0).num_days() as f64 / 360.0;
        }
    }
    out.into_iter().filter(|(_, v)| v.abs() > 1e-9).collect()
}

/// A swap's value on `on` in local currency for the bank, with every curve moved by `bp`.
/// Single curve: the floating leg is worth par plus the coupon already fixed at the next reset.
pub fn swap_pv(m: &Market, s: &Swap, on: NaiveDate, bp: f64) -> f64 {
    let n = swap_notional(s, on + Duration::days(1));
    if on >= s.maturity || n <= 0.0 {
        return 0.0;
    }
    let c = m.curve(&s.ccy, bp);
    let df = |d: NaiveDate| c.df((d - on).num_days().max(0) as f64 / 365.0);
    let fixed: f64 = fixed_periods(s).iter().filter(|(_, e)| *e > on).map(|(a, e)| n * s.fixed_rate / 100.0 * leg_fraction(s.fixed_day_count, *a, *e) * df(*e)).sum();
    let periods = float_periods(s);
    // On a reset date (or before the start) the floating leg is par; inside a period it is par at
    // the next reset plus the coupon already fixed.
    let float = match periods.iter().find(|(a, e)| *a < on && on < *e) {
        Some(&(a, e)) => {
            let fix = m.fixing(&s.ccy, a, (e - a).num_days()) + s.float_spread;
            let later: f64 = periods.iter().filter(|(x, _)| *x >= e).map(|(x, y)| n * s.float_spread / 100.0 * (*y - *x).num_days() as f64 / 360.0 * df(*y)).sum();
            n * ((1.0 + fix / 100.0 * (e - a).num_days() as f64 / 360.0) * df(e) - df(s.maturity)) + later
        }
        None => n * (df(s.start) - df(s.maturity)),
    };
    if s.pay_fixed { float - fixed } else { fixed - float }
}

/// A repo's daily interest from `from` to `to`, as a positive cost.
pub fn walk_repo(r: &Repo, from: NaiveDate, to: NaiveDate, mut f: impl FnMut(NaiveDate, f64)) {
    let mut d = from.max(r.start + Duration::days(1));
    let end = to.min(r.end);
    while d <= end {
        f(d, r.cash * r.rate / 100.0 / mm_basis(&r.ccy));
        d += Duration::days(1);
    }
}
