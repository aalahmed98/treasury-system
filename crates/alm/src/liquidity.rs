//! The liquidity maturity ladder: when cash comes in and goes out, per time bucket and currency,
//! contractually and under the bank's behavioural assumptions, with the cumulative gap against the
//! bank's floors. Principal only; interest is small against the buckets and is left out, as is
//! usual for the ladder.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use domain::{AlmParams, BsPosition, BsSide, LiquidityBucket, LiquidityCcy, LiquidityGapResult, LiquidityLadder};

use crate::bucket::{GAP_BUCKETS, gap_bucket};
use crate::lcr::Spots;
use crate::schedule::cash_flows;

const LAST: usize = GAP_BUCKETS.len() - 1;

/// Non-maturity balances: deposits with no term, overdrafts, cash, other assets, equity.
fn non_maturing(p: &BsPosition) -> bool {
    p.maturity.is_none()
}

/// Principal per bucket for one cohort, in its own currency: positive in, negative out.
pub fn slots(p: &BsPosition, params: &AlmParams, as_of: NaiveDate, behavioural: bool) -> Vec<(usize, f64)> {
    let sign = match p.side {
        BsSide::Asset => 1.0,
        BsSide::Liability => -1.0,
        BsSide::OffBalance => {
            // Undrawn commitments: nothing contractually; a share drawn within the month behaviourally.
            return if behavioural && p.undrawn > 0.0 { vec![(gap_bucket(30), -p.undrawn * params.liquidity_gap.commitment_draw_pct / 100.0)] } else { Vec::new() };
        }
    };
    if p.balance <= 0.0 {
        return Vec::new();
    }
    // Capital and other assets never fall due.
    if p.book == "EQUITY" || p.book == "OTHER" {
        return vec![(LAST, sign * p.balance)];
    }
    // Unencumbered liquid securities can be sold or repoed at once, after the LCR haircut.
    if behavioural
        && p.side == BsSide::Asset
        && !p.encumbered
        && let Some(level) = p.hqla_level
    {
        let haircut = params.lcr.haircuts.get(level.as_str()).copied().unwrap_or(0.0) / 100.0;
        return vec![(0, p.balance * (1.0 - haircut))];
    }
    if non_maturing(p) {
        if p.book == "CASH" {
            return vec![(0, sign * p.balance)];
        }
        if !behavioural {
            // Contractually, a current account or an overdraft is repayable on demand.
            return vec![(0, sign * p.balance)];
        }
        // Behaviourally: the stable share runs off evenly over the behavioural life, the rest
        // leaves (or, for an asset, comes back) overnight.
        let stable = if p.side == BsSide::Liability { params.liquidity_gap.stable_share_pct.get(&p.segment).copied().unwrap_or(50.0) / 100.0 } else { 1.0 };
        let months = params.gap.nmd_behavioural_months.get(&p.product).copied().or(p.behavioural_months.map(f64::from)).unwrap_or(12.0).max(1.0).round() as i64;
        let mut out: BTreeMap<usize, f64> = BTreeMap::new();
        *out.entry(0).or_default() += sign * p.balance * (1.0 - stable);
        let slice = p.balance * stable / months as f64;
        for m in 0..months {
            *out.entry(gap_bucket(30 * m + 15)).or_default() += sign * slice;
        }
        return out.into_iter().collect();
    }
    let mut out: BTreeMap<usize, f64> = BTreeMap::new();
    for f in cash_flows(p, as_of).iter().filter(|f| f.principal > 0.0) {
        *out.entry(gap_bucket((f.date - as_of).num_days().max(1))).or_default() += sign * f.principal;
    }
    out.into_iter().collect()
}

fn ladder(positions: &[BsPosition], params: &AlmParams, spots: &Spots, as_of: NaiveDate, behavioural: bool, total_assets: f64) -> LiquidityLadder {
    let fx = |ccy: &str| if ccy == params.reporting_ccy { 1.0 } else { spots.get(ccy).copied().unwrap_or(1.0) };
    let n = GAP_BUCKETS.len();
    let mut inflows = vec![0.0; n];
    let mut outflows = vec![0.0; n];
    let mut by_ccy: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for p in positions {
        let rate = fx(&p.ccy);
        for (b, amount) in slots(p, params, as_of, behavioural) {
            let v = amount * rate;
            if v >= 0.0 { inflows[b] += v } else { outflows[b] -= v }
            by_ccy.entry(p.ccy.clone()).or_insert_with(|| vec![0.0; n])[b] += v;
        }
    }
    let mut cumulative = 0.0;
    let mut first_negative = None;
    let buckets = GAP_BUCKETS
        .iter()
        .enumerate()
        .map(|(i, (label, _))| {
            let gap = inflows[i] - outflows[i];
            cumulative += gap;
            if cumulative < 0.0 && first_negative.is_none() {
                first_negative = Some((*label).to_string());
            }
            let pct = if total_assets > 0.0 { cumulative / total_assets * 100.0 } else { 0.0 };
            // The bank's floors apply to the behavioural ladder; the contractual one is shown as is.
            let floor = if behavioural { params.liquidity_gap.cumulative_floor_pct_assets.get(*label).map(|f| -f.abs()) } else { None };
            LiquidityBucket {
                label: (*label).into(),
                inflows: inflows[i],
                outflows: outflows[i],
                gap,
                cumulative,
                cumulative_pct_assets: pct,
                floor_pct: floor,
                breach: floor.is_some_and(|f| pct < f),
            }
        })
        .collect();
    let by_ccy = by_ccy
        .into_iter()
        .map(|(ccy, gaps)| {
            let mut run = 0.0;
            let cumulative = gaps
                .iter()
                .map(|g| {
                    run += g;
                    run
                })
                .collect();
            LiquidityCcy { ccy, gaps, cumulative }
        })
        .collect();
    LiquidityLadder { buckets, by_ccy, first_negative }
}

pub fn compute(positions: &[BsPosition], params: &AlmParams, spots: &Spots, as_of: NaiveDate, params_version: Option<i32>) -> LiquidityGapResult {
    let fx = |ccy: &str| if ccy == params.reporting_ccy { 1.0 } else { spots.get(ccy).copied().unwrap_or(1.0) };
    let total_assets: f64 = positions.iter().filter(|p| p.side == BsSide::Asset).map(|p| p.balance * fx(&p.ccy)).sum();
    let contractual = ladder(positions, params, spots, as_of, false, total_assets);
    let behavioural = ladder(positions, params, spots, as_of, true, total_assets);
    let breaches = behavioural
        .buckets
        .iter()
        .filter(|b| b.breach)
        .map(|b| format!("cumulative gap to {} is {:.1}% of assets, below the floor of {:.1}%", b.label, b.cumulative_pct_assets, b.floor_pct.unwrap_or(0.0)))
        .collect();
    LiquidityGapResult { as_of, params_version, reporting_ccy: params.reporting_ccy.clone(), total_assets, contractual, behavioural, breaches }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{Amort, DayCount, HqlaLevel, RateType};

    fn pos(id: &str, side: BsSide, book: &str, segment: &str, product: &str, balance: f64, maturity: Option<NaiveDate>) -> BsPosition {
        BsPosition {
            as_of: NaiveDate::from_ymd_opt(2026, 11, 4).unwrap(),
            position_id: id.into(),
            side,
            book: book.into(),
            segment: segment.into(),
            product: product.into(),
            ccy: "BHD".into(),
            balance,
            rate: 0.0,
            rate_type: RateType::Fixed,
            index_code: None,
            spread: 0.0,
            reset_months: None,
            pay_months: None,
            amort: Amort::Bullet,
            next_repricing: None,
            maturity,
            origination: None,
            day_count: DayCount::Act360,
            hqla_level: None,
            encumbered: false,
            lcr_class: None,
            nsfr_class: None,
            beta: 0.0,
            behavioural_months: None,
            undrawn: 0.0,
        }
    }

    #[test]
    fn contractual_and_behavioural_views_of_a_small_book() {
        let as_of = NaiveDate::from_ymd_opt(2026, 11, 4).unwrap();
        let mut bond = pos("BOND", BsSide::Asset, "SECURITIES", "SOVEREIGN", "GOVT_BOND", 400.0, Some(as_of + chrono::Duration::days(700)));
        bond.hqla_level = Some(HqlaLevel::L1);
        let book = vec![
            pos("CASH", BsSide::Asset, "CASH", "CENTRAL_BANK", "CASH_AND_RESERVES", 100.0, None),
            bond,
            pos("LOAN", BsSide::Asset, "LOANS", "CORPORATE", "TERM_LOAN", 500.0, Some(as_of + chrono::Duration::days(20))),
            pos("CA", BsSide::Liability, "DEPOSITS", "RETAIL", "CURRENT_ACCOUNT", 600.0, None),
            pos("EQ", BsSide::Liability, "EQUITY", "SHAREHOLDERS", "EQUITY", 400.0, None),
        ];
        let mut params = AlmParams::default();
        params.gap.nmd_behavioural_months.insert("CURRENT_ACCOUNT".into(), 1.0);
        let r = compute(&book, &params, &Spots::new(), as_of, None);
        // Contractually the whole current account is due overnight: 100 cash in, 600 out.
        assert_eq!(r.contractual.buckets[0].gap, -500.0);
        assert_eq!(r.contractual.first_negative.as_deref(), Some("ON"));
        // The 20-day loan arrives in the 1M bucket, the bond in 2Y, equity never.
        assert_eq!(r.contractual.buckets[2].inflows, 500.0);
        assert_eq!(r.contractual.buckets[6].inflows, 400.0);
        assert_eq!(r.contractual.buckets[8].outflows, 400.0);
        // Behaviourally 15% of retail current accounts leave overnight (90), the bond is sold at once
        // (400, no L1 haircut) and the stable 510 runs off in the first month.
        assert!((r.behavioural.buckets[0].outflows - 90.0).abs() < 1e-9);
        assert!((r.behavioural.buckets[0].inflows - 500.0).abs() < 1e-9);
        assert!((r.behavioural.buckets[2].outflows - 510.0).abs() < 1e-9);
        // Cumulative, the book balances over its life.
        assert!(r.behavioural.buckets.last().unwrap().cumulative.abs() < 1e-9);
        assert!(r.contractual.buckets.last().unwrap().cumulative.abs() < 1e-9);
    }
}
