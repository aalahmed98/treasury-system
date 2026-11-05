//! One bond: its coupon dates, accrued interest, price and yield, the effective interest rate a
//! purchase earns, and its duration. Amounts per 100 of face unless a function takes a face.

use alm::schedule::add_months;
use chrono::NaiveDate;
use domain::{Bond, BondDayCount};

/// 30/360 (US) day count between two dates.
pub fn days_30_360(a: NaiveDate, b: NaiveDate) -> i64 {
    use chrono::Datelike;
    let (y1, m1, mut d1) = (a.year() as i64, a.month() as i64, a.day() as i64);
    let (y2, m2, mut d2) = (b.year() as i64, b.month() as i64, b.day() as i64);
    if d1 == 31 {
        d1 = 30;
    }
    if d2 == 31 && d1 == 30 {
        d2 = 30;
    }
    360 * (y2 - y1) + 30 * (m2 - m1) + (d2 - d1)
}

fn days(a: NaiveDate, b: NaiveDate) -> i64 {
    (b - a).num_days()
}

/// The share of the period `prev..next` that has run by `d`, under the day count.
pub fn period_fraction(dc: BondDayCount, prev: NaiveDate, d: NaiveDate, next: NaiveDate) -> f64 {
    let (num, den) = match dc {
        BondDayCount::Thirty360 => (days_30_360(prev, d), days_30_360(prev, next)),
        _ => (days(prev, d), days(prev, next)),
    };
    if den <= 0 { 0.0 } else { (num as f64 / den as f64).clamp(0.0, 1.0) }
}

/// The coupon paid for the period `prev..next`, per 100 of face, at `rate` percent a year.
/// Money-market (ACT/360) coupons pay the actual days; the others pay rate over frequency.
pub fn coupon_cash(dc: BondDayCount, rate: f64, freq: u32, prev: NaiveDate, next: NaiveDate) -> f64 {
    match dc {
        BondDayCount::Act360 => rate * days(prev, next) as f64 / 360.0,
        _ => rate / freq.max(1) as f64,
    }
}

/// Year fraction of one swap-leg period.
pub fn leg_fraction(dc: BondDayCount, a: NaiveDate, b: NaiveDate) -> f64 {
    match dc {
        BondDayCount::Act360 => days(a, b) as f64 / 360.0,
        BondDayCount::Thirty360 => days_30_360(a, b) as f64 / 360.0,
        BondDayCount::Act365 | BondDayCount::ActAct => days(a, b) as f64 / 365.0,
    }
}

/// Months between coupons.
pub fn step_months(freq: u32) -> i64 {
    (12 / freq.clamp(1, 12)) as i64
}

/// Payment dates counted back from `end` in steps of `months`, after `start`, ascending.
pub fn dates_back(start: NaiveDate, end: NaiveDate, months: i64) -> Vec<NaiveDate> {
    let mut out = Vec::new();
    let mut k = 0;
    loop {
        let d = add_months(end, -k * months);
        if d <= start || k > 2_000 {
            break;
        }
        out.push(d);
        k += 1;
    }
    out.reverse();
    out
}

/// The bond's coupon dates after its issue date; the last is the maturity.
pub fn schedule(b: &Bond) -> Vec<NaiveDate> {
    dates_back(b.issue_date, b.maturity, step_months(b.coupon_freq))
}

/// The coupon period holding `d`: the last coupon date on or before `d` (or the issue date) and
/// the first after it. `None` once the bond has matured.
pub fn period_of(issue: NaiveDate, sched: &[NaiveDate], d: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    let i = sched.partition_point(|x| *x <= d);
    let next = *sched.get(i)?;
    let prev = if i == 0 { issue } else { sched[i - 1] };
    Some((prev, next))
}

/// Accrued interest per 100 on `d` at `rate` for the current period.
pub fn accrued(b: &Bond, sched: &[NaiveDate], d: NaiveDate, rate: f64) -> f64 {
    match period_of(b.issue_date, sched, d) {
        Some((prev, next)) => coupon_cash(b.day_count, rate, b.coupon_freq, prev, next) * period_fraction(b.day_count, prev, d, next),
        None => 0.0,
    }
}

/// Dirty price per 100 at a yield (percent, compounded at the coupon frequency), street convention:
/// the first period discounted by the fraction of it still to run.
pub fn dirty_from_yield(b: &Bond, sched: &[NaiveDate], settle: NaiveDate, ytm: f64) -> f64 {
    let Some((prev, next)) = period_of(b.issue_date, sched, settle) else { return 0.0 };
    let f = b.coupon_freq.max(1) as f64;
    let v = 1.0 - period_fraction(b.day_count, prev, settle, next);
    let base = 1.0 + ytm / 100.0 / f;
    let first = sched.partition_point(|x| *x <= settle);
    let mut pv = 0.0;
    let mut p = prev;
    for (k, d) in sched[first..].iter().enumerate() {
        let mut cf = coupon_cash(b.day_count, b.coupon, b.coupon_freq, p, *d);
        if *d == b.maturity {
            cf += 100.0;
        }
        pv += cf / base.powf(v + k as f64);
        p = *d;
    }
    pv
}

pub fn clean_from_yield(b: &Bond, sched: &[NaiveDate], settle: NaiveDate, ytm: f64) -> f64 {
    dirty_from_yield(b, sched, settle, ytm) - accrued(b, sched, settle, b.coupon)
}

/// Solves `f(x) = target` for an increasing or decreasing `f` on `[lo, hi]` by bisection.
fn solve(mut lo: f64, mut hi: f64, target: f64, f: impl Fn(f64) -> f64) -> f64 {
    let (flo, fhi) = (f(lo) - target, f(hi) - target);
    if flo.signum() == fhi.signum() {
        return if flo.abs() < fhi.abs() { lo } else { hi };
    }
    let rising = fhi > flo;
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        let v = f(mid) - target;
        if v.abs() < 1e-13 || (hi - lo) < 1e-14 {
            return mid;
        }
        if (v > 0.0) == rising {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    0.5 * (lo + hi)
}

/// Yield (percent) from a clean price per 100.
pub fn yield_from_clean(b: &Bond, sched: &[NaiveDate], settle: NaiveDate, clean: f64) -> f64 {
    solve(-10.0, 60.0, clean, |y| clean_from_yield(b, sched, settle, y))
}

/// Modified duration at a yield, by a central difference of one basis point.
pub fn mod_duration(b: &Bond, sched: &[NaiveDate], settle: NaiveDate, ytm: f64) -> f64 {
    let p = dirty_from_yield(b, sched, settle, ytm);
    if p <= 0.0 {
        return 0.0;
    }
    (dirty_from_yield(b, sched, settle, ytm - 0.01) - dirty_from_yield(b, sched, settle, ytm + 0.01)) / (2.0 * 0.0001) / p
}

/// The bond's remaining cash flows after `d` per 100, with every coupon at `rate`.
pub fn flows_after(b: &Bond, sched: &[NaiveDate], d: NaiveDate, rate: f64) -> Vec<(NaiveDate, f64)> {
    let mut out = Vec::new();
    let first = sched.partition_point(|x| *x <= d);
    let mut p = if first == 0 { b.issue_date } else { sched[first - 1] };
    for x in &sched[first..] {
        let mut cf = coupon_cash(b.day_count, rate, b.coupon_freq, p, *x);
        if *x == b.maturity {
            cf += 100.0;
        }
        out.push((*x, cf));
        p = *x;
    }
    out
}

/// The effective interest rate (percent a year, annual compounding on ACT/365) that discounts
/// `flows` back to `value` on `d`.
pub fn eir_of(d: NaiveDate, value: f64, flows: &[(NaiveDate, f64)]) -> f64 {
    if flows.is_empty() || value <= 0.0 {
        return 0.0;
    }
    solve(-20.0, 80.0, value, |e| {
        let base = 1.0 + e / 100.0;
        flows.iter().map(|(t, cf)| cf / base.powf((*t - d).num_days() as f64 / 365.0)).sum()
    })
}

/// The effective interest rate of a purchase: settles at `settle` for `clean` plus the accrued
/// interest, then receives the bond's flows at `rate` (its coupon; a floater's current coupon).
pub fn purchase_eir(b: &Bond, settle: NaiveDate, clean: f64, rate: f64) -> f64 {
    let sched = schedule(b);
    let paid = clean + accrued(b, &sched, settle, rate);
    eir_of(settle, paid, &flows_after(b, &sched, settle, rate))
}
