use std::collections::BTreeMap;

use alm::lcr::cap_adjustments;
use alm::schedule::{cash_flows, due_within};
use chrono::NaiveDate;
use domain::{AlmParams, Amort, BsPosition, BsSide, DayCount, HqlaLevel, RateType};

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn pos(id: &str, side: BsSide, ccy: &str, balance: f64) -> BsPosition {
    BsPosition {
        as_of: d("2026-11-04"),
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
fn level_2_caps_follow_the_basel_formula() {
    // Level 2 may be 40% of the stock (two thirds of Level 1), Level 2B 15% of the stock.
    let (adj_l2b, adj_l2) = cap_adjustments(100.0, 50.0, 30.0, 40.0, 15.0);
    assert!((adj_l2b - 5.0).abs() < 1e-9, "{adj_l2b}");
    assert!((adj_l2 - 8.333333).abs() < 1e-5, "{adj_l2}");
    let stock = 180.0 - adj_l2b - adj_l2;
    assert!((stock - 166.666667).abs() < 1e-5);
    // Level 2 is exactly two thirds of Level 1 and Level 2B exactly 15% of the stock afterwards.
    assert!(((50.0 + 30.0 - adj_l2b - adj_l2) - 2.0 / 3.0 * 100.0).abs() < 1e-6);
    assert!(((30.0 - adj_l2b) - 0.15 * stock).abs() < 1e-6);
    // Nothing to adjust when Level 2 is small.
    assert_eq!(cap_adjustments(100.0, 10.0, 5.0, 40.0, 15.0), (0.0, 0.0));
}

#[test]
fn a_hand_worked_lcr() {
    let as_of = d("2026-11-04");
    let mut cash = pos("cash", BsSide::Asset, "BHD", 1000.0);
    cash.hqla_level = Some(HqlaLevel::L1);
    let mut stable = pos("retail", BsSide::Liability, "BHD", 5000.0);
    stable.lcr_class = Some("RET_STABLE".into()); // 5% -> 250
    let mut fi = pos("fi-term", BsSide::Liability, "BHD", 1000.0);
    fi.lcr_class = Some("FI_WHOLESALE".into()); // 100% -> 1000, matures inside 30 days
    fi.maturity = Some(d("2026-11-14"));
    let mut far = pos("fi-far", BsSide::Liability, "BHD", 9999.0);
    far.lcr_class = Some("FI_WHOLESALE".into());
    far.maturity = Some(d("2027-06-01")); // beyond the horizon: no outflow
    let mut placement = pos("placement", BsSide::Asset, "BHD", 800.0);
    placement.lcr_class = Some("INFLOW_FI".into()); // 100%
    placement.maturity = Some(d("2026-11-09"));
    placement.rate = 0.0;
    let mut bond = pos("bond", BsSide::Asset, "BHD", 500.0);
    bond.hqla_level = Some(HqlaLevel::L2A); // 15% haircut -> 425, never an inflow
    bond.lcr_class = Some("INFLOW_SECURITIES".into());
    bond.maturity = Some(d("2026-11-10"));
    let mut pledged = pos("pledged", BsSide::Asset, "BHD", 400.0);
    pledged.hqla_level = Some(HqlaLevel::L1);
    pledged.encumbered = true; // not liquid
    let mut facility = pos("facility", BsSide::OffBalance, "BHD", 0.0);
    facility.undrawn = 2000.0;
    facility.lcr_class = Some("COMMIT_CREDIT_CORP".into()); // 10% -> 200

    let params = AlmParams::default();
    let r = alm::lcr(&[cash, stable, fi, far, placement, bond, pledged, facility], &params, &BTreeMap::new(), as_of, Some(1));
    let t = &r.total;
    assert!((t.stock - 1425.0).abs() < 1e-9, "stock {}", t.stock);
    assert!((t.total_outflows - 1450.0).abs() < 1e-9, "outflows {}", t.total_outflows);
    assert!((t.total_inflows - 800.0).abs() < 1e-9, "inflows {}", t.total_inflows);
    // 75% of 1450 = 1087.5 > 800: no cap bites.
    assert!((t.net_outflows - 650.0).abs() < 1e-9);
    assert!((t.ratio.unwrap() - 1425.0 / 650.0 * 100.0).abs() < 1e-9);
    assert_eq!(r.by_ccy.len(), 1);
    assert_eq!(r.params_version, Some(1));
}

#[test]
fn inflows_are_capped_and_currencies_convert() {
    let as_of = d("2026-11-04");
    let mut cash = pos("cash", BsSide::Asset, "USD", 100.0);
    cash.hqla_level = Some(HqlaLevel::L1);
    let mut dep = pos("dep", BsSide::Liability, "USD", 1000.0);
    dep.lcr_class = Some("NONFIN_WHOLESALE".into()); // 40% -> 400
    let mut placement = pos("pl", BsSide::Asset, "USD", 5000.0);
    placement.lcr_class = Some("INFLOW_FI".into());
    placement.maturity = Some(d("2026-11-20"));
    placement.rate = 0.0;
    let params = AlmParams::default();
    let spots: BTreeMap<String, f64> = [("USD".to_string(), 0.376)].into_iter().collect();
    let r = alm::lcr(&[cash, dep, placement], &params, &spots, as_of, None);
    let usd = &r.by_ccy[0];
    assert!((usd.capped_inflows - 300.0).abs() < 1e-9, "cap at 75% of outflows");
    assert!((usd.net_outflows - 100.0).abs() < 1e-9);
    assert!((usd.ratio.unwrap() - 100.0).abs() < 1e-9);
    // The book total is in BHD: everything scaled by the spot, so the ratio is unchanged.
    assert!((r.total.stock - 37.6).abs() < 1e-9);
    assert!((r.total.ratio.unwrap() - 100.0).abs() < 1e-6);
    assert_eq!(r.reporting_ccy, "BHD");
}

#[test]
fn no_net_outflows_means_no_ratio() {
    let mut cash = pos("cash", BsSide::Asset, "BHD", 100.0);
    cash.hqla_level = Some(HqlaLevel::L1);
    let r = alm::lcr(&[cash], &AlmParams::default(), &BTreeMap::new(), d("2026-11-04"), None);
    assert_eq!(r.total.ratio, None);
}

#[test]
fn schedules_follow_the_amortisation_profile() {
    let as_of = d("2026-11-04");
    let mut bullet = pos("b", BsSide::Asset, "BHD", 1200.0);
    bullet.maturity = Some(d("2027-11-04"));
    bullet.rate = 3.6; // 3.6% ACT/360 over 365 days = 43.8
    let f = cash_flows(&bullet, as_of);
    assert_eq!(f.len(), 1);
    assert!((f[0].principal - 1200.0).abs() < 1e-9);
    assert!((f[0].interest - 1200.0 * 0.036 * 365.0 / 360.0).abs() < 1e-9);

    let mut linear = bullet.clone();
    linear.pay_months = Some(3);
    linear.amort = Amort::Linear;
    let f = cash_flows(&linear, as_of);
    assert_eq!(f.len(), 4, "quarterly over a year");
    assert!(f.iter().all(|x| (x.principal - 300.0).abs() < 1e-9));
    assert!(f.windows(2).all(|w| w[0].interest > w[1].interest), "interest falls as principal repays");
    assert!((due_within(&linear, as_of, 30) - 0.0).abs() < 1e-9, "first payment is in February");
    assert!(due_within(&linear, as_of, 95) > 300.0);

    let mut annuity = bullet.clone();
    annuity.pay_months = Some(1);
    annuity.amort = Amort::Annuity;
    let f = cash_flows(&annuity, as_of);
    assert_eq!(f.len(), 12);
    let total_principal: f64 = f.iter().map(|x| x.principal).sum();
    assert!((total_principal - 1200.0).abs() < 1e-6, "annuity repays in full: {total_principal}");
    assert!(f.windows(2).all(|w| w[0].principal <= w[1].principal + 1e-6), "annuity principal rises over time");

    let mut nmd = bullet.clone();
    nmd.maturity = None;
    assert!(cash_flows(&nmd, as_of).is_empty());
}
