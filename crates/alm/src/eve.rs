//! Economic value of equity: the present value of every rate-sensitive cash flow on the base and
//! the shocked curve. Fixed cohorts contribute their schedule, floating cohorts par plus accrued
//! at the next reset, administered deposits their behavioural run-off; equity and other
//! non-sensitive items are left out.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use domain::{AlmParams, BsPosition, BsSide, EveCurrency, EveResult, RateScenario, RateType};

use crate::curve::{Curve, scenario_curve};
use crate::lcr::Spots;
use crate::nii::Curves;
use crate::schedule::{add_months, cash_flows};

/// (years, amount) flows of a cohort in its own currency.
pub fn flows(pos: &BsPosition, params: &AlmParams, as_of: NaiveDate) -> Vec<(f64, f64)> {
    let years = |d: NaiveDate| (d - as_of).num_days().max(0) as f64 / 365.0;
    match pos.rate_type {
        RateType::NotSensitive => Vec::new(),
        RateType::Fixed => cash_flows(pos, as_of).into_iter().map(|f| (years(f.date), f.total())).collect(),
        RateType::Float => {
            let reset = pos.next_repricing.or(pos.maturity);
            match reset {
                Some(r) if r > as_of => {
                    let accrued = pos.balance * pos.rate / 100.0 * (r - as_of).num_days() as f64 / pos.day_count.basis();
                    vec![(years(r), pos.balance + accrued)]
                }
                _ => vec![(0.0, pos.balance)],
            }
        }
        RateType::Admin => {
            let months = params
                .gap
                .nmd_behavioural_months
                .get(&pos.product)
                .copied()
                .or(pos.behavioural_months.map(|m| m as f64))
                .unwrap_or(1.0)
                .max(1.0)
                .round() as i64;
            let slice = pos.balance / months as f64;
            (0..months).map(|m| (years(add_months(as_of, m)) + 0.5 / 12.0, slice)).collect()
        }
    }
}

fn pv(flows: &[(f64, f64)], curve: &Curve) -> f64 {
    flows.iter().map(|(t, a)| a * curve.df(*t)).sum()
}

#[allow(clippy::too_many_arguments)]
pub fn compute(
    positions: &[BsPosition],
    params: &AlmParams,
    spots: &Spots,
    curves: &Curves,
    as_of: NaiveDate,
    scenario: RateScenario,
    custom_bp: f64,
    params_version: Option<i32>,
) -> EveResult {
    let fx = |ccy: &str| if ccy == params.reporting_ccy { 1.0 } else { spots.get(ccy).copied().unwrap_or(1.0) };
    let fallback = Curve::flat("", 0.0);
    let mut per: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    for pos in positions {
        if pos.side == BsSide::OffBalance {
            continue;
        }
        let base = curves.get(&pos.ccy).unwrap_or(&fallback);
        let sh = scenario_curve(base, params, scenario, custom_bp);
        let fl = flows(pos, params, as_of);
        let sign = if pos.side == BsSide::Asset { 1.0 } else { -1.0 };
        let e = per.entry(pos.ccy.clone()).or_insert((0.0, 0.0));
        e.0 += sign * pv(&fl, base) * fx(&pos.ccy);
        e.1 += sign * pv(&fl, &sh) * fx(&pos.ccy);
    }
    let by_ccy: Vec<EveCurrency> = per.into_iter().map(|(ccy, (base, shocked))| EveCurrency { ccy, base, shocked, delta: shocked - base }).collect();
    let base: f64 = by_ccy.iter().map(|c| c.base).sum();
    let shocked: f64 = by_ccy.iter().map(|c| c.shocked).sum();
    let losses: f64 = by_ccy.iter().map(|c| (-c.delta).max(0.0)).sum();
    let gains: f64 = by_ccy.iter().map(|c| c.delta.max(0.0)).sum();
    EveResult {
        as_of,
        params_version,
        reporting_ccy: params.reporting_ccy.clone(),
        scenario,
        base,
        shocked,
        delta: shocked - base,
        basel_loss: (losses - 0.5 * gains).max(0.0),
        by_ccy,
    }
}
