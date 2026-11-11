//! Trial hedges: positions an ALM user adds to see what they would do to the gap, NII and EVE.
//! They are never booked and never stored with the balance sheet. Each is two cohorts in the
//! `HEDGE` book: a fixed leg to the hedge's tenor and a floating leg that resets on the way.
//! Paying fixed (a pay-fixed swap, or new fixed-rate funding held in short money) is a fixed
//! liability and a floating asset; receiving fixed (a receive-fixed swap, or a fixed-rate
//! investment funded short) is the other way round. With no rate given the fixed leg takes the
//! base curve at its tenor, so the hedge starts worth about nothing.

use chrono::NaiveDate;
use domain::{Amort, BsPosition, BsSide, DayCount, HedgeKind, RateType, TrialHedge};

use crate::nii::Curves;
use crate::schedule::add_months;

/// The book the legs sit in. Balances in it are notionals, so they stay out of total assets and
/// earning assets.
pub const HEDGE_BOOK: &str = "HEDGE";

/// How often the floating leg resets, in months: the swap's own, else three; funding and
/// investments sit against one-month money.
pub fn reset_months(h: &TrialHedge) -> u32 {
    match h.kind {
        HedgeKind::PayFixedSwap | HedgeKind::ReceiveFixedSwap => h.reset_months.unwrap_or(3).clamp(1, 12),
        HedgeKind::FixedFunding | HedgeKind::FixedInvestment => 1,
    }
}

/// The fixed rate a hedge runs at: its own, else the base curve at its tenor.
pub fn fixed_rate(h: &TrialHedge, curves: &Curves) -> f64 {
    h.rate.unwrap_or_else(|| curves.get(&h.ccy).map(|c| c.rate(h.tenor_months as f64 / 12.0)).unwrap_or(0.0))
}

fn leg(as_of: NaiveDate, id: String, side: BsSide, h: &TrialHedge, maturity: NaiveDate) -> BsPosition {
    BsPosition {
        as_of,
        position_id: id,
        side,
        book: HEDGE_BOOK.into(),
        segment: "TRIAL".into(),
        product: h.kind.as_str().into(),
        ccy: h.ccy.clone(),
        balance: h.notional,
        rate: 0.0,
        rate_type: RateType::Fixed,
        index_code: None,
        spread: 0.0,
        reset_months: None,
        pay_months: None,
        amort: Amort::Bullet,
        next_repricing: None,
        maturity: Some(maturity),
        origination: Some(as_of),
        day_count: DayCount::Act365,
        hqla_level: None,
        encumbered: false,
        lcr_class: None,
        nsfr_class: None,
        beta: 0.0,
        behavioural_months: None,
        undrawn: 0.0,
    }
}

/// The cohorts the hedges add to the balance sheet.
pub fn positions(hedges: &[TrialHedge], curves: &Curves, as_of: NaiveDate) -> Vec<BsPosition> {
    let mut out = Vec::with_capacity(hedges.len() * 2);
    for (i, h) in hedges.iter().enumerate() {
        let maturity = add_months(as_of, h.tenor_months as i64);
        let reset = reset_months(h);
        let pays_fixed = matches!(h.kind, HedgeKind::PayFixedSwap | HedgeKind::FixedFunding);
        let (fixed_side, float_side) = if pays_fixed { (BsSide::Liability, BsSide::Asset) } else { (BsSide::Asset, BsSide::Liability) };

        let mut fixed = leg(as_of, format!("HEDGE-{i}-FIXED"), fixed_side, h, maturity);
        fixed.rate = fixed_rate(h, curves);
        // Interest yearly, so a leg at the curve's rate is worth par.
        fixed.pay_months = Some(h.tenor_months.min(12) as i16);
        out.push(fixed);

        let mut float = leg(as_of, format!("HEDGE-{i}-FLOAT"), float_side, h, maturity);
        float.rate_type = RateType::Float;
        float.rate = curves.get(&h.ccy).map(|c| c.rate(reset as f64 / 12.0)).unwrap_or(0.0);
        float.reset_months = Some(reset as i16);
        float.next_repricing = Some(add_months(as_of, reset as i64).min(maturity));
        out.push(float);
    }
    out
}
