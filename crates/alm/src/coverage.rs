//! What each ALM lever applies to in a snapshot, in the reporting currency, so a page can show a
//! lever that has nothing to act on (a run-off rate for a class the bank does not hold, a factor for
//! an empty maturity band) instead of leaving it to look broken. Keys are the parameter's path as
//! the pages write it: `lcr.outflow_rates.FI_WHOLESALE`, `nsfr.asf.FI.1`, `nii.betas.SAVINGS`.
//! The LCR and NSFR amounts are read from those calculations' own lines, so they count exactly
//! what the ratios count (the 30-day window, the maturity bands).

use std::collections::BTreeMap;

use chrono::NaiveDate;
use domain::{AlmParams, BsPosition, BsSide, RateType};

use crate::bucket::{NSFR_BANDS, nsfr_band};
use crate::lcr::Spots;

pub fn compute(positions: &[BsPosition], params: &AlmParams, spots: &Spots, as_of: NaiveDate) -> BTreeMap<String, f64> {
    let fx = |ccy: &str| if ccy == params.reporting_ccy { 1.0 } else { spots.get(ccy).copied().unwrap_or(1.0) };
    let mut out: BTreeMap<String, f64> = BTreeMap::new();
    let mut add = |key: String, amount: f64| *out.entry(key).or_default() += amount;

    // Every lever the parameters hold starts at zero, so an empty one is reported as such.
    for k in params.lcr.outflow_rates.keys() {
        add(format!("lcr.outflow_rates.{k}"), 0.0);
    }
    for k in params.lcr.inflow_rates.keys() {
        add(format!("lcr.inflow_rates.{k}"), 0.0);
    }
    for k in params.lcr.haircuts.keys() {
        add(format!("lcr.haircuts.{k}"), 0.0);
        add(format!("nsfr.hqla_rsf.{k}"), 0.0);
    }
    for k in params.nsfr.asf.keys() {
        for i in 0..3 {
            add(format!("nsfr.asf.{k}.{i}"), 0.0);
        }
    }
    for k in params.nsfr.rsf.keys() {
        for i in 0..3 {
            add(format!("nsfr.rsf.{k}.{i}"), 0.0);
        }
    }
    for k in params.liquidity_gap.stable_share_pct.keys() {
        add(format!("liquidity_gap.stable_share_pct.{k}"), 0.0);
    }
    for k in params.gap.nmd_behavioural_months.keys() {
        add(format!("gap.nmd_behavioural_months.{k}"), 0.0);
    }
    for k in params.nii.betas.keys() {
        add(format!("nii.betas.{k}"), 0.0);
    }
    for k in ["nsfr.encumbered_rsf_pct", "nsfr.off_balance_rsf_pct", "liquidity_gap.commitment_draw_pct", "nii.default_beta", "lcr.cap_l2_total_pct", "lcr.cap_l2b_pct", "lcr.inflow_cap_pct"] {
        add(k.into(), 0.0);
    }

    // LCR: the classes as the ratio counts them, in the reporting currency.
    let lcr = crate::lcr::compute(positions, params, spots, as_of, None);
    for l in &lcr.total.outflows {
        add(format!("lcr.outflow_rates.{}", l.class), l.amount);
    }
    for l in &lcr.total.inflows {
        add(format!("lcr.inflow_rates.{}", l.class), l.amount);
    }
    add("lcr.inflow_cap_pct".into(), lcr.total.total_inflows);

    // NSFR: by class and band from its lines.
    let nsfr = crate::nsfr::compute(positions, params, spots, as_of, None);
    let band = |label: &str| NSFR_BANDS.iter().position(|b| *b == label).unwrap_or(0);
    for l in &nsfr.asf_lines {
        add(format!("nsfr.asf.{}.{}", l.class, band(&l.bucket)), l.amount);
    }
    for l in &nsfr.rsf_lines {
        if let Some(level) = l.class.strip_prefix("HQLA ") {
            add(format!("nsfr.hqla_rsf.{level}"), l.amount);
        } else if l.class == "OFF_BALANCE" {
            add("nsfr.off_balance_rsf_pct".into(), l.amount);
        } else {
            add(format!("nsfr.rsf.{}.{}", l.class, band(&l.bucket)), l.amount);
        }
    }

    let mut l2 = 0.0;
    let mut l2b = 0.0;
    for p in positions {
        let amount = p.balance * fx(&p.ccy);
        match p.side {
            BsSide::Asset => {
                if let Some(level) = p.hqla_level
                    && !p.encumbered
                {
                    add(format!("lcr.haircuts.{}", level.as_str()), amount);
                    if level.as_str() != "L1" {
                        l2 += amount;
                    }
                    if level.as_str() == "L2B" {
                        l2b += amount;
                    }
                }
                if p.encumbered && nsfr_band(p.maturity.map(|m| (m - as_of).num_days())) == 2 {
                    add("nsfr.encumbered_rsf_pct".into(), amount);
                }
            }
            BsSide::Liability => {
                if p.maturity.is_none() && p.balance > 0.0 && !matches!(p.book.as_str(), "EQUITY" | "OTHER" | "CASH") {
                    add(format!("liquidity_gap.stable_share_pct.{}", p.segment), amount);
                }
            }
            BsSide::OffBalance => add("liquidity_gap.commitment_draw_pct".into(), p.undrawn * fx(&p.ccy)),
        }
        if p.rate_type == RateType::Admin {
            add(format!("gap.nmd_behavioural_months.{}", p.product), amount);
            let both = format!("{}.{}", p.product, p.segment);
            if params.nii.betas.contains_key(&both) {
                add(format!("nii.betas.{both}"), amount);
            } else if params.nii.betas.contains_key(&p.product) {
                add(format!("nii.betas.{}", p.product), amount);
            } else {
                add("nii.default_beta".into(), amount);
            }
        }
    }
    add("lcr.cap_l2_total_pct".into(), l2);
    add("lcr.cap_l2b_pct".into(), l2b);
    out
}
