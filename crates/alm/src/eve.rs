//! Economic value of equity: the present value of every rate-sensitive cash flow on the base and
//! the shocked curve. Fixed cohorts contribute their schedule, floating cohorts par plus accrued
//! at the next reset, administered deposits their behavioural run-off; equity and other
//! non-sensitive items are left out. Beside the full revaluation it reports the duration gap and
//! what that predicts for a parallel move, so the two can be read against each other, and the
//! sensitivity bucket by bucket (key rates), which gives an estimate for any shape of shock.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use domain::{AlmParams, BsPosition, BsSide, DurationGap, EveBucket, EveCurrency, EveResult, KeyRate, RateScenario, RateType};

use crate::bucket::GAP_BUCKETS;
use crate::curve::{Curve, CustomShock, bucket_of_years, scenario_curve, shock_for, shock_label};
use crate::lcr::Spots;
use crate::nii::Curves;
use crate::nmd;
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
        RateType::Admin => nmd::profile(pos, params)
            .into_iter()
            .map(|(month, amount)| match month {
                Some(m) => (years(add_months(as_of, m)) + 0.5 / 12.0, amount),
                None => (1.0 / 365.0, amount),
            })
            .collect(),
    }
}

/// The parallel move a scenario makes in a currency, in basis points; none when it is not parallel.
fn parallel_bp(params: &AlmParams, ccy: &str, scenario: RateScenario, custom: CustomShock) -> Option<f64> {
    match scenario {
        RateScenario::ParallelUp => Some(shock_for(params, ccy).parallel),
        RateScenario::ParallelDown => Some(-shock_for(params, ccy).parallel),
        RateScenario::Custom if custom.is_parallel() => Some(custom.bp),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn compute(
    positions: &[BsPosition],
    params: &AlmParams,
    spots: &Spots,
    curves: &Curves,
    as_of: NaiveDate,
    scenario: RateScenario,
    custom_bp: impl Into<CustomShock>,
    params_version: Option<i32>,
) -> EveResult {
    let custom = custom_bp.into();
    let fx = |ccy: &str| if ccy == params.reporting_ccy { 1.0 } else { spots.get(ccy).copied().unwrap_or(1.0) };
    let fallback = Curve::flat("", 0.0);
    let mut per: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    // Per bucket: assets and liabilities at base, then the net at base and shocked.
    let mut buckets = vec![(0.0f64, 0.0f64, 0.0f64, 0.0f64); GAP_BUCKETS.len()];
    // Present value and time-weighted present value per side, on the base curve.
    let (mut pv_a, mut pvt_a, mut pv_l, mut pvt_l) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    // For the yield: the base rate averaged over the flows, weighted as duration weights them.
    let (mut rate_weight, mut weight) = (0.0f64, 0.0f64);
    // Per currency: assets' less liabilities' time-weighted present value.
    let mut dollar_duration: BTreeMap<String, f64> = BTreeMap::new();
    // Per bucket: the first-order change in value for one basis point there, assets then
    // liabilities (as it reaches EVE), and the scenario's first-order effect.
    let mut key = vec![(0.0f64, 0.0f64, 0.0f64); GAP_BUCKETS.len()];
    for pos in positions {
        if pos.side == BsSide::OffBalance {
            continue;
        }
        let base = curves.get(&pos.ccy).unwrap_or(&fallback);
        let sh = scenario_curve(base, params, scenario, custom);
        let sign = if pos.side == BsSide::Asset { 1.0 } else { -1.0 };
        let rate = fx(&pos.ccy);
        let e = per.entry(pos.ccy.clone()).or_insert((0.0, 0.0));
        for (t, amount) in flows(pos, params, as_of) {
            let b = amount * base.df(t) * rate;
            let s = amount * sh.df(t) * rate;
            e.0 += sign * b;
            e.1 += sign * s;
            let slot = &mut buckets[bucket_of_years(t)];
            if pos.side == BsSide::Asset {
                slot.0 += b;
                pv_a += b;
                pvt_a += b * t;
            } else {
                slot.1 += b;
                pv_l += b;
                pvt_l += b * t;
            }
            slot.2 += sign * b;
            slot.3 += sign * s;
            rate_weight += b.abs() * t * base.rate(t);
            weight += b.abs() * t;
            *dollar_duration.entry(pos.ccy.clone()).or_default() += sign * b * t;
            // dPV/dr with yearly compounding is -t * PV / (1 + r); one basis point is 1e-4.
            let per_unit = -sign * b * t / (1.0 + base.rate(t) / 100.0);
            let k = &mut key[bucket_of_years(t)];
            if pos.side == BsSide::Asset {
                k.0 += per_unit * 1e-4;
            } else {
                k.1 += per_unit * 1e-4;
            }
            k.2 += per_unit * (sh.rate(t) - base.rate(t)) / 100.0;
        }
    }
    let by_ccy: Vec<EveCurrency> = per.into_iter().map(|(ccy, (base, shocked))| EveCurrency { ccy, base, shocked, delta: shocked - base }).collect();
    let base: f64 = by_ccy.iter().map(|c| c.base).sum();
    let shocked: f64 = by_ccy.iter().map(|c| c.shocked).sum();
    let losses: f64 = by_ccy.iter().map(|c| (-c.delta).max(0.0)).sum();
    let gains: f64 = by_ccy.iter().map(|c| c.delta.max(0.0)).sum();
    let basel_loss = (losses - 0.5 * gains).max(0.0);

    let d_assets = if pv_a.abs() > 1e-9 { pvt_a / pv_a } else { 0.0 };
    let d_liabilities = if pv_l.abs() > 1e-9 { pvt_l / pv_l } else { 0.0 };
    let gap_years = if pv_a.abs() > 1e-9 { d_assets - pv_l / pv_a * d_liabilities } else { 0.0 };
    let yield_pct = if weight > 1e-9 { rate_weight / weight } else { 0.0 };
    let moves: Vec<(f64, f64)> = dollar_duration.iter().filter_map(|(ccy, dd)| parallel_bp(params, ccy, scenario, custom).map(|bp| (*dd, bp))).collect();
    let parallel = !moves.is_empty() && moves.len() == dollar_duration.len();
    let estimate = parallel.then(|| -moves.iter().map(|(dd, bp)| dd * bp / 10_000.0).sum::<f64>() / (1.0 + yield_pct / 100.0));
    let shock_bp = match moves.split_first() {
        Some(((_, first), rest)) if parallel && rest.iter().all(|(_, bp)| (bp - first).abs() < 1e-9) => Some(*first),
        _ => None,
    };

    let shocked_run = scenario != RateScenario::Base;
    let key_rates: Vec<KeyRate> = GAP_BUCKETS
        .iter()
        .zip(&key)
        .map(|((label, _), (assets, liabilities, estimate))| KeyRate { label: (*label).into(), assets: *assets, liabilities: *liabilities, net: assets + liabilities, estimate: shocked_run.then_some(*estimate) })
        .collect();
    let key_rate_estimate = shocked_run.then(|| key.iter().map(|k| k.2).sum::<f64>());

    let tier1 = params.tier1_capital;
    let loss_pct_tier1 = (tier1 > 0.0).then(|| basel_loss / tier1 * 100.0);
    EveResult {
        as_of,
        params_version,
        reporting_ccy: params.reporting_ccy.clone(),
        scenario,
        base,
        shocked,
        delta: shocked - base,
        basel_loss,
        by_ccy,
        shock_label: shock_label(scenario, custom),
        by_bucket: GAP_BUCKETS
            .iter()
            .zip(buckets)
            .map(|((label, _), (assets, liabilities, base, shocked))| EveBucket { label: (*label).into(), assets, liabilities, base, shocked, delta: shocked - base })
            .collect(),
        duration: DurationGap { assets: pv_a, liabilities: pv_l, d_assets, d_liabilities, gap_years, yield_pct, shock_bp, estimate },
        key_rates,
        key_rate_estimate,
        tier1_capital: tier1,
        loss_pct_tier1,
        limit_pct_tier1: params.gap.eve_limit_pct_tier1,
        breach: loss_pct_tier1.is_some_and(|p| p > params.gap.eve_limit_pct_tier1),
    }
}
