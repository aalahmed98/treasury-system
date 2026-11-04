//! Net interest income variance between two periods, line by line: the effect of the number of
//! days, of volume, of rate and the joint residual, with volume split at portfolio level into
//! pure volume and mix. Assets and liabilities are separate portfolios and currencies are
//! converted at one spot, so the effects add up to the change exactly.

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};
use domain::{AttributionLine, AttributionResult, AttributionStep, BsSide, CompareTo, PeriodLine};

use crate::lcr::Spots;
use crate::schedule::add_months;

fn days_in_month(d: NaiveDate) -> f64 {
    let first = NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap_or(d);
    (add_months(first, 1) - first).num_days() as f64
}

fn basis(ccy: &str) -> f64 {
    if ccy == "GBP" { 365.0 } else { 360.0 }
}

/// `a` and `b` are the two periods' lines (any kinds; the caller picks actual or budget).
pub fn compute(a: &[PeriodLine], b: &[PeriodLine], period_a: NaiveDate, period_b: NaiveDate, compare_to: CompareTo, reporting_ccy: &str, spots: &Spots) -> AttributionResult {
    let fx = |ccy: &str| if ccy == reporting_ccy { 1.0 } else { spots.get(ccy).copied().unwrap_or(1.0) };
    let d_a = days_in_month(period_a);
    let d_b = days_in_month(period_b);
    let key = |l: &PeriodLine| (l.book.clone(), l.side, l.ccy.clone());
    let mut keys: BTreeMap<(String, BsSide, String), (Option<&PeriodLine>, Option<&PeriodLine>)> = BTreeMap::new();
    for l in a {
        keys.entry(key(l)).or_default().0 = Some(l);
    }
    for l in b {
        keys.entry(key(l)).or_default().1 = Some(l);
    }

    let mut lines = Vec::new();
    for ((book, side, ccy), (la, lb)) in &keys {
        let v = fx(ccy);
        let bs = basis(ccy);
        let (n_a, v_a) = la.map(|l| (l.nii * v, l.avg_balance * v)).unwrap_or((0.0, 0.0));
        let (n_b, v_b) = lb.map(|l| (l.nii * v, l.avg_balance * v)).unwrap_or((0.0, 0.0));
        if v_a <= 0.0 && v_b <= 0.0 {
            continue;
        }
        // Period rates as decimals per annum; a side that did not exist takes the other's rate.
        let mut r_a = if v_a > 0.0 { n_a * bs / (v_a * d_a) } else { 0.0 };
        let mut r_b = if v_b > 0.0 { n_b * bs / (v_b * d_b) } else { 0.0 };
        if v_a <= 0.0 {
            r_a = r_b;
        }
        if v_b <= 0.0 {
            r_b = r_a;
        }
        let days_effect = v_a * r_a * (d_b - d_a) / bs;
        let volume_effect = (v_b - v_a) * r_a * d_b / bs;
        let rate_effect = (r_b - r_a) * v_a * d_b / bs;
        let residual = (v_b - v_a) * (r_b - r_a) * d_b / bs;
        lines.push(AttributionLine {
            book: book.clone(),
            side: *side,
            ccy: ccy.clone(),
            nii_a: n_a,
            nii_b: n_b,
            volume_a: v_a,
            volume_b: v_b,
            rate_a: r_a * 100.0,
            rate_b: r_b * 100.0,
            days_effect,
            volume_effect,
            rate_effect,
            residual,
        });
    }

    // Portfolio split per side and currency: pure volume at the portfolio's old average rate,
    // mix from the shift of weights between lines.
    let mut pure_volume = 0.0;
    let mut mix = 0.0;
    let mut groups: BTreeMap<(BsSide, String), Vec<&AttributionLine>> = BTreeMap::new();
    for l in &lines {
        groups.entry((l.side, l.ccy.clone())).or_default().push(l);
    }
    for ((_, ccy), g) in &groups {
        let bs = basis(ccy);
        let sum_a: f64 = g.iter().map(|l| l.volume_a).sum();
        let sum_b: f64 = g.iter().map(|l| l.volume_b).sum();
        let total_volume: f64 = g.iter().map(|l| l.volume_effect).sum();
        if sum_a <= 0.0 || sum_b <= 0.0 {
            pure_volume += total_volume;
            continue;
        }
        let r_bar_a: f64 = g.iter().map(|l| l.volume_a / sum_a * l.rate_a / 100.0).sum();
        let pure = (sum_b - sum_a) * r_bar_a * d_b / bs;
        pure_volume += pure;
        mix += total_volume - pure;
    }

    let nii_a: f64 = lines.iter().map(|l| l.nii_a).sum();
    let nii_b: f64 = lines.iter().map(|l| l.nii_b).sum();
    let days_effect: f64 = lines.iter().map(|l| l.days_effect).sum();
    let rate_effect: f64 = lines.iter().map(|l| l.rate_effect).sum();
    let residual: f64 = lines.iter().map(|l| l.residual).sum();
    let steps = vec![
        AttributionStep { label: format!("NII {}", period_a.format("%b %Y")), value: nii_a },
        AttributionStep { label: "Days".into(), value: days_effect },
        AttributionStep { label: "Volume".into(), value: pure_volume },
        AttributionStep { label: "Mix".into(), value: mix },
        AttributionStep { label: "Rate".into(), value: rate_effect },
        AttributionStep { label: "Residual".into(), value: residual },
        AttributionStep { label: format!("NII {}", period_b.format("%b %Y")), value: nii_b },
    ];
    AttributionResult {
        period_a,
        period_b,
        compare_to,
        reporting_ccy: reporting_ccy.into(),
        nii_a,
        nii_b,
        delta: nii_b - nii_a,
        days_effect,
        pure_volume_effect: pure_volume,
        mix_effect: mix,
        rate_effect,
        residual,
        steps,
        lines,
    }
}
