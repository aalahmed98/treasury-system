//! Hand-worked cases for every calculator beyond the LCR.

use std::collections::BTreeMap;

use alm::curve::{Curve, CustomShock, shock_bp};
use alm::funding::DepositorGroup;
use alm::nii::Curves;
use alm::{attribution, behaviour, coverage, eve, funding, gap, hedge, ird, nii, nmd, nsfr, schedule, stress};
use chrono::NaiveDate;
use domain::{AlmParams, Amort, BalanceSheetMode, HedgeKind, TrialHedge, BsPosition, BsSide, CompareTo, DayCount, HqlaLevel, PeriodKind, PeriodLine, RateScenario, RateShock, RateType};

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

const AS_OF: &str = "2026-11-04";

fn pos(id: &str, side: BsSide, ccy: &str, balance: f64) -> BsPosition {
    BsPosition {
        as_of: d(AS_OF),
        position_id: id.into(),
        side,
        book: "TEST".into(),
        segment: "TEST".into(),
        product: "TEST".into(),
        ccy: ccy.into(),
        balance,
        rate: 5.0,
        rate_type: RateType::Fixed,
        index_code: None,
        spread: 0.0,
        reset_months: None,
        pay_months: None,
        amort: Amort::Bullet,
        next_repricing: None,
        maturity: None,
        origination: None,
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

fn flat(ccy: &str, rate: f64) -> Curves {
    [(ccy.to_string(), Curve::flat(ccy, rate))].into_iter().collect()
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol * b.abs().max(1.0)
}

#[test]
fn curves_interpolate_shock_and_floor() {
    let c = Curve { ccy: "BHD".into(), points: vec![(0.0, 5.0), (1.0, 6.0)] };
    assert!(close(c.rate(0.5), 5.5, 1e-12));
    assert!(close(c.rate(2.0), 6.0, 1e-12), "flat beyond the last tenor");
    assert!(close(c.rate(-1.0), 5.0, 1e-12));
    assert!(close(c.df(2.0), 1.0 / (1.06f64).powi(2), 1e-12));

    let s = RateShock { parallel: 200.0, short: 300.0, long: 150.0 };
    assert_eq!(shock_bp(RateScenario::ParallelUp, s, 3.0), 200.0);
    assert_eq!(shock_bp(RateScenario::ParallelDown, s, 3.0), -200.0);
    assert!(close(shock_bp(RateScenario::ShortUp, s, 0.0), 300.0, 1e-12));
    assert!(close(shock_bp(RateScenario::ShortUp, s, 4.0), 300.0 * (-1.0f64).exp(), 1e-9));
    assert!(close(shock_bp(RateScenario::Steepener, s, 0.0), -195.0, 1e-9));
    assert!(close(shock_bp(RateScenario::Steepener, s, 40.0), 135.0, 1e-3), "long end of the steepener");
    assert!(close(shock_bp(RateScenario::Flattener, s, 0.0), 240.0, 1e-9));
    assert!(close(shock_bp(RateScenario::Flattener, s, 40.0), -90.0, 1e-3));
    assert_eq!(shock_bp(RateScenario::Base, s, 1.0), 0.0);

    let low = Curve { ccy: "EUR".into(), points: vec![(0.0, 0.5), (10.0, 0.5), (20.0, 0.5), (30.0, 0.5)] };
    let down = low.shocked(RateScenario::ParallelDown, s, -100.0, 5.0);
    assert!(close(down.rate(0.0), -1.0, 1e-9), "floored at -100 bp at the short end");
    assert!(close(down.rate(10.0), -0.5, 1e-9), "the floor rises 5 bp a year");
    assert!(close(down.rate(20.0), 0.0, 1e-9), "and reaches zero at twenty years");
}

#[test]
fn nsfr_applies_factors_by_class_and_band() {
    let as_of = d(AS_OF);
    let mut eq = pos("eq", BsSide::Liability, "BHD", 100.0);
    eq.nsfr_class = Some("CAPITAL".into());
    eq.rate_type = RateType::NotSensitive;
    let mut ret = pos("ret", BsSide::Liability, "BHD", 1000.0);
    ret.nsfr_class = Some("RETAIL_SME_STABLE".into());
    let mut fi = pos("fi", BsSide::Liability, "BHD", 200.0);
    fi.nsfr_class = Some("FI".into());
    fi.maturity = Some(d("2027-02-04"));
    let mut corp = pos("corp", BsSide::Liability, "BHD", 300.0);
    corp.nsfr_class = Some("NONFIN_WHOLESALE".into());
    corp.maturity = Some(d("2027-07-04"));
    let mut cash = pos("cash", BsSide::Asset, "BHD", 100.0);
    cash.nsfr_class = Some("CASH_CB".into());
    let mut govt = pos("govt", BsSide::Asset, "BHD", 500.0);
    govt.hqla_level = Some(HqlaLevel::L1);
    let mut l2a = pos("l2a", BsSide::Asset, "BHD", 100.0);
    l2a.hqla_level = Some(HqlaLevel::L2A);
    let mut loan3y = pos("loan3y", BsSide::Asset, "BHD", 1000.0);
    loan3y.nsfr_class = Some("LOANS_OTHER".into());
    loan3y.maturity = Some(d("2029-11-04"));
    let mut loan5m = pos("loan5m", BsSide::Asset, "BHD", 200.0);
    loan5m.nsfr_class = Some("LOANS_OTHER".into());
    loan5m.maturity = Some(d("2027-04-04"));
    let mut commit = pos("commit", BsSide::OffBalance, "BHD", 0.0);
    commit.undrawn = 400.0;
    let mut pledged = pos("pledged", BsSide::Asset, "BHD", 100.0);
    pledged.hqla_level = Some(HqlaLevel::L1);
    pledged.encumbered = true;
    pledged.maturity = Some(d("2028-11-04"));

    let r = nsfr::compute(&[eq, ret, fi, corp, cash, govt, l2a, loan3y, loan5m, commit, pledged], &AlmParams::default(), &BTreeMap::new(), as_of, Some(1));
    assert!(close(r.asf, 100.0 + 950.0 + 0.0 + 150.0, 1e-9), "asf {}", r.asf);
    assert!(close(r.rsf, 25.0 + 15.0 + 850.0 + 100.0 + 20.0 + 100.0, 1e-9), "rsf {}", r.rsf);
    assert!(close(r.ratio.unwrap(), 1200.0 / 1110.0 * 100.0, 1e-9));
    assert!(r.asf_lines.iter().any(|l| l.class == "NONFIN_WHOLESALE" && l.bucket == "6M-1Y" && (l.factor - 50.0).abs() < 1e-9));
}

#[test]
fn gap_slots_by_repricing_and_checks_cumulative_limits() {
    let as_of = d(AS_OF);
    let mut float = pos("float", BsSide::Asset, "BHD", 1000.0);
    float.rate_type = RateType::Float;
    float.next_repricing = Some(as_of + chrono::Duration::days(45));
    float.maturity = Some(d("2030-01-01"));
    let mut bullet = pos("bullet", BsSide::Asset, "BHD", 500.0);
    bullet.maturity = Some(as_of + chrono::Duration::days(400));
    let mut linear = pos("linear", BsSide::Asset, "BHD", 300.0);
    linear.maturity = Some(d("2027-11-04"));
    linear.pay_months = Some(6);
    linear.amort = Amort::Linear;
    let mut ca = pos("ca", BsSide::Liability, "BHD", 3600.0);
    ca.rate_type = RateType::Admin;
    ca.product = "CURRENT_ACCOUNT".into(); // 36 behavioural months by parameter
    let mut equity = pos("eq", BsSide::Liability, "BHD", 1000.0);
    equity.rate_type = RateType::NotSensitive;

    let r = gap::compute(&[float, bullet, linear, ca, equity], &AlmParams::default(), &BTreeMap::new(), as_of, None);
    let b = |label: &str| r.buckets.iter().find(|b| b.label == label).unwrap().clone();
    assert!(close(r.total_assets, 1800.0, 1e-9));
    assert!(close(b("3M").assets, 1000.0, 1e-9), "the floater reprices in 45 days");
    assert!(close(b("2Y").assets, 500.0, 1e-9), "the bullet at 400 days");
    assert!(close(b("6M").assets, 150.0, 1e-9) && close(b("1Y").assets, 150.0, 1e-9), "linear principal at six and twelve months");
    for (label, expect) in [("1M", 100.0), ("3M", 200.0), ("6M", 300.0), ("1Y", 600.0), ("2Y", 1200.0), ("5Y", 1200.0)] {
        assert!(close(b(label).liabilities, expect, 1e-9), "{label}: {}", b(label).liabilities);
    }
    assert!(close(b("1M").cumulative, -100.0, 1e-9));
    assert!(close(b("3M").cumulative, 700.0, 1e-9));
    assert!(!b("1M").breach, "5.6% is inside the 15% limit");
    assert!(b("3M").breach, "38.9% is outside the 20% limit");
    assert_eq!(b("ON").limit_pct, None);
    assert_eq!(r.by_ccy.len(), 1);
}

#[test]
fn nii_projects_fixed_floating_and_administered_cohorts() {
    let as_of = d(AS_OF);
    let p = AlmParams::default();
    let curves = flat("BHD", 4.0);
    let mut fixed = pos("fixed", BsSide::Asset, "BHD", 1200.0);
    fixed.rate = 3.6;
    fixed.day_count = DayCount::Act360;
    fixed.maturity = Some(d("2028-11-04"));
    fixed.origination = Some(d("2025-11-04"));
    let base = nii::compute(&[fixed.clone()], &p, &BTreeMap::new(), &curves, as_of, RateScenario::Base, 0.0, Some(1));
    assert_eq!(base.months.len(), 12);
    assert!(close(base.nii, 1200.0 * 0.036 * 365.0 / 360.0, 1e-9), "nii {}", base.nii);
    assert!(close(base.nim.unwrap(), base.nii / 1200.0 * 100.0, 1e-9));
    let up = nii::compute(&[fixed.clone()], &p, &BTreeMap::new(), &curves, as_of, RateScenario::ParallelUp, 0.0, Some(1));
    assert!(close(up.nii, base.nii, 1e-12), "a fixed cohort beyond the horizon does not move");

    let mut float = pos("float", BsSide::Asset, "BHD", 1000.0);
    float.rate_type = RateType::Float;
    float.rate = 5.0;
    float.spread = 1.0;
    float.reset_months = Some(3);
    float.next_repricing = Some(d("2026-12-04"));
    float.maturity = Some(d("2030-01-01"));
    float.day_count = DayCount::Act360;
    let fb = nii::compute(&[float.clone()], &p, &BTreeMap::new(), &curves, as_of, RateScenario::Base, 0.0, None);
    let fu = nii::compute(&[float.clone()], &p, &BTreeMap::new(), &curves, as_of, RateScenario::ParallelUp, 0.0, None);
    // After the first reset the floater pays 2% more for the remaining 335 days.
    assert!(close(fu.nii - fb.nii, 1000.0 * 0.02 * 335.0 / 360.0, 1e-6), "delta {}", fu.nii - fb.nii);
    assert!(fu.nii > fb.nii, "an asset-sensitive book gains when rates rise");

    let mut dep = pos("dep", BsSide::Liability, "BHD", 1000.0);
    dep.rate_type = RateType::Admin;
    dep.rate = 1.0;
    dep.beta = 0.5;
    dep.day_count = DayCount::Act360;
    let db = nii::compute(&[dep.clone()], &p, &BTreeMap::new(), &curves, as_of, RateScenario::Base, 0.0, None);
    let du = nii::compute(&[dep.clone()], &p, &BTreeMap::new(), &curves, as_of, RateScenario::ParallelUp, 0.0, None);
    assert!(close(db.nii, -1000.0 * 0.01 * 365.0 / 360.0, 1e-9));
    assert!(close(du.nii - db.nii, -1000.0 * 0.01 * 365.0 / 360.0, 1e-9), "half the 200 bp passes through");
    assert_eq!(du.earning_assets, 0.0);
    assert_eq!(du.nim, None);
}

#[test]
fn deposit_betas_come_from_the_assumptions_by_product_and_segment() {
    let as_of = d(AS_OF);
    let curves = flat("BHD", 4.0);
    let deposit = |product: &str, segment: &str| {
        let mut p = pos(product, BsSide::Liability, "BHD", 1000.0);
        p.rate_type = RateType::Admin;
        p.rate = 1.0;
        p.beta = 0.9; // the source's beta: ignored, the assumption decides
        p.day_count = DayCount::Act360;
        p.product = product.into();
        p.segment = segment.into();
        p
    };
    let mut params = AlmParams::default();
    params.nii.betas = [("SAVINGS".to_string(), 0.4), ("CURRENT_ACCOUNT".to_string(), 0.2), ("CURRENT_ACCOUNT.RETAIL".to_string(), 0.0)].into();
    params.nii.default_beta = 0.5;
    // Under +200 bp the extra cost is beta × 2% on 1000 for the year.
    let delta = |p: &BsPosition, params: &AlmParams| {
        let b = nii::compute(std::slice::from_ref(p), params, &BTreeMap::new(), &curves, as_of, RateScenario::Base, 0.0, None).nii;
        let u = nii::compute(std::slice::from_ref(p), params, &BTreeMap::new(), &curves, as_of, RateScenario::ParallelUp, 0.0, None).nii;
        (b - u) / (1000.0 * 0.02 * 365.0 / 360.0)
    };
    assert!(close(delta(&deposit("SAVINGS", "RETAIL"), &params), 0.4, 1e-9));
    assert!(close(delta(&deposit("CURRENT_ACCOUNT", "CORPORATE"), &params), 0.2, 1e-9));
    assert!(close(delta(&deposit("CURRENT_ACCOUNT", "RETAIL"), &params), 0.0, 1e-9), "the segment's entry wins");
    assert!(close(delta(&deposit("CALL_ACCOUNT", "RETAIL"), &params), 0.5, 1e-9), "an unlisted product takes the default");
    params.nii.betas.insert("SAVINGS".into(), 0.7);
    assert!(close(delta(&deposit("SAVINGS", "RETAIL"), &params), 0.7, 1e-9), "the lever moves the result");
}

#[test]
fn coverage_says_what_each_lever_acts_on_and_what_has_nothing() {
    let as_of = d(AS_OF);
    let mut sav = pos("sav", BsSide::Liability, "BHD", 400.0);
    sav.rate_type = RateType::Admin;
    sav.product = "SAVINGS".into();
    sav.segment = "RETAIL".into();
    sav.lcr_class = Some("RET_LESS_STABLE".into());
    sav.nsfr_class = Some("RETAIL_SME_LESS_STABLE".into());
    let mut call = sav.clone();
    call.position_id = "call".into();
    call.product = "CALL_ACCOUNT".into();
    call.segment = "CORPORATE".into();
    call.balance = 100.0;
    let mut td = pos("td", BsSide::Liability, "BHD", 300.0);
    td.lcr_class = Some("FI_WHOLESALE".into());
    td.nsfr_class = Some("FI".into());
    td.maturity = Some(d("2027-08-04")); // past 30 days and six months
    let c = coverage::compute(&[sav, call, td], &AlmParams::default(), &BTreeMap::new(), as_of);
    assert_eq!(c["nii.betas.SAVINGS"], 400.0);
    assert_eq!(c["nii.default_beta"], 100.0, "a product with no entry takes the default");
    assert_eq!(c["nii.betas.OVERDRAFT"], 0.0);
    assert_eq!(c["lcr.outflow_rates.RET_LESS_STABLE"], 500.0);
    assert_eq!(c["lcr.outflow_rates.FI_WHOLESALE"], 0.0, "maturing after 30 days: no LCR outflow");
    assert_eq!(c["nsfr.asf.FI.1"], 300.0);
    assert_eq!(c["nsfr.asf.FI.0"], 0.0);
    assert_eq!(c["liquidity_gap.stable_share_pct.RETAIL"], 400.0);
    assert_eq!(c["nsfr.encumbered_rsf_pct"], 0.0);
}

#[test]
fn cash_at_the_central_bank_needs_no_stable_funding_even_as_hqla() {
    let as_of = d(AS_OF);
    let mut cash = pos("cash", BsSide::Asset, "BHD", 1000.0);
    cash.nsfr_class = Some("CASH_CB".into());
    cash.hqla_level = Some(HqlaLevel::L1);
    let mut govt = pos("govt", BsSide::Asset, "BHD", 1000.0);
    govt.hqla_level = Some(HqlaLevel::L1);
    let r = nsfr::compute(&[cash, govt], &AlmParams::default(), &BTreeMap::new(), as_of, None);
    assert!(close(r.rsf, 50.0, 1e-9), "cash 0%, Level 1 securities 5%: rsf {}", r.rsf);
}

#[test]
fn nii_on_a_running_off_balance_sheet_stops_at_maturity() {
    let as_of = d(AS_OF);
    let curves = flat("BHD", 4.0);
    let mut loan = pos("loan", BsSide::Asset, "BHD", 1200.0);
    loan.rate = 5.0;
    loan.day_count = DayCount::Act360;
    loan.origination = Some(d("2025-11-04"));
    loan.maturity = Some(schedule::add_months(as_of, 4));
    let mut p = AlmParams::default();
    let constant = nii::compute(&[loan.clone()], &p, &BTreeMap::new(), &curves, as_of, RateScenario::Base, 0.0, None);
    p.nii.constant_balance_sheet = false;
    let runoff = nii::compute(&[loan.clone()], &p, &BTreeMap::new(), &curves, as_of, RateScenario::Base, 0.0, None);
    // Running off: 5% until maturity, then the cash it releases earns the one-month rate (4%).
    let four_months = (schedule::add_months(as_of, 4) - as_of).num_days() as f64;
    let expected = 1200.0 * 0.05 * four_months / 360.0 + 1200.0 * 0.04 * (365.0 - four_months) / 360.0;
    assert!(close(runoff.nii, expected, 1e-6), "runoff {} expected {expected}", runoff.nii);
    assert!(runoff.by_book.iter().any(|b| b.key == "BALANCING" && b.nii > 0.0));
    // A maturing deposit is replaced by funding at the one-month rate.
    let mut dep = pos("dep", BsSide::Liability, "BHD", 1000.0);
    dep.rate = 3.0;
    dep.day_count = DayCount::Act360;
    dep.maturity = Some(schedule::add_months(as_of, 4));
    let funded = nii::compute(&[dep], &p, &BTreeMap::new(), &curves, as_of, RateScenario::Base, 0.0, None);
    let expected = -(1000.0 * 0.03 * four_months / 360.0 + 1000.0 * 0.04 * (365.0 - four_months) / 360.0);
    assert!(close(funded.nii, expected, 1e-6), "funded {} expected {expected}", funded.nii);
    // Constant: rolled at today's curve plus its margin (5% over a flat 4%), so the full year at 5%.
    assert!(close(constant.nii, 1200.0 * 0.05 * 365.0 / 360.0, 1e-9), "constant {}", constant.nii);
}

#[test]
fn eve_discounts_flows_on_the_shocked_curve() {
    let as_of = d(AS_OF);
    let p = AlmParams::default();
    let curves: Curves = [("BHD".to_string(), Curve::flat("BHD", 5.0)), ("USD".to_string(), Curve::flat("USD", 5.0))].into_iter().collect();
    let spots: BTreeMap<String, f64> = [("USD".to_string(), 0.376)].into_iter().collect();
    let mut asset = pos("a", BsSide::Asset, "BHD", 1000.0);
    asset.maturity = Some(d("2027-11-04"));
    let mut equity = pos("e", BsSide::Liability, "BHD", 100.0);
    equity.rate_type = RateType::NotSensitive;
    let r = eve::compute(&[asset.clone(), equity.clone()], &p, &spots, &curves, as_of, RateScenario::ParallelUp, 0.0, None);
    assert!(close(r.base, 1000.0, 1e-9), "par at the base curve: {}", r.base);
    assert!(close(r.shocked, 1050.0 / 1.07, 1e-9));
    assert!(close(r.delta, 1050.0 / 1.07 - 1000.0, 1e-9));
    assert!(close(r.basel_loss, 1000.0 - 1050.0 / 1.07, 1e-9));

    let mut usd_liab = pos("l", BsSide::Liability, "USD", 500.0);
    usd_liab.maturity = Some(d("2027-11-04"));
    let r2 = eve::compute(&[asset, equity, usd_liab], &p, &spots, &curves, as_of, RateScenario::ParallelUp, 0.0, None);
    let loss = 1000.0 - 1050.0 / 1.07;
    let gain = (500.0 - 525.0 / 1.07) * 0.376;
    assert!(close(r2.delta, -loss + gain, 1e-9));
    assert!(close(r2.basel_loss, loss - 0.5 * gain, 1e-9), "losses less half the gains");
    assert_eq!(r2.by_ccy.len(), 2);
}

#[test]
fn stress_runs_every_scenario_and_the_liquidity_horizon() {
    let as_of = d(AS_OF);
    let mut p = AlmParams::default();
    p.tier1_capital = 90.0;
    let curves = flat("BHD", 5.0);
    let mut asset = pos("a", BsSide::Asset, "BHD", 1000.0);
    asset.maturity = Some(d("2027-11-04"));
    let mut cash = pos("cash", BsSide::Asset, "BHD", 100.0);
    cash.hqla_level = Some(HqlaLevel::L1);
    cash.rate_type = RateType::NotSensitive;
    let mut nmd = pos("nmd", BsSide::Liability, "BHD", 5000.0);
    nmd.rate_type = RateType::Admin;
    nmd.rate = 0.0;
    nmd.lcr_class = Some("RET_STABLE".into());
    let mut eur = pos("eur", BsSide::Asset, "EUR", 100.0);
    eur.rate_type = RateType::NotSensitive;
    let spots: BTreeMap<String, f64> = [("EUR".to_string(), 0.41)].into_iter().collect();

    let r = stress::compute(&[asset, cash, nmd, eur], &p, &spots, &curves, as_of, Some((-50.0).into()), Some(1));
    assert_eq!(r.rate_scenarios.len(), 7);
    let up = r.rate_scenarios.iter().find(|s| s.scenario == RateScenario::ParallelUp).unwrap();
    assert!(up.eve_loss > 0.0 && up.eve_breach, "the bond loses 18.7 and the deposit gains 3.9 in the same currency: 14.8 on 90 of Tier 1 breaches the 15% limit: {up:?}");
    // A one-year bond is hurt most by the short-rate shock: 300 bp decays to 234 bp at one year,
    // more than the 200 bp parallel move.
    assert_eq!(r.worst_eve, Some(RateScenario::ShortUp));
    let short = r.rate_scenarios.iter().find(|s| s.scenario == RateScenario::ShortUp).unwrap();
    assert!(short.eve_loss > up.eve_loss);
    let custom = r.rate_scenarios.iter().find(|s| s.scenario == RateScenario::Custom).unwrap();
    assert!(custom.label.contains("-50"));
    // Liquidity: run-off 7.5% of 5000 over thirty days against a stock of 95 runs out on day 8.
    assert!(close(r.liquidity.stock_stressed, 95.0, 1e-9));
    assert_eq!(r.liquidity.survival_days, Some(8));
    assert_eq!(r.liquidity.horizon.len(), 90);
    assert!(r.liquidity.lcr_stressed.unwrap() < r.liquidity.lcr_base.unwrap());
    // FX: 100 EUR long, a 15% fall costs 15% of 41.
    let fx = r.fx.iter().find(|f| f.ccy == "EUR").unwrap();
    assert!(close(fx.exposure, 41.0, 1e-9) && close(fx.pnl, -6.15, 1e-9));
}

#[test]
fn attribution_effects_add_up_exactly_including_mix_and_day_bases() {
    let a = d("2026-09-01");
    let b = d("2026-10-01");
    let line = |period, book: &str, side, ccy: &str, balance: f64, rate: f64, days: f64, basis: f64| PeriodLine {
        period,
        book: book.into(),
        side,
        ccy: ccy.into(),
        kind: PeriodKind::Actual,
        nii: if side == BsSide::Asset { 1.0 } else { -1.0 } * balance * rate / 100.0 * days / basis,
        avg_balance: balance,
    };
    let pa = vec![
        line(a, "LOANS", BsSide::Asset, "BHD", 1000.0, 5.0, 30.0, 360.0),
        line(a, "SECURITIES", BsSide::Asset, "BHD", 1000.0, 3.0, 30.0, 360.0),
        line(a, "DEPOSITS", BsSide::Liability, "BHD", 800.0, 2.0, 30.0, 360.0),
        line(a, "LOANS", BsSide::Asset, "GBP", 100.0, 4.0, 30.0, 365.0),
    ];
    let pb = vec![
        line(b, "LOANS", BsSide::Asset, "BHD", 1200.0, 6.0, 31.0, 360.0),
        line(b, "SECURITIES", BsSide::Asset, "BHD", 800.0, 3.0, 31.0, 360.0),
        line(b, "DEPOSITS", BsSide::Liability, "BHD", 800.0, 2.5, 31.0, 360.0),
        line(b, "LOANS", BsSide::Asset, "GBP", 100.0, 4.0, 31.0, 365.0),
        line(b, "NEW", BsSide::Asset, "BHD", 50.0, 7.0, 31.0, 360.0),
    ];
    let spots: BTreeMap<String, f64> = [("GBP".to_string(), 0.482)].into_iter().collect();
    let r = attribution::compute(&pa, &pb, a, b, CompareTo::Prior, "BHD", &spots);
    let sum = r.days_effect + r.pure_volume_effect + r.mix_effect + r.rate_effect + r.residual;
    assert!(close(sum, r.delta, 1e-9), "effects {sum} vs delta {}", r.delta);
    assert!(close(r.nii_a + sum, r.nii_b, 1e-9));
    for l in &r.lines {
        assert!(close(l.days_effect + l.volume_effect + l.rate_effect + l.residual, l.nii_b - l.nii_a, 1e-9), "{} {:?}", l.book, l.side);
    }
    let loans = r.lines.iter().find(|l| l.book == "LOANS" && l.ccy == "BHD").unwrap();
    assert!(close(loans.rate_a, 5.0, 1e-9) && close(loans.rate_b, 6.0, 1e-9));
    assert!(close(loans.days_effect, 1000.0 * 0.05 / 360.0, 1e-9));
    assert!(close(loans.volume_effect, 200.0 * 0.05 * 31.0 / 360.0, 1e-9));
    assert!(close(loans.rate_effect, 0.01 * 1000.0 * 31.0 / 360.0, 1e-9));
    // BHD assets moved money from the 3% line to the 5% line with the total rising by only 50:
    // most of the volume effect is mix.
    assert!(r.mix_effect > 0.3, "mix {}", r.mix_effect);
    let new = r.lines.iter().find(|l| l.book == "NEW").unwrap();
    assert!(close(new.rate_a, 7.0, 1e-9), "a new line takes the rate it was booked at");
    assert_eq!(r.steps.len(), 7);
    assert!(close(r.steps[0].value, r.nii_a, 1e-12) && close(r.steps[6].value, r.nii_b, 1e-12));
}

#[test]
fn ird_follows_covered_interest_parity() {
    let f = ird::forward(1.1000, 90, 3.0, 360.0, 5.0, 360.0);
    assert!(close(f, 1.1 * 1.0125 / 1.0075, 1e-12));
    let q = ird::quote("3M", 90, 1.1000, 3.0, "EUR", 5.0, "USD");
    assert!(close(q.points, (f - 1.1) * 10_000.0, 1e-9), "{}", q.points);
    assert!(q.points > 54.0 && q.points < 55.0);
    assert!(close(ird::implied_quote_rate(1.1, f, 90, 3.0, 360.0, 360.0).unwrap(), 5.0, 1e-9));
    assert!(close(ird::implied_base_rate(1.1, f, 90, 5.0, 360.0, 360.0).unwrap(), 3.0, 1e-9));
    assert!(close(ird::carry_per_day(1_000_000.0, 1.1, 3.0, 360.0, 5.0, 360.0), -1_000_000.0 * 1.1 * 0.02 / 360.0, 1e-9));
    assert_eq!(ird::pip_factor("JPY"), 100.0);
    assert_eq!(ird::basis("GBP"), 365.0);
    assert_eq!(ird::implied_quote_rate(1.1, f, 0, 3.0, 360.0, 360.0), None);
}

#[test]
fn funding_concentration_and_wholesale_share() {
    let as_of = d(AS_OF);
    let mk = |id: &str, book: &str, segment: &str, balance: f64| {
        let mut p = pos(id, BsSide::Liability, "BHD", balance);
        p.book = book.into();
        p.segment = segment.into();
        p
    };
    let positions = vec![
        mk("r", "DEPOSITS", "RETAIL", 1000.0),
        mk("c", "DEPOSITS", "CORPORATE", 500.0),
        mk("f", "DEPOSITS", "FI", 300.0),
        mk("ib", "INTERBANK", "FI", 200.0),
        mk("tf", "TERM_FUNDING", "FI", 300.0),
        mk("eq", "EQUITY", "SHAREHOLDERS", 400.0),
        mk("ol", "OTHER", "OTHER", 100.0),
    ];
    let groups = vec![
        DepositorGroup { name: "A".into(), segment: "SOVEREIGN".into(), ccy: "BHD".into(), balance: 150.0 },
        DepositorGroup { name: "B".into(), segment: "CORPORATE".into(), ccy: "BHD".into(), balance: 100.0 },
        DepositorGroup { name: "C".into(), segment: "CORPORATE".into(), ccy: "BHD".into(), balance: 50.0 },
        DepositorGroup { name: "D".into(), segment: "FI".into(), ccy: "USD".into(), balance: 100.0 },
    ];
    let spots: BTreeMap<String, f64> = [("USD".to_string(), 0.376)].into_iter().collect();
    let mut params = AlmParams::default();
    let r = funding::compute(&positions, &groups, &params, &spots, as_of, Some(1));
    assert!(close(r.total_deposits, 1800.0, 1e-9) && close(r.total_funding, 2400.0, 1e-9));
    assert!(close(r.wholesale_funding, 800.0, 1e-9) && close(r.wholesale_pct, 800.0 / 2400.0 * 100.0, 1e-9));
    assert!(close(r.top1_pct, 150.0 / 1800.0 * 100.0, 1e-9));
    assert!(close(r.top10_pct, (150.0 + 100.0 + 50.0 + 37.6) / 1800.0 * 100.0, 1e-9));
    assert!(r.breaches.is_empty(), "{:?}", r.breaches);
    assert_eq!(r.groups[0].name, "A");
    params.funding.top1_limit_pct = 5.0;
    params.funding.wholesale_share_limit_pct = 30.0;
    let r = funding::compute(&positions, &groups, &params, &spots, as_of, None);
    assert_eq!(r.breaches.len(), 2, "{:?}", r.breaches);
}

#[test]
fn a_custom_shock_moves_only_the_buckets_it_names() {
    let all = CustomShock::from_labels(100.0, &[]).unwrap();
    assert!(all.is_parallel());
    assert_eq!(all.label(), "Custom parallel +100 bp");
    let some = CustomShock::from_labels(100.0, &["3M".to_string(), "6M".to_string()]).unwrap();
    assert!(!some.is_parallel());
    assert_eq!(some.label(), "Custom +100 bp at 3M to 6M");
    assert_eq!(CustomShock::from_labels(-25.0, &["ON".to_string(), "1Y".to_string()]).unwrap().label(), "Custom -25 bp at ON, 1Y");
    assert!(CustomShock::from_labels(100.0, &["4M".to_string()]).is_err());
    assert_eq!(some.bp_at(1.0 / 12.0), 0.0, "the one-month index sits in the 1M bucket");
    assert_eq!(some.bp_at(0.25), 100.0, "the three-month index sits in the 3M bucket");
    assert_eq!(some.bp_at(0.5), 100.0);
    assert_eq!(some.bp_at(0.75), 0.0);

    // On the curve: a step at each bucket edge, whole days landing where the gap report slots them.
    let c = Curve::flat("BHD", 5.0).bucket_shocked(some, -100.0, 5.0);
    assert!(close(c.rate(30.0 / 365.0), 5.0, 1e-9), "day 30 is the last of 1M");
    assert!(close(c.rate(31.0 / 365.0), 6.0, 1e-9), "day 31 is the first of 3M");
    assert!(close(c.rate(182.0 / 365.0), 6.0, 1e-9));
    assert!(close(c.rate(183.0 / 365.0), 5.0, 1e-9));
    assert!(close(c.rate(3.0), 5.0, 1e-9));
    let floored = Curve::flat("BHD", 5.0).bucket_shocked(CustomShock::from_labels(-700.0, &["3M".to_string()]).unwrap(), -100.0, 5.0);
    assert!(close(floored.rate(0.25), (-100.0 + 5.0 * 0.25) / 100.0, 1e-9), "held above the floor");

    // Through the engines: a one-year bond feels a shock aimed at 1Y exactly as a parallel one,
    // and one aimed at 3M not at all.
    let as_of = d(AS_OF);
    let p = AlmParams::default();
    let curves = flat("BHD", 5.0);
    let mut bond = pos("a", BsSide::Asset, "BHD", 1000.0);
    bond.maturity = Some(d("2027-11-04"));
    let at_1y = CustomShock::from_labels(200.0, &["1Y".to_string()]).unwrap();
    let hit = eve::compute(&[bond.clone()], &p, &BTreeMap::new(), &curves, as_of, RateScenario::Custom, at_1y, None);
    assert!(close(hit.shocked, 1050.0 / 1.07, 1e-9));
    assert_eq!(hit.shock_label.as_deref(), Some("Custom +200 bp at 1Y"));
    let at_3m = CustomShock::from_labels(200.0, &["3M".to_string()]).unwrap();
    let miss = eve::compute(&[bond.clone()], &p, &BTreeMap::new(), &curves, as_of, RateScenario::Custom, at_3m, None);
    assert!(close(miss.delta, 0.0, 1e-9));

    // A floater resetting to the three-month index earns the shock only when 3M moves.
    let mut float = pos("f", BsSide::Asset, "BHD", 1200.0);
    float.rate_type = RateType::Float;
    float.reset_months = Some(3);
    float.day_count = DayCount::Act360;
    let base = nii::compute(&[float.clone()], &p, &BTreeMap::new(), &curves, as_of, RateScenario::Base, 0.0, None).nii;
    let moved = nii::compute(&[float.clone()], &p, &BTreeMap::new(), &curves, as_of, RateScenario::Custom, at_3m, None).nii;
    let unmoved = nii::compute(&[float], &p, &BTreeMap::new(), &curves, as_of, RateScenario::Custom, at_1y, None).nii;
    assert!(close(moved - base, 1200.0 * 0.02 * 365.0 / 360.0, 1e-9), "{}", moved - base);
    assert!(close(unmoved, base, 1e-9));
}

#[test]
fn the_gap_estimates_what_a_shock_does_to_income() {
    let as_of = d(AS_OF);
    let p = AlmParams::default();
    let curves = flat("BHD", 5.0);
    let mut float = pos("float", BsSide::Asset, "BHD", 1000.0);
    float.rate_type = RateType::Float;
    float.next_repricing = Some(as_of + chrono::Duration::days(45));
    let mut bullet = pos("bullet", BsSide::Asset, "BHD", 500.0);
    bullet.maturity = Some(as_of + chrono::Duration::days(400));
    let mut dep = pos("dep", BsSide::Liability, "BHD", 400.0);
    dep.maturity = Some(as_of + chrono::Duration::days(20));
    let book = [float, bullet, dep];
    let plain = gap::compute(&book, &p, &BTreeMap::new(), as_of, None);
    assert_eq!(plain.nii_effect_12m, None);
    assert_eq!(plain.buckets[0].nii_effect, None);

    let base = gap::with_shock(plain.clone(), &p, &curves, RateScenario::Base, 100.0);
    assert_eq!((base.shock_label, base.nii_effect_12m), (None, None), "nothing to estimate on the base curve");

    let up = gap::with_shock(plain.clone(), &p, &curves, RateScenario::Custom, 100.0);
    let b = |r: &domain::GapResult, label: &str| r.buckets.iter().find(|b| b.label == label).unwrap().clone();
    let m3 = 1000.0 * 0.01 * (1.0 - 60.5 / 365.0);
    let m1 = -400.0 * 0.01 * (1.0 - 18.5 / 365.0);
    assert!(close(b(&up, "3M").nii_effect.unwrap(), m3, 1e-9));
    assert!(close(b(&up, "1M").nii_effect.unwrap(), m1, 1e-9), "a liability gap costs income when rates rise");
    assert!(close(b(&up, "2Y").nii_effect.unwrap(), 0.0, 1e-12), "nothing reprices within the year");
    assert!(close(b(&up, "2Y").shock_bp.unwrap(), 100.0, 1e-9));
    assert!(close(up.nii_effect_12m.unwrap(), m3 + m1, 1e-9));
    assert!(close(b(&up, "3M").gap, b(&plain, "3M").gap, 1e-12), "the balances do not move");

    let aimed = gap::with_shock(plain, &p, &curves, RateScenario::Custom, CustomShock::from_labels(100.0, &["1M".to_string()]).unwrap());
    assert!(close(aimed.nii_effect_12m.unwrap(), m1, 1e-9), "only the bucket the shock is aimed at");
    assert!(close(b(&aimed, "3M").shock_bp.unwrap(), 0.0, 1e-9));
}

#[test]
fn the_duration_gap_predicts_the_revaluation_for_a_parallel_move() {
    let as_of = d(AS_OF);
    let mut p = AlmParams::default();
    p.tier1_capital = 100.0;
    let curves = flat("BHD", 5.0);
    let mut asset = pos("a", BsSide::Asset, "BHD", 1000.0);
    asset.maturity = Some(d("2028-11-04"));
    let mut liab = pos("l", BsSide::Liability, "BHD", 500.0);
    liab.maturity = Some(d("2027-11-04"));
    let book = [asset, liab];
    let r = eve::compute(&book, &p, &BTreeMap::new(), &curves, as_of, RateScenario::Custom, 100.0, None);
    let g = &r.duration;
    assert!(g.assets > 990.0 && g.assets < 1000.0 && close(g.liabilities, 500.0, 1e-9), "simple interest paid at two years is worth a little under par: {g:?}");
    assert!(close(g.d_liabilities, 1.0, 1e-9), "one flow at one year");
    assert!(close(g.d_assets, 731.0 / 365.0, 1e-9), "a two-year bond paying at maturity: {}", g.d_assets);
    assert!(close(g.gap_years, g.d_assets - g.liabilities / g.assets * g.d_liabilities, 1e-9));
    assert!(close(g.yield_pct, 5.0, 1e-9));
    assert_eq!(g.shock_bp, Some(100.0));
    let estimate = g.estimate.unwrap();
    assert!(close(estimate, -g.gap_years * g.assets * 0.01 / 1.05, 1e-9), "the formula as written");
    assert!(r.delta < 0.0 && (estimate - r.delta).abs() < 0.03 * r.delta.abs(), "close to the full revaluation: {estimate} against {}", r.delta);
    assert!(close(r.by_bucket.iter().map(|b| b.delta).sum::<f64>(), r.delta, 1e-9));
    assert!(close(r.loss_pct_tier1.unwrap(), r.basel_loss, 1e-9), "Tier 1 is 100");

    let steep = eve::compute(&book, &p, &BTreeMap::new(), &curves, as_of, RateScenario::Steepener, 0.0, None);
    assert_eq!(steep.duration.estimate, None, "the formula assumes a parallel move");
    let aimed = eve::compute(&book, &p, &BTreeMap::new(), &curves, as_of, RateScenario::Custom, CustomShock::from_labels(100.0, &["1Y".to_string()]).unwrap(), None);
    assert_eq!(aimed.duration.estimate, None);
    assert!(aimed.delta > 0.0, "only the liability is discounted harder");
}

#[test]
fn deposits_split_into_a_core_that_decays_and_a_rest_that_reprices_overnight() {
    let as_of = d(AS_OF);
    let mut p = AlmParams::default();
    let mut ca = pos("ca", BsSide::Liability, "BHD", 3600.0);
    ca.rate_type = RateType::Admin;
    ca.product = "CURRENT_ACCOUNT".into(); // 36 behavioural months by parameter

    // Nothing set: all core, evenly over the life, as it has always been slotted.
    let even = nmd::profile(&ca, &p);
    assert_eq!(even.len(), 36);
    assert!(even.iter().all(|(m, a)| m.is_some() && close(*a, 100.0, 1e-9)));

    // Half core: the other half sits overnight, in the gap and in EVE.
    p.gap.nmd_core_pct.insert("CURRENT_ACCOUNT".into(), 50.0);
    let half = nmd::profile(&ca, &p);
    assert_eq!(half[0], (None, 1800.0));
    assert!(close(half.iter().map(|x| x.1).sum::<f64>(), 3600.0, 1e-9));
    let g = gap::compute(std::slice::from_ref(&ca), &p, &BTreeMap::new(), as_of, None);
    let b = |label: &str| g.buckets.iter().find(|b| b.label == label).unwrap().liabilities;
    assert!(close(b("ON"), 1800.0, 1e-9));
    assert!(close(b("1M"), 50.0, 1e-9) && close(b("3M"), 100.0, 1e-9));
    let flows = eve::flows(&ca, &p, as_of);
    assert!(close(flows[0].0, 1.0 / 365.0, 1e-12) && close(flows[0].1, 1800.0, 1e-9));

    // A yearly decay front-loads the core: a fifth in the first year, the remainder at the end of the life.
    p.gap.nmd_decay_pct.insert("CURRENT_ACCOUNT".into(), 20.0);
    let decayed = nmd::profile(&ca, &p);
    let core: Vec<f64> = decayed.iter().filter(|x| x.0.is_some()).map(|x| x.1).collect();
    assert!(close(core.iter().sum::<f64>(), 1800.0, 1e-9), "the amounts still add up to the balance");
    assert!(close(core[..12].iter().sum::<f64>(), 1800.0 * 0.2, 1e-9));
    assert!(close(core[35], 1800.0 * 0.8f64.powf(35.0 / 12.0), 1e-9));
    assert!(core[0] > core[1] && core[1] > core[34]);

    // No core at all: everything overnight.
    p.gap.nmd_core_pct.insert("CURRENT_ACCOUNT".into(), 0.0);
    assert_eq!(nmd::profile(&ca, &p), vec![(None, 3600.0)]);

    // A shorter repricing profile holds less value at risk: EVE gains less from a rise when the
    // deposit is treated as overnight money.
    let curves = flat("BHD", 5.0);
    let short = eve::compute(std::slice::from_ref(&ca), &p, &BTreeMap::new(), &curves, as_of, RateScenario::Custom, 100.0, None).delta;
    let long = eve::compute(std::slice::from_ref(&ca), &AlmParams::default(), &BTreeMap::new(), &curves, as_of, RateScenario::Custom, 100.0, None).delta;
    assert!(long > short && short >= 0.0, "long {long} short {short}");
}

#[test]
fn a_dynamic_balance_sheet_grows_and_funds_the_growth() {
    let as_of = d(AS_OF);
    let mut p = AlmParams::default();
    let curves = flat("BHD", 4.0);
    let loan = pos("loan", BsSide::Asset, "BHD", 1200.0);
    let constant = nii::compute(std::slice::from_ref(&loan), &p, &BTreeMap::new(), &curves, as_of, RateScenario::Base, 0.0, None);
    assert_eq!(constant.mode, BalanceSheetMode::Constant);

    p.nii.set_mode(BalanceSheetMode::Dynamic);
    let flat_growth = nii::compute(std::slice::from_ref(&loan), &p, &BTreeMap::new(), &curves, as_of, RateScenario::Base, 0.0, None);
    assert!(close(flat_growth.nii, constant.nii, 1e-12), "no growth set: the constant balance sheet");

    p.nii.growth_assets_pct = 12.0;
    let grown = nii::compute(std::slice::from_ref(&loan), &p, &BTreeMap::new(), &curves, as_of, RateScenario::Base, 0.0, None);
    assert_eq!(grown.mode, BalanceSheetMode::Dynamic);
    assert!(grown.income > constant.income, "more loans earn more");
    assert!(grown.expense > 0.0, "the growth is funded at the one-month rate");
    assert!(grown.nii > constant.nii, "5% loans funded at 4% add income");
    let balancing = grown.by_book.iter().find(|b| b.key == "BALANCING").unwrap();
    assert!(balancing.nii < 0.0);
    // Month eleven: the loan book is 1200 * 1.12^(11/12).
    let last = &grown.months[11];
    let days = (alm::schedule::add_months(as_of, 12) - alm::schedule::add_months(as_of, 11)).num_days() as f64;
    assert!(close(last.income, 1200.0 * 1.12f64.powf(11.0 / 12.0) * 0.05 * days / 365.0, 1e-9));

    // A product's own rate beats the side's.
    p.nii.growth_by_product.insert("TEST".into(), 0.0);
    let held = nii::compute(std::slice::from_ref(&loan), &p, &BTreeMap::new(), &curves, as_of, RateScenario::Base, 0.0, None);
    assert!(close(held.nii, constant.nii, 1e-12));
    p.nii.growth_by_product.clear();

    // Side by side: three assumptions, whichever one the parameters select.
    let rows = nii::modes(std::slice::from_ref(&loan), &p, &BTreeMap::new(), &curves, as_of, RateScenario::Custom, 100.0, None);
    assert_eq!(rows.iter().map(|r| r.mode).collect::<Vec<_>>(), [BalanceSheetMode::Constant, BalanceSheetMode::RunOff, BalanceSheetMode::Dynamic]);
    assert!(close(rows[0].base_nii.unwrap(), constant.nii, 1e-12));
    assert!(close(rows[2].base_nii.unwrap(), grown.nii, 1e-12));
    let products = nii::products(std::slice::from_ref(&loan), &p, &BTreeMap::new());
    assert_eq!((products[0].product.as_str(), products[0].side), ("TEST", BsSide::Asset));
}

#[test]
fn a_trial_hedge_starts_at_par_and_takes_the_rate_risk_it_is_sized_for() {
    let as_of = d(AS_OF);
    let p = AlmParams::default();
    let curves = flat("BHD", 5.0);
    let none = BTreeMap::new();
    let mut bond = pos("bond", BsSide::Asset, "BHD", 1000.0);
    bond.maturity = Some(d("2031-11-04"));
    bond.pay_months = Some(12);
    let mut book = vec![bond];

    let swap = TrialHedge { kind: HedgeKind::PayFixedSwap, ccy: "BHD".into(), notional: 1000.0, tenor_months: 60, rate: None, reset_months: None };
    assert!(close(hedge::fixed_rate(&swap, &curves), 5.0, 1e-12), "at the curve when no rate is given");
    let legs = hedge::positions(std::slice::from_ref(&swap), &curves, as_of);
    assert_eq!(legs.len(), 2);
    assert_eq!((legs[0].side, legs[0].rate_type, legs[0].maturity), (BsSide::Liability, RateType::Fixed, Some(d("2031-11-04"))));
    assert_eq!((legs[1].side, legs[1].rate_type, legs[1].next_repricing), (BsSide::Asset, RateType::Float, Some(d("2027-02-04"))));
    assert!(legs.iter().all(|l| l.book == hedge::HEDGE_BOOK && l.lcr_class.is_none() && l.nsfr_class.is_none() && l.hqla_level.is_none()));

    // Worth about nothing on today's curve: it moves no value until rates do.
    let unhedged = eve::compute(&book, &p, &none, &curves, as_of, RateScenario::Custom, 100.0, None);
    book.extend(legs);
    let hedged = eve::compute(&book, &p, &none, &curves, as_of, RateScenario::Custom, 100.0, None);
    assert!((hedged.base - unhedged.base).abs() < 0.005 * 1000.0, "base {} against {}", hedged.base, unhedged.base);
    // A five-year pay-fixed swap against a five-year bond takes nearly all of the loss from a rise.
    assert!(unhedged.delta < -40.0, "the bond alone loses: {}", unhedged.delta);
    assert!(hedged.delta.abs() < 0.1 * unhedged.delta.abs(), "hedged {} against {}", hedged.delta, unhedged.delta);

    // The gap: the fixed leg sits against the bond (1,826 days out, a day past the 5Y edge), the
    // floating leg reprices at its first reset (92 days, a day past the 3M edge), and the notionals
    // stay out of total assets.
    let g = gap::compute(&book, &p, &none, as_of, None);
    let b = |label: &str| g.buckets.iter().find(|b| b.label == label).unwrap().clone();
    assert!(close(g.total_assets, 1000.0, 1e-9));
    assert!(close(b("10Y+").assets, 1000.0, 1e-9) && close(b("10Y+").gap, 0.0, 1e-9), "{:?}", b("10Y+"));
    assert!(close(b("6M").gap, 1000.0, 1e-9), "{:?}", b("6M"));

    // NII: on today's curve the swap carries nothing; when rates rise the floating leg earns the move.
    let n0 = nii::compute(&book[..1], &p, &none, &curves, as_of, RateScenario::Base, 0.0, None);
    let n1 = nii::compute(&book, &p, &none, &curves, as_of, RateScenario::Base, 0.0, None);
    assert!((n1.nii - n0.nii).abs() < 1.0, "carry {}", n1.nii - n0.nii);
    assert!(close(n1.earning_assets, n0.earning_assets, 1e-9), "notionals are not earning assets");
    let up = nii::compute(&book, &p, &none, &curves, as_of, RateScenario::Custom, 100.0, None);
    assert!(up.nii - n1.nii > 5.0, "nine months of +100 bp on 1000: {}", up.nii - n1.nii);
    assert!(up.by_book.iter().any(|b| b.key == hedge::HEDGE_BOOK));

    // Receiving fixed is the mirror image; funding and investments sit against one-month money.
    let recv = TrialHedge { kind: HedgeKind::ReceiveFixedSwap, rate: Some(4.5), reset_months: Some(6), ..swap.clone() };
    let legs = hedge::positions(std::slice::from_ref(&recv), &curves, as_of);
    assert_eq!((legs[0].side, legs[0].rate, legs[1].side, legs[1].reset_months), (BsSide::Asset, 4.5, BsSide::Liability, Some(6)));
    let funding = TrialHedge { kind: HedgeKind::FixedFunding, ..swap };
    assert_eq!(hedge::reset_months(&funding), 1);
}

#[test]
fn key_rates_place_the_risk_on_the_curve_and_estimate_any_shape_of_shock() {
    let as_of = d(AS_OF);
    let p = AlmParams::default();
    let curves = flat("BHD", 5.0);
    let none = BTreeMap::new();
    let mut bond = pos("bond", BsSide::Asset, "BHD", 1000.0);
    bond.maturity = Some(d("2029-11-04")); // three years: the 5Y bucket
    let mut dep = pos("dep", BsSide::Liability, "BHD", 600.0);
    dep.maturity = Some(d("2027-11-04")); // one year: the 1Y bucket
    let book = [bond, dep];

    let base = eve::compute(&book, &p, &none, &curves, as_of, RateScenario::Base, 0.0, None);
    assert_eq!(base.key_rates.len(), 9);
    assert_eq!(base.key_rate_estimate, None);
    let k = |r: &domain::EveResult, label: &str| r.key_rates.iter().find(|k| k.label == label).unwrap().clone();
    let (y5, y1) = (k(&base, "5Y"), k(&base, "1Y"));
    assert!(y5.assets < 0.0 && y5.liabilities == 0.0, "the bond loses when its rate rises: {y5:?}");
    assert!(y1.liabilities > 0.0 && y1.assets == 0.0, "the deposit is worth less, which EVE gains: {y1:?}");
    assert!(close(y1.net, 630.0 / 1.05 * 1.0 / 1.05 * 1e-4, 1e-9), "t * PV / (1 + r) per basis point: {}", y1.net);
    assert!(base.key_rates.iter().filter(|k| k.label != "5Y" && k.label != "1Y").all(|k| k.net == 0.0));
    assert_eq!(y5.estimate, None);

    // One basis point at one bucket moves EVE by that bucket's key rate.
    for label in ["1Y", "5Y"] {
        let one = eve::compute(&book, &p, &none, &curves, as_of, RateScenario::Custom, CustomShock::from_labels(1.0, &[label.to_string()]).unwrap(), None);
        assert!((one.delta - k(&base, label).net).abs() < 1e-3 * k(&base, label).net.abs(), "{label}: {} against {}", one.delta, k(&base, label).net);
    }

    // A shock aimed at some tenors: the duration formula has nothing to say, the key rates do.
    let aimed = eve::compute(&book, &p, &none, &curves, as_of, RateScenario::Custom, CustomShock::from_labels(100.0, &["5Y".to_string()]).unwrap(), None);
    assert_eq!(aimed.duration.estimate, None);
    let est = aimed.key_rate_estimate.unwrap();
    assert!(close(est, y5.net * 100.0, 1e-9), "only the 5Y bucket moves");
    assert!(close(k(&aimed, "5Y").estimate.unwrap(), est, 1e-9) && close(k(&aimed, "1Y").estimate.unwrap(), 0.0, 1e-12));
    assert!(aimed.delta < 0.0 && (est - aimed.delta).abs() < 0.05 * aimed.delta.abs(), "close to the full revaluation: {est} against {}", aimed.delta);

    // A steepener, where every tenor moves by a different amount.
    let steep = eve::compute(&book, &p, &none, &curves, as_of, RateScenario::Steepener, 0.0, None);
    let est = steep.key_rate_estimate.unwrap();
    assert!((est - steep.delta).abs() < 0.05 * steep.delta.abs().max(1.0), "{est} against {}", steep.delta);

    // On a flat curve a parallel move gives the duration formula's answer.
    let par = eve::compute(&book, &p, &none, &curves, as_of, RateScenario::Custom, 100.0, None);
    assert!(close(par.key_rate_estimate.unwrap(), par.duration.estimate.unwrap(), 1e-9));
}

#[test]
fn prepayment_and_early_withdrawal_shorten_fixed_contracts_in_the_rates_tools() {
    let as_of = d(AS_OF);
    let none = BTreeMap::new();
    let curves = flat("BHD", 5.0);
    let mut p = AlmParams::default();
    let mut loan = pos("loan", BsSide::Asset, "BHD", 1000.0);
    loan.product = "MORTGAGE".into();
    loan.maturity = Some(d("2031-11-04"));
    loan.origination = Some(d("2026-11-04"));
    loan.pay_months = Some(12);
    let mut dep = pos("dep", BsSide::Liability, "BHD", 1000.0);
    dep.product = "TERM_DEPOSIT".into();
    dep.maturity = Some(d("2028-11-04"));

    // Nothing set: the contractual schedule, flow for flow.
    assert_eq!(behaviour::speed(&loan, &p), 0.0);
    assert_eq!(behaviour::flows(&loan, as_of, 0.0), schedule::cash_flows(&loan, as_of));
    let plain_gap = gap::compute(std::slice::from_ref(&loan), &p, &none, as_of, None);
    let plain_eve = eve::compute(std::slice::from_ref(&loan), &p, &none, &curves, as_of, RateScenario::Custom, 200.0, None);
    let plain_nii = nii::compute(std::slice::from_ref(&loan), &p, &none, &curves, as_of, RateScenario::Custom, 100.0, None);
    let plain_base = nii::compute(std::slice::from_ref(&loan), &p, &none, &curves, as_of, RateScenario::Base, 0.0, None);

    // Ten percent of mortgages a year: speeds by product and side, moved by the direction of rates.
    p.behaviour.prepayment_pct.insert("MORTGAGE".into(), 10.0);
    p.behaviour.withdrawal_pct.insert("TERM_DEPOSIT".into(), 5.0);
    assert!(close(behaviour::speed(&loan, &p), 0.10, 1e-12) && close(behaviour::speed(&dep, &p), 0.05, 1e-12));
    let mut floater = loan.clone();
    floater.rate_type = RateType::Float;
    assert_eq!(behaviour::speed(&floater, &p), 0.0, "only fixed-rate contracts");
    assert_eq!((behaviour::multiplier(&loan, &p, 1.0), behaviour::multiplier(&loan, &p, -1.0), behaviour::multiplier(&loan, &p, 0.0)), (0.8, 1.2, 1.0));
    assert_eq!((behaviour::multiplier(&dep, &p, 1.0), behaviour::multiplier(&dep, &p, -1.0)), (1.2, 0.8));

    // The flows: something leaves every month, each contractual payment is cut to what survives,
    // and the principal still adds up to the balance.
    let flows = behaviour::flows(&loan, as_of, 0.10);
    assert!(close(flows.iter().map(|f| f.principal).sum::<f64>(), 1000.0, 1e-9));
    let first = &flows[0];
    assert_eq!(first.date, d("2026-12-04"));
    assert!(close(first.principal, 1000.0 * (1.0 - 0.9f64.powf(30.0 / 365.0)), 1e-9));
    let last = flows.last().unwrap();
    let years = (d("2031-11-04") - as_of).num_days() as f64 / 365.0;
    assert_eq!(last.date, d("2031-11-04"));
    assert!(close(last.principal, 1000.0 * 0.9f64.powf(years), 1e-9), "what is left at maturity: {}", last.principal);
    let mut linear = loan.clone();
    linear.amort = Amort::Linear;
    linear.pay_months = Some(6);
    assert!(close(behaviour::flows(&linear, as_of, 0.25).iter().map(|f| f.principal).sum::<f64>(), 1000.0, 1e-9), "an amortising loan too");

    // The gap: principal moves out of the far bucket into the near ones.
    let g = gap::compute(std::slice::from_ref(&loan), &p, &none, as_of, None);
    let far = |r: &domain::GapResult| r.buckets.iter().find(|b| b.label == "10Y+").unwrap().assets;
    let near = |r: &domain::GapResult| r.buckets.iter().filter(|b| ["1M", "3M", "6M", "1Y"].contains(&b.label.as_str())).map(|b| b.assets).sum::<f64>();
    assert!(close(far(&plain_gap), 1000.0, 1e-9) && near(&plain_gap) == 0.0, "contractually it all sits past five years");
    assert!(far(&g) < 600.0 && close(near(&g), 1000.0 * (1.0 - 0.9f64.powf(1.0)), 1e-6), "a tenth reprices within the year: {}", near(&g));
    assert!(close(g.buckets.iter().map(|b| b.assets).sum::<f64>(), 1000.0, 1e-9));

    // EVE: a shorter loan loses less when rates rise, even prepaying a little slower as they do.
    let e = eve::compute(std::slice::from_ref(&loan), &p, &none, &curves, as_of, RateScenario::Custom, 200.0, None);
    assert!(plain_eve.delta < 0.0 && e.delta < 0.0 && e.delta.abs() < plain_eve.delta.abs(), "{} against {}", e.delta, plain_eve.delta);
    assert!(e.duration.d_assets < plain_eve.duration.d_assets);
    // The direction matters: with no slowdown when rates rise the loan is shorter still.
    let mut steady = p.clone();
    steady.behaviour.prepayment_when_rates_rise = 1.0;
    let e1 = eve::compute(std::slice::from_ref(&loan), &steady, &none, &curves, as_of, RateScenario::Custom, 200.0, None);
    assert!(e1.delta.abs() < e.delta.abs(), "{} against {}", e1.delta, e.delta);

    // NII on a constant balance sheet: nothing changes on today's curve (what leaves is replaced
    // like for like), but under a rise the prepaid share already earns the higher rate.
    let n0 = nii::compute(std::slice::from_ref(&loan), &p, &none, &curves, as_of, RateScenario::Base, 0.0, None);
    assert!(close(n0.nii, plain_base.nii, 1e-12));
    let n1 = nii::compute(std::slice::from_ref(&loan), &p, &none, &curves, as_of, RateScenario::Custom, 100.0, None);
    assert!(close(plain_nii.nii, plain_base.nii, 1e-12), "a fixed loan past the horizon ignores the rise");
    assert!(n1.nii > plain_nii.nii, "{} against {}", n1.nii, plain_nii.nii);
    // Running off, what is prepaid goes to the balancing book.
    p.nii.set_mode(BalanceSheetMode::RunOff);
    let off = nii::compute(std::slice::from_ref(&loan), &p, &none, &curves, as_of, RateScenario::Base, 0.0, None);
    assert!(off.by_book.iter().any(|b| b.key == "BALANCING" && b.nii > 0.0));
    assert!(close(off.earning_assets, plain_base.earning_assets, 1e-6), "the cash is still an earning asset");

    // What each lever acts on.
    let cover = coverage::compute(&[loan, dep], &p, &none, as_of);
    assert!(close(cover["behaviour.prepayment_pct.MORTGAGE"], 1000.0, 1e-9) && close(cover["behaviour.withdrawal_pct.TERM_DEPOSIT"], 1000.0, 1e-9));
    assert_eq!(cover["behaviour.prepayment_pct.PERSONAL"], 0.0);
}
