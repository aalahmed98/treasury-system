//! Hand-worked cases for every calculator beyond the LCR.

use std::collections::BTreeMap;

use alm::curve::{Curve, shock_bp};
use alm::funding::DepositorGroup;
use alm::nii::Curves;
use alm::{attribution, eve, funding, gap, ird, nii, nsfr, stress};
use chrono::NaiveDate;
use domain::{AlmParams, Amort, BsPosition, BsSide, CompareTo, DayCount, HqlaLevel, PeriodKind, PeriodLine, RateScenario, RateShock, RateType};

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

    let r = stress::compute(&[asset, cash, nmd, eur], &p, &spots, &curves, as_of, Some(-50.0), Some(1));
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
