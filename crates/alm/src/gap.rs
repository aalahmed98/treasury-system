//! Repricing gap: rate-sensitive assets less liabilities per time bucket, by the date each
//! cohort next reprices (floating at its reset, fixed at each principal date, administered
//! deposits over their behavioural life), with the bank's cumulative limits.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use domain::{AlmParams, BsPosition, BsSide, GapBucket, GapCurrency, GapResult, RateType};

use crate::bucket::{GAP_BUCKETS, gap_bucket};
use crate::lcr::Spots;
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
            let mut out: BTreeMap<usize, f64> = BTreeMap::new();
            for m in 0..months {
                *out.entry(gap_bucket(30 * m + 15)).or_default() += slice;
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
        if pos.side == BsSide::Asset {
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
    }
}
