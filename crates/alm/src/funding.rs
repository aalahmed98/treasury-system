//! Funding concentration: the largest depositor groups as a share of deposits and the share of
//! funding that is wholesale, against the bank's limits.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use domain::{AlmParams, BsPosition, BsSide, FundingGroup, FundingResult, NiiBreakdown};

use crate::lcr::Spots;

/// A depositor group's balance in its own currency.
pub struct DepositorGroup {
    pub name: String,
    pub segment: String,
    pub ccy: String,
    pub balance: f64,
}

pub fn compute(positions: &[BsPosition], groups: &[DepositorGroup], params: &AlmParams, spots: &Spots, as_of: NaiveDate, params_version: Option<i32>) -> FundingResult {
    let fx = |ccy: &str| if ccy == params.reporting_ccy { 1.0 } else { spots.get(ccy).copied().unwrap_or(1.0) };
    let mut total_deposits = 0.0;
    let mut total_funding = 0.0;
    let mut wholesale = 0.0;
    let mut by_segment: BTreeMap<String, f64> = BTreeMap::new();
    for pos in positions.iter().filter(|p| p.side == BsSide::Liability) {
        let v = pos.balance * fx(&pos.ccy);
        if pos.book == "EQUITY" {
            continue;
        }
        total_funding += v;
        if pos.book == "DEPOSITS" {
            total_deposits += v;
            *by_segment.entry(pos.segment.clone()).or_default() += v;
            if pos.segment == "FI" {
                wholesale += v;
            }
        } else if pos.book == "INTERBANK" || pos.book == "TERM_FUNDING" {
            wholesale += v;
        }
    }
    let mut merged: BTreeMap<String, (String, f64)> = BTreeMap::new();
    for g in groups {
        let e = merged.entry(g.name.clone()).or_insert((g.segment.clone(), 0.0));
        e.1 += g.balance * fx(&g.ccy);
    }
    let mut list: Vec<FundingGroup> = merged
        .into_iter()
        .map(|(name, (segment, balance))| FundingGroup { name, segment, balance, share_pct: if total_deposits > 0.0 { balance / total_deposits * 100.0 } else { 0.0 } })
        .collect();
    list.sort_by(|a, b| b.balance.partial_cmp(&a.balance).unwrap_or(std::cmp::Ordering::Equal));
    let top1_pct = list.first().map(|g| g.share_pct).unwrap_or(0.0);
    let top10_pct = list.iter().take(10).map(|g| g.share_pct).sum();
    let wholesale_pct = if total_funding > 0.0 { wholesale / total_funding * 100.0 } else { 0.0 };
    let f = &params.funding;
    let mut breaches = Vec::new();
    if top1_pct > f.top1_limit_pct {
        breaches.push(format!("largest depositor {top1_pct:.1}% exceeds {:.1}%", f.top1_limit_pct));
    }
    if top10_pct > f.top10_limit_pct {
        breaches.push(format!("ten largest depositors {top10_pct:.1}% exceed {:.1}%", f.top10_limit_pct));
    }
    if wholesale_pct > f.wholesale_share_limit_pct {
        breaches.push(format!("wholesale funding {wholesale_pct:.1}% exceeds {:.1}%", f.wholesale_share_limit_pct));
    }
    FundingResult {
        as_of,
        params_version,
        reporting_ccy: params.reporting_ccy.clone(),
        total_deposits,
        total_funding,
        wholesale_funding: wholesale,
        top1_pct,
        top10_pct,
        wholesale_pct,
        top1_limit_pct: f.top1_limit_pct,
        top10_limit_pct: f.top10_limit_pct,
        wholesale_limit_pct: f.wholesale_share_limit_pct,
        breaches,
        groups: list,
        by_segment: by_segment.into_iter().map(|(key, nii)| NiiBreakdown { key, nii }).collect(),
    }
}
