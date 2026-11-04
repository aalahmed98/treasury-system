//! Contractual cash flows of a cohort: principal by amortisation profile and payment frequency,
//! interest on the outstanding balance at the cohort's current rate. Used by the LCR inflows, the
//! survival horizon, the repricing gap and the economic value calculations.

use chrono::{Months, NaiveDate};
use domain::{Amort, BsPosition};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flow {
    pub date: NaiveDate,
    pub principal: f64,
    pub interest: f64,
}

impl Flow {
    pub fn total(&self) -> f64 {
        self.principal + self.interest
    }
}

pub fn add_months(d: NaiveDate, n: i64) -> NaiveDate {
    if n >= 0 { d.checked_add_months(Months::new(n as u32)).unwrap_or(d) } else { d.checked_sub_months(Months::new((-n) as u32)).unwrap_or(d) }
}

/// Flows strictly after `as_of` up to and including maturity. Non-maturing cohorts have none.
/// A cohort with no payment frequency pays everything at maturity.
pub fn cash_flows(p: &BsPosition, as_of: NaiveDate) -> Vec<Flow> {
    let Some(maturity) = p.maturity else { return Vec::new() };
    if maturity <= as_of || p.balance <= 0.0 {
        return Vec::new();
    }
    let basis = p.day_count.basis();
    let annual = p.rate / 100.0;
    let interest_for = |balance: f64, from: NaiveDate, to: NaiveDate| balance * annual * (to - from).num_days().max(0) as f64 / basis;

    let Some(step) = p.pay_months.filter(|m| *m > 0) else {
        return vec![Flow { date: maturity, principal: p.balance, interest: interest_for(p.balance, as_of, maturity) }];
    };

    // Payment dates counted back from maturity, keeping those after as_of.
    let mut dates = Vec::new();
    let mut k = 0i64;
    loop {
        let d = add_months(maturity, -k * step as i64);
        if d <= as_of {
            break;
        }
        dates.push(d);
        k += 1;
        if k > 1200 {
            break;
        }
    }
    dates.reverse();
    let n = dates.len();
    if n == 0 {
        return Vec::new();
    }

    let mut out = Vec::with_capacity(n);
    let mut outstanding = p.balance;
    let mut prev = as_of;
    // Level payment for an annuity over the remaining periods at the periodic rate.
    let periodic = annual * step as f64 / 12.0;
    let annuity_payment = if periodic.abs() < 1e-12 { p.balance / n as f64 } else { p.balance * periodic / (1.0 - (1.0 + periodic).powi(-(n as i32))) };
    for (i, d) in dates.iter().enumerate() {
        let interest = interest_for(outstanding, prev, *d);
        let last = i + 1 == n;
        let principal = match p.amort {
            Amort::Bullet => {
                if last { outstanding } else { 0.0 }
            }
            Amort::Linear => {
                if last { outstanding } else { p.balance / n as f64 }
            }
            Amort::Annuity => {
                if last { outstanding } else { (annuity_payment - outstanding * periodic).clamp(0.0, outstanding) }
            }
        };
        out.push(Flow { date: *d, principal, interest });
        outstanding -= principal;
        prev = *d;
    }
    out
}

/// Principal and interest due within `days` calendar days after `as_of`.
pub fn due_within(p: &BsPosition, as_of: NaiveDate, days: i64) -> f64 {
    let limit = as_of + chrono::Duration::days(days);
    cash_flows(p, as_of).iter().filter(|f| f.date <= limit).map(Flow::total).sum()
}
