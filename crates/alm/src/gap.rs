//! Repricing gap: rate-sensitive assets less liabilities per time bucket, by the date each
//! cohort next reprices (floating at its reset, fixed at each principal date, administered
//! deposits by their behavioural profile: the non-core share overnight, the core over its life),
//! with the bank's cumulative limits.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use domain::{AlmParams, BsPosition, BsSide, GapBucket, GapCurrency, GapResult, RateScenario, RateType};

use crate::bucket::{GAP_BUCKETS, gap_bucket};
use crate::curve::{Curve, CustomShock, scenario_curve, shock_label};
use crate::hedge::HEDGE_BOOK;
use crate::lcr::Spots;
use crate::nii::Curves;
use crate::nmd;
use crate::schedule::cash_flows;

/// Balance slotted per bucket for one cohort, in its own currency.
pub fn slots(pos: &BsPosition, params: &AlmParams, as_of: NaiveDate) -> Vec<(usize, f64)> {
    match pos.rate_type {
        RateType::NotSensitive => Vec::new(),
        RateType::Float => {
            let days = pos.next_repricing.or(pos.maturity).map(|d| (d - as_of).num_days()).unwrap_or(0);
            vec![(gap_bucket(days.max(1)), pos.balance)]
        }
        RateType::Fixed => {
            let flows = cash_flows(pos, as_of);
            if flows.is_empty() {
                return vec![(0, pos.balance)];
            }
            let mut out: BTreeMap<usize, f64> = BTreeMap::new();
            for f in flows.iter().filter(|f| f.principal > 0.0) {
                *out.entry(gap_bucket((f.date - as_of).num_days().max(1))).or_default() += f.principal;
            }
            out.into_iter().collect()
        }
        RateType::Admin => {
            let mut out: BTreeMap<usize, f64> = BTreeMap::new();
            for (month, amount) in nmd::profile(pos, params) {
                let days = month.map(|m| 30 * m + 15).unwrap_or(1);
                *out.entry(gap_bucket(days)).or_default() += amount;
            }
            out.into_iter().collect()
        }
    }
}

fn buckets_from(totals: &[(f64, f64); 9], total_assets: f64, params: &AlmParams) -> Vec<GapBucket> {
    let mut cumulative = 0.0;
    GAP_BUCKETS
        .iter()
        .enumerate()
        .map(|(i, (label, _))| {
            let (assets, liabilities) = totals[i];
            let gap = assets - liabilities;
            cumulative += gap;
            let pct = if total_assets > 0.0 { cumulative / total_assets * 100.0 } else { 0.0 };
            let limit = params.gap.cumulative_gap_limit_pct_assets.get(*label).copied();
            GapBucket {
                label: (*label).into(),
                assets,
                liabilities,
                gap,
                cumulative,
                cumulative_pct_assets: pct,
                limit_pct: limit,
                breach: limit.is_some_and(|l| pct.abs() > l),
                shock_bp: None,
                nii_effect: None,
            }
        })
        .collect()
}

pub fn compute(positions: &[BsPosition], params: &AlmParams, spots: &Spots, as_of: NaiveDate, params_version: Option<i32>) -> GapResult {
    let fx = |ccy: &str| if ccy == params.reporting_ccy { 1.0 } else { spots.get(ccy).copied().unwrap_or(1.0) };
    let mut total = [(0.0, 0.0); 9];
    let mut per_ccy: BTreeMap<String, [(f64, f64); 9]> = BTreeMap::new();
    let mut total_assets = 0.0;
    for pos in positions {
        let rate = fx(&pos.ccy);
        // A trial hedge's legs are notionals: they reprice, but they are not assets of the bank.
        if pos.side == BsSide::Asset && pos.book != HEDGE_BOOK {
            total_assets += pos.balance * rate;
        }
        if pos.side == BsSide::OffBalance {
            continue;
        }
        let entry = per_ccy.entry(pos.ccy.clone()).or_insert([(0.0, 0.0); 9]);
        for (bucket, amount) in slots(pos, params, as_of) {
            let v = amount * rate;
            match pos.side {
                BsSide::Asset => {
                    total[bucket].0 += v;
                    entry[bucket].0 += v;
                }
                BsSide::Liability => {
                    total[bucket].1 += v;
                    entry[bucket].1 += v;
                }
                BsSide::OffBalance => {}
            }
        }
    }
    GapResult {
        as_of,
        params_version,
        reporting_ccy: params.reporting_ccy.clone(),
        total_assets,
        buckets: buckets_from(&total, total_assets, params),
        by_ccy: per_ccy.into_iter().map(|(ccy, t)| GapCurrency { ccy, buckets: buckets_from(&t, total_assets, params) }).collect(),
        scenario: RateScenario::Base,
        shock_label: None,
        nii_effect_12m: None,
    }
}

/// The middle of each bucket in years: where its balances are taken to reprice.
pub const BUCKET_MID_YEARS: [f64; 9] = [0.5 / 365.0, 4.0 / 365.0, 18.5 / 365.0, 60.5 / 365.0, 136.5 / 365.0, 273.5 / 365.0, 1.5, 3.5, 7.5];

/// The gap under a rate scenario. The balances do not move; what the scenario adds is the classic
/// gap estimate of its effect on the next twelve months' NII: per bucket and currency, the gap
/// times the rate move at that tenor times the part of the year left once the bucket has
/// repriced. It takes every balance to pass the move on in full, so it overstates what deposits
/// with a low beta give up; the NII projection is the fuller answer.
pub fn with_shock(mut r: GapResult, params: &AlmParams, curves: &Curves, scenario: RateScenario, custom: impl Into<CustomShock>) -> GapResult {
    let custom = custom.into();
    r.scenario = scenario;
    r.shock_label = shock_label(scenario, custom);
    if scenario == RateScenario::Base {
        return r;
    }
    let fallback = Curve::flat("", 0.0);
    let n = GAP_BUCKETS.len();
    let mut total = vec![0.0; n];
    let mut moves: Vec<Vec<f64>> = vec![Vec::new(); n];
    for c in &mut r.by_ccy {
        let base = curves.get(&c.ccy).unwrap_or(&fallback);
        let shocked = scenario_curve(base, params, scenario, custom);
        for (i, b) in c.buckets.iter_mut().enumerate() {
            let t = BUCKET_MID_YEARS[i];
            let bp = (shocked.rate(t) - base.rate(t)) * 100.0;
            let effect = b.gap * bp / 10_000.0 * (1.0 - t).max(0.0);
            b.shock_bp = Some(bp);
            b.nii_effect = Some(effect);
            total[i] += effect;
            moves[i].push(bp);
        }
    }
    for (i, b) in r.buckets.iter_mut().enumerate() {
        b.nii_effect = Some(total[i]);
        b.shock_bp = match moves[i].split_first() {
            Some((first, rest)) if rest.iter().all(|x| (x - first).abs() < 0.01) => Some(*first),
            _ => None,
        };
    }
    r.nii_effect_12m = Some(total.iter().sum());
    r
}
