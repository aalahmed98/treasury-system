//! Behaviour that shortens fixed-rate contracts: borrowers prepaying loans and depositors breaking
//! term deposits. Each listed product has a yearly rate at which the outstanding balance leaves
//! early, and Basel-style multipliers speed it up or slow it down with the direction of a rate
//! move: loans prepay faster when rates fall, deposits are broken sooner when rates rise.
//!
//! This reaches the rates tools only (the gap, EVE and NII). The liquidity ratios and the
//! maturity ladder stay contractual and read `schedule::cash_flows` directly.

use chrono::NaiveDate;
use domain::{AlmParams, BsPosition, BsSide, RateType};

use crate::schedule::{Flow, add_months, cash_flows};

/// The yearly rate at which a cohort's balance leaves early, as a fraction: its product's, for a
/// fixed-rate asset or liability with a maturity. Zero for everything else.
pub fn speed(pos: &BsPosition, params: &AlmParams) -> f64 {
    if pos.rate_type != RateType::Fixed || pos.maturity.is_none() {
        return 0.0;
    }
    let pct = match pos.side {
        BsSide::Asset => params.behaviour.prepayment_pct.get(&pos.product),
        BsSide::Liability => params.behaviour.withdrawal_pct.get(&pos.product),
        BsSide::OffBalance => None,
    };
    (pct.copied().unwrap_or(0.0) / 100.0).clamp(0.0, 0.95)
}

/// How a rate move changes that speed: `move_pct` is the move at the cohort's remaining maturity,
/// in percent. Moves under half a basis point count as none.
pub fn multiplier(pos: &BsPosition, params: &AlmParams, move_pct: f64) -> f64 {
    let b = &params.behaviour;
    let m = match (pos.side, move_pct) {
        (_, d) if d.abs() < 0.005 => 1.0,
        (BsSide::Asset, d) if d > 0.0 => b.prepayment_when_rates_rise,
        (BsSide::Asset, _) => b.prepayment_when_rates_fall,
        (BsSide::Liability, d) if d > 0.0 => b.withdrawal_when_rates_rise,
        (BsSide::Liability, _) => b.withdrawal_when_rates_fall,
        (BsSide::OffBalance, _) => 1.0,
    };
    m.clamp(0.0, 5.0)
}

/// The share still outstanding `years` from now at a yearly rate of leaving.
pub fn surviving(yearly: f64, years: f64) -> f64 {
    (1.0 - yearly.clamp(0.0, 0.95)).powf(years.max(0.0))
}

/// The cohort's cash flows with early repayment at a yearly rate: each contractual payment is cut
/// to the share still outstanding, and what leaves early is paid month by month before it. The
/// principal still adds up to the balance. At a rate of zero these are the contractual flows.
pub fn flows(pos: &BsPosition, as_of: NaiveDate, yearly: f64) -> Vec<Flow> {
    let contractual = cash_flows(pos, as_of);
    let Some(last) = contractual.last().map(|f| f.date) else { return contractual };
    if yearly <= 0.0 {
        return contractual;
    }
    // Every month end and every contractual date, in order.
    let mut dates: Vec<NaiveDate> = contractual.iter().map(|f| f.date).collect();
    let mut m = 1;
    loop {
        let d = add_months(as_of, m);
        if d >= last || m > 1200 {
            break;
        }
        dates.push(d);
        m += 1;
    }
    dates.sort();
    dates.dedup();

    let years = |d: NaiveDate| (d - as_of).num_days().max(0) as f64 / 365.0;
    let mut out = Vec::with_capacity(dates.len() + contractual.len());
    let mut before = 1.0;
    for d in dates {
        let now = surviving(yearly, years(d));
        // What was contractually due on or after this date was outstanding through the interval.
        let outstanding: f64 = contractual.iter().filter(|f| f.date >= d).map(|f| f.principal).sum();
        let early = outstanding * (before - now);
        if early > 0.0 {
            out.push(Flow { date: d, principal: early, interest: 0.0 });
        }
        if let Some(f) = contractual.iter().find(|f| f.date == d) {
            out.push(Flow { date: d, principal: f.principal * now, interest: f.interest * now });
        }
        before = now;
    }
    out
}
