//! Hand-worked cases for the PMU engine. Each expected figure can be checked on paper.

use std::collections::BTreeMap;

use alm::curve::Curve;
use chrono::{Duration, NaiveDate};
use domain::{Bond, BondDayCount, BondRateType, CurvePoint, FtpCurve, Income, Lot, PmuBook, Swap, SwapUnwind};
use pmu::bond::{self, accrued, clean_from_yield, dirty_from_yield, mod_duration, schedule, yield_from_clean};
use pmu::eval::{self, Portfolio};
use pmu::ledger::{swap_pv, walk_lot, walk_swap};
use pmu::{Ftp, Market};
use uuid::Uuid;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn bond(isin: &str, coupon: f64, freq: u32, dc: BondDayCount, issue: NaiveDate, maturity: NaiveDate) -> Bond {
    Bond {
        isin: isin.into(),
        name: format!("Test {isin}"),
        issuer: "Test".into(),
        issuer_type: "SOVEREIGN".into(),
        ccy: "USD".into(),
        coupon,
        coupon_freq: freq,
        day_count: dc,
        issue_date: issue,
        maturity,
        rate_type: BondRateType::Fixed,
        float_index: None,
        float_spread: 0.0,
        hqla_level: None,
    }
}

fn lot(isin: &str, settle: NaiveDate, face: f64, clean: f64, ftp: Option<f64>) -> Lot {
    Lot {
        id: Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("{isin}{settle}{face}").as_bytes()),
        book: "AFS USD".into(),
        isin: isin.into(),
        trade_date: settle,
        settle_date: settle,
        face,
        clean_price: clean,
        accrued_paid: 0.0,
        kondor_id: Some("K1".into()),
        eir: 0.0,
        ftp_rate: ftp,
        ftp_publication_id: None,
        status: "OPEN".into(),
        origin: "FEED".into(),
        sales: Vec::new(),
    }
}

/// A market on `as_of` with the dollar at 0.376 and every USD curve flat at `rate` throughout.
fn flat_market(as_of: NaiveDate, rate: f64) -> Market {
    let mut spots = BTreeMap::new();
    spots.insert(d(2020, 1, 1), BTreeMap::from([("USD".to_string(), 0.376)]));
    let mut hist = BTreeMap::new();
    hist.insert(d(2020, 1, 1), Curve::flat("USD", rate));
    Market { as_of, spots, history: BTreeMap::from([("USD".into(), hist)]), today: BTreeMap::from([("USD".into(), Curve::flat("USD", rate))]), prices: BTreeMap::new(), ftp: Ftp::default() }
}

#[test]
fn a_par_purchase_earns_its_coupon() {
    // 5% semi-annual bought at par on its issue date: 2.5% every half year, which compounds to
    // an annual effective rate of 1.025^2 - 1 = 5.0625% (on ACT/365 the half years are not quite
    // half of 365 days, hence the tolerance).
    let b = bond("US0000000001", 5.0, 2, BondDayCount::Thirty360, d(2026, 1, 15), d(2029, 1, 15));
    let e = bond::purchase_eir(&b, d(2026, 1, 15), 100.0, 5.0);
    assert!(close(e, 5.0625, 0.01), "{e}");
    // And the price at a 5% yield on a coupon date is par.
    assert!(close(clean_from_yield(&b, &schedule(&b), d(2026, 7, 15), 5.0), 100.0, 1e-9));
}

#[test]
fn accrued_interest_follows_the_day_count() {
    // 6% semi-annual 30/360 from 15 Jan: on 15 Apr, 90 of 180 days, so 3 x 90/180 = 1.5 per 100.
    let b = bond("US0000000002", 6.0, 2, BondDayCount::Thirty360, d(2026, 1, 15), d(2031, 1, 15));
    assert!(close(accrued(&b, &schedule(&b), d(2026, 4, 15), 6.0), 1.5, 1e-12));
    // ACT/360 pays the actual days: 15 Jan to 15 Jul is 181 days, so the coupon is 6 x 181/360,
    // and on 15 Apr (90 days in) 90/181 of it has accrued: 6 x 90/360 = 1.5.
    let mm = Bond { day_count: BondDayCount::Act360, ..b.clone() };
    assert!(close(bond::coupon_cash(BondDayCount::Act360, 6.0, 2, d(2026, 1, 15), d(2026, 7, 15)), 6.0 * 181.0 / 360.0, 1e-12));
    assert!(close(accrued(&mm, &schedule(&mm), d(2026, 4, 15), 6.0), 1.5, 1e-12));
    // ACT/ACT: 89 of 181 actual days (15 Jan to 14 Apr) of a 3.0 coupon.
    let aa = Bond { day_count: BondDayCount::ActAct, ..b };
    assert!(close(accrued(&aa, &schedule(&aa), d(2026, 4, 14), 6.0), 3.0 * 89.0 / 181.0, 1e-12));
}

#[test]
fn price_and_yield_invert_and_dv01_matches_duration() {
    let b = bond("US0000000003", 4.25, 2, BondDayCount::Thirty360, d(2024, 3, 1), d(2034, 3, 1));
    let s = schedule(&b);
    let on = d(2026, 11, 5);
    let clean = clean_from_yield(&b, &s, on, 5.2);
    assert!(clean < 100.0, "a 4.25% coupon below a 5.2% yield prices under par: {clean}");
    assert!(close(yield_from_clean(&b, &s, on, clean), 5.2, 1e-9));
    // DV01 of 1m face: the dirty price change for +1bp, against modified duration x price x 1bp.
    let dirty = dirty_from_yield(&b, &s, on, 5.2);
    let dv01 = (dirty_from_yield(&b, &s, on, 5.21) - dirty) * 10_000.0;
    let md = mod_duration(&b, &s, on, 5.2);
    assert!(close(dv01, -md * dirty * 10_000.0 * 0.0001, 0.01 * dv01.abs()), "{dv01} vs {}", -md * dirty);
    assert!(md > 5.0 && md < 7.0, "{md}");
}

#[test]
fn a_premium_bond_amortises_to_par_exactly_at_maturity() {
    // 6% annual ACT/ACT, three years, bought at 103 on its issue date. Over its life the coupons
    // are 18 per 100 and the amortisation is the premium given back: -3 per 100.
    let b = bond("US0000000004", 6.0, 1, BondDayCount::ActAct, d(2025, 6, 30), d(2028, 6, 30));
    let l = lot("US0000000004", d(2025, 6, 30), 1_000_000.0, 103.0, Some(4.0));
    let m = flat_market(d(2026, 11, 5), 4.0);
    let (mut coupon, mut amort, mut cash, mut principal, mut ftp) = (0.0, 0.0, 0.0, 0.0, 0.0);
    let mut first = None;
    let st = walk_lot(&m, &l, &b, d(2025, 6, 30), d(2030, 1, 1), |x| {
        coupon += x.coupon;
        amort += x.amortisation;
        cash += x.coupon_cash;
        principal += x.principal;
        ftp += x.ftp;
        first.get_or_insert(*x);
    });
    assert!(st.matured);
    assert!(close(coupon, 180_000.0, 1e-4), "{coupon}");
    assert!(close(cash, 180_000.0, 1e-6));
    assert!(close(principal, 1_000_000.0, 1e-6));
    assert!(close(amort, -30_000.0, 1e-4), "{amort}");
    assert!(close(st.ac_dirty, 0.0, 1e-9));
    // The first day: 1.03m at the effective rate for a day; FTP is the book value at 4% / 360.
    let f = first.unwrap();
    let e = bond::purchase_eir(&b, d(2025, 6, 30), 103.0, 6.0);
    assert!(close(f.coupon + f.amortisation, 1_030_000.0 * ((1.0 + e / 100.0).powf(1.0 / 365.0) - 1.0), 1e-6));
    assert!(close(f.ftp, 1_030_000.0 * 0.04 / 360.0, 1e-9));
    assert!(e < 6.0, "a premium lowers the yield below the coupon: {e}");
}

#[test]
fn ftp_locks_to_the_curve_published_on_the_trade_date() {
    let curve = |r: f64| vec![FtpCurve { ccy: "USD".into(), points: vec![CurvePoint { tenor: "1Y".into(), tenor_days: 365, rate: r }, CurvePoint { tenor: "5Y".into(), tenor_days: 1825, rate: r + 1.0 }] }];
    let ftp = Ftp::from_publications([(d(2026, 1, 10), 1, curve(5.0)), (d(2026, 3, 1), 2, curve(7.0))]);
    // A 3-year purchase on 12 Jan reads the 10 Jan publication, halfway between 1Y and 5Y.
    let (r, id) = ftp.rate("USD", d(2026, 1, 12), 3 * 365).unwrap();
    assert_eq!(id, 1);
    assert!(close(r, 5.5, 1e-12), "{r}");
    assert_eq!(ftp.rate("USD", d(2026, 3, 2), 365).unwrap().1, 2);
    assert!(ftp.rate("USD", d(2026, 1, 9), 365).is_none(), "nothing was published before 10 Jan");
    assert!(ftp.rate("SAR", d(2026, 3, 2), 365).is_none(), "no SAR curve was published");
    // The lot keeps 5.5% for life whatever is published later.
    let b = bond("US0000000005", 5.0, 2, BondDayCount::Thirty360, d(2026, 1, 12), d(2029, 1, 12));
    let l = lot("US0000000005", d(2026, 1, 12), 2_000_000.0, 100.0, Some(r));
    let mut m = flat_market(d(2026, 11, 5), 4.0);
    m.ftp = ftp;
    let mut charges = Vec::new();
    walk_lot(&m, &l, &b, d(2026, 6, 1), d(2026, 6, 2), |x| charges.push((x.ftp, x.book_value)));
    let (charge, _) = charges[0];
    assert!(close(charge, 2_000_000.0 * 0.055 / 360.0, 2.0), "{charge}");
}

#[test]
fn a_payer_swap_struck_at_par_is_worth_nothing_and_carries_the_spread() {
    let on = d(2026, 11, 5);
    let m = flat_market(on, 4.0);
    let c = Curve::flat("USD", 4.0);
    let mut s = Swap {
        id: Uuid::nil(),
        book: "AFS USD".into(),
        kondor_id: None,
        ccy: "USD".into(),
        notional: 10_000_000.0,
        start: on,
        maturity: d(2031, 11, 5),
        pay_fixed: true,
        fixed_rate: 0.0,
        fixed_freq: 1,
        fixed_day_count: BondDayCount::Act365,
        float_index: "SOFR".into(),
        float_tenor_months: 12,
        float_spread: 0.0,
        status: "ACTIVE".into(),
        hedges: vec![],
        unwinds: vec![],
    };
    // The par rate on one curve: (1 - df(T)) / sum of year fraction x df at each fixed date.
    let annuity: f64 = pmu::ledger::fixed_periods(&s).iter().map(|(a, b)| (*b - *a).num_days() as f64 / 365.0 * c.df((*b - on).num_days() as f64 / 365.0)).sum();
    s.fixed_rate = (1.0 - c.df((s.maturity - on).num_days() as f64 / 365.0)) / annuity * 100.0;
    assert!(close(swap_pv(&m, &s, on, 0.0), 0.0, 1e-4), "{}", swap_pv(&m, &s, on, 0.0));
    // Rates up: the payer gains.
    assert!(swap_pv(&m, &s, on, 1.0) > 0.0);

    // Carry: paying 4% fixed on ACT/360 against a 5% fixing earns 10m x 1% / 360 a day.
    let m5 = flat_market(on, 5.0);
    let mut s2 = Swap { start: d(2026, 1, 5), fixed_rate: 4.0, fixed_day_count: BondDayCount::Act360, ..s };
    let mut days = Vec::new();
    walk_swap(&m5, &s2, d(2026, 3, 1), d(2026, 3, 2), |_, c| days.push(c));
    // The fixing is the 1Y rate on a flat 5% curve.
    assert!(close(days[0], 10_000_000.0 * 0.01 / 360.0, 1e-6), "{}", days[0]);
    // Half unwound on 1 Mar: from 2 Mar the carry halves.
    s2.unwinds.push(SwapUnwind { date: d(2026, 3, 1), notional: 5_000_000.0, settlement: 12_000.0 });
    let mut after = Vec::new();
    walk_swap(&m5, &s2, d(2026, 3, 1), d(2026, 3, 2), |_, c| after.push(c));
    assert!(close(after[0], days[0], 1e-9) && close(after[1], days[0] / 2.0, 1e-9));
}

fn portfolio(on: NaiveDate) -> (Portfolio, Market) {
    let b1 = bond("US0000000006", 5.0, 2, BondDayCount::Thirty360, d(2024, 2, 15), d(2027, 2, 15));
    let b2 = bond("US0000000007", 4.0, 2, BondDayCount::Thirty360, d(2025, 5, 1), d(2032, 5, 1));
    let l1 = lot(&b1.isin, d(2025, 8, 15), 3_000_000.0, 99.5, Some(4.5));
    let l2 = lot(&b2.isin, d(2025, 11, 1), 5_000_000.0, 98.0, Some(4.8));
    let swap = Swap {
        id: Uuid::from_u128(7),
        book: "AFS USD".into(),
        kondor_id: Some("S1".into()),
        ccy: "USD".into(),
        notional: 5_000_000.0,
        start: d(2025, 11, 3),
        maturity: d(2032, 5, 1),
        pay_fixed: true,
        fixed_rate: 3.9,
        fixed_freq: 2,
        fixed_day_count: BondDayCount::Thirty360,
        float_index: "SOFR".into(),
        float_tenor_months: 3,
        float_spread: 0.0,
        status: "ACTIVE".into(),
        hedges: vec![(l2.id, 5_000_000.0)],
        unwinds: vec![],
    };
    let p = Portfolio {
        books: vec![PmuBook { code: "AFS USD".into(), name: "AFS dollar".into(), accounting: domain::Accounting::Afs, ccy: "USD".into() }],
        bonds: BTreeMap::from([(b1.isin.clone(), b1), (b2.isin.clone(), b2)]),
        lots: vec![l1, l2],
        swaps: vec![swap],
        repos: vec![],
        quotes: vec![],
    };
    let mut m = flat_market(on, 4.2);
    m.ftp = Ftp::from_publications([(d(2025, 1, 1), 1, vec![FtpCurve { ccy: "USD".into(), points: vec![CurvePoint { tenor: "1Y".into(), tenor_days: 365, rate: 4.6 }] }])]);
    (p, m)
}

#[test]
fn the_parts_of_income_add_up_and_the_days_sum_to_the_period() {
    let on = d(2026, 11, 5);
    let (p, m) = portfolio(on);
    let run = eval::income(&m, &p, d(2026, 10, 1), on);
    let t = run.total;
    assert!(close(t.eir_income, t.coupon + t.amortisation, 1e-6));
    assert!(close(t.carry, t.eir_income - t.ftp, 1e-6));
    assert!(close(t.net_nii, t.carry + t.irs, 1e-6));
    assert!(close(t.accounting_nii, t.eir_income + t.irs - t.repo, 1e-6));
    let mut sum = Income::default();
    for i in run.daily.values() {
        sum.add(i);
    }
    assert!(close(sum.net_nii, t.net_nii, 1e-6));
    assert_eq!(run.daily.len(), 36);
    // The swap's carry sits on the bond it hedges, and the hedged face is the swap's notional.
    let l2 = p.lots[1].id;
    assert!(run.by_lot[&l2].irs != 0.0);
    assert!(close(eval::hedged_face(&p, &p.lots[1], on), 5_000_000.0, 1e-6));
    assert!(close(eval::hedged_face(&p, &p.lots[0], on), 0.0, 1e-6));
    // The tree: one book, two bonds, two lots; the book adds up its bonds.
    let snap = eval::Snapshot::new(&m, &p, on);
    let rows = snap.rows(&run);
    assert_eq!(rows.len(), 5);
    assert!(close(rows[0].face_bhd, 8_000_000.0 * 0.376, 1e-6));
    assert!(close(rows[0].income.net_nii, t.net_nii, 1e-6));
    // Hedged: the bond's DV01 and the swap's nearly offset.
    let pairs = snap.hedge_pairs();
    assert_eq!(pairs.len(), 1);
    let hp = &pairs[0];
    assert!(hp.bond_dv01 < 0.0 && hp.irs_dv01 > 0.0, "{hp:?}");
    assert!(hp.net_dv01.abs() < 0.25 * hp.bond_dv01.abs(), "{hp:?}");
}

#[test]
fn the_forecast_runs_off_or_reinvests_and_the_two_agree_until_the_first_maturity() {
    let on = d(2026, 11, 5);
    let (p, m) = portfolio(on);
    let run_off = eval::forecast(&m, &p, on, false);
    let reinvest = eval::forecast(&m, &p, on, true);
    // The first bond matures 15 Feb 2027: this year is identical either way.
    assert!(close(run_off.full_year.net_nii, reinvest.full_year.net_nii, 1e-6));
    assert!(close(run_off.full_year.net_nii, run_off.year_to_date.net_nii + run_off.rest_of_year.net_nii, 1e-6));
    assert_eq!(run_off.monthly.iter().filter(|m| m.actual).count(), 11);
    // Next year the run-off loses the first bond's coupons after February; reinvesting keeps them.
    let y1 = |f: &eval::Forecast| f.periods[1].income.eir_income;
    assert!(y1(&reinvest) > y1(&run_off) + 1.0, "{} vs {}", y1(&reinvest), y1(&run_off));
    assert_eq!(run_off.periods.len(), 4);
    assert!(run_off.periods[0].partial && !run_off.periods[1].partial);
    // Year-end face: both bonds still held on 31 Dec.
    let snap = eval::Snapshot::new(&m, &p, on);
    let (face, hedged) = snap.year_end_face();
    assert!(close(face, 8_000_000.0 * 0.376, 1e-6) && close(hedged, 5_000_000.0 * 0.376, 1e-6));
    let _ = Duration::days(1);
}

fn sale(face: f64, price: f64, on: NaiveDate) -> domain::LotSale {
    domain::LotSale { id: Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("{on}{face}").as_bytes()), sale_ref: "PMU-S-1".into(), trade_date: on, settle_date: on, face, clean_price: price, kondor_id: None }
}

#[test]
fn a_half_sale_halves_the_amortised_cost_and_realises_the_gain() {
    // 6% annual, bought at 103 for 10m; half sold at 104 on 30 Jun 2026, a year in. On that day
    // the book value sold is half the lot's book value, so the gain is 5m x 1.04 less that.
    let b = bond("US0000000008", 6.0, 1, BondDayCount::ActAct, d(2025, 6, 30), d(2028, 6, 30));
    let plain = lot("US0000000008", d(2025, 6, 30), 10_000_000.0, 103.0, None);
    let mut sold = plain.clone();
    sold.sales.push(sale(5_000_000.0, 104.0, d(2026, 7, 15)));
    let m = flat_market(d(2026, 11, 5), 4.0);
    let mut before = None;
    walk_lot(&m, &plain, &b, d(2026, 7, 15), d(2026, 7, 15), |x| before = Some(*x));
    let before = before.unwrap();
    let (mut realised, mut after) = (0.0, None);
    let mut income_before_sale = (0.0, 0.0);
    walk_lot(&m, &plain, &b, d(2026, 1, 1), d(2026, 7, 14), |x| income_before_sale.0 += x.coupon + x.amortisation);
    walk_lot(&m, &sold, &b, d(2026, 1, 1), d(2026, 7, 14), |x| income_before_sale.1 += x.coupon + x.amortisation);
    assert!(close(income_before_sale.0, income_before_sale.1, 1e-6), "income before the sale is untouched");
    walk_lot(&m, &sold, &b, d(2026, 7, 15), d(2026, 7, 15), |x| {
        realised = x.realised;
        after = Some(*x);
    });
    let after = after.unwrap();
    assert!(close(after.face, 5_000_000.0, 1e-9));
    assert!(close(after.book_value, before.book_value / 2.0, 1e-6), "{} vs {}", after.book_value, before.book_value);
    assert!(close(after.accrued, before.accrued / 2.0, 1e-6));
    assert!(close(realised, 5_000_000.0 * 1.04 - before.book_value / 2.0, 1e-6), "{realised}");
    // The rest still amortises to par: the half kept redeems 5m, and its amortised cost reaches zero.
    let st = walk_lot(&m, &sold, &b, d(2028, 6, 30), d(2028, 6, 30), |x| assert!(close(x.principal, 5_000_000.0, 1e-9)));
    assert!(close(st.ac_dirty, 0.0, 1e-6));
    assert_eq!(sold.face_on(d(2026, 7, 14)), 10_000_000.0);
    assert_eq!(sold.face_on(d(2026, 7, 15)), 5_000_000.0);
}

#[test]
fn a_sale_draws_on_the_oldest_free_lots_and_skips_pledged_face() {
    let on = d(2026, 11, 5);
    let (mut p, m) = portfolio(on);
    // A second, younger lot of the 2032 bond, and a repo pledging 4m of the older one.
    let older = p.lots[1].clone();
    let mut younger = lot(&older.isin, d(2026, 3, 1), 2_000_000.0, 99.0, Some(4.8));
    younger.id = Uuid::from_u128(99);
    p.lots.push(younger.clone());
    p.repos.push(domain::Repo {
        id: Uuid::from_u128(5),
        counterparty: "Citibank N.A.".into(),
        kondor_id: None,
        ccy: "USD".into(),
        start: d(2026, 11, 1),
        end: d(2026, 12, 1),
        cash: 3_800_000.0,
        rate: 4.5,
        haircut_pct: 5.0,
        collateral: vec![(older.id, 4_000_000.0)],
        direction: "REPO".into(),
        closed_on: None,
        origin: "FEED".into(),
        collateral_isin: None,
        collateral_face: None,
        collateral_note: None,
    });
    // 5m older lot, 4m of it pledged: 1m free there, then the younger lot's 2m.
    let (legs, candidates, short) = pmu::booking::allocate(&m, &p, &older.isin, None, d(2026, 11, 9), 99.5, 2_500_000.0, None);
    assert_eq!(candidates.len(), 2);
    assert_eq!(legs.iter().map(|l| (l.lot_id, l.face)).collect::<Vec<_>>(), vec![(older.id, 1_000_000.0), (younger.id, 1_500_000.0)]);
    assert!(close(short, 0.0, 1e-9));
    let (_, _, short) = pmu::booking::allocate(&m, &p, &older.isin, None, d(2026, 11, 9), 99.5, 4_000_000.0, None);
    assert!(close(short, 1_000_000.0, 1e-6), "only 3m is free: {short}");
    // After the repo ends, all 7m is free.
    let (_, _, short) = pmu::booking::allocate(&m, &p, &older.isin, None, d(2026, 12, 2), 99.5, 7_000_000.0, None);
    assert!(close(short, 0.0, 1e-9));
    // Closing the repo early frees it from that day.
    p.repos[0].closed_on = Some(d(2026, 11, 8));
    let (_, _, short) = pmu::booking::allocate(&m, &p, &older.isin, None, d(2026, 11, 9), 99.5, 7_000_000.0, None);
    assert!(close(short, 0.0, 1e-9));
}

#[test]
fn reverse_repo_interest_is_income_and_a_closed_repo_stops_accruing() {
    let on = d(2026, 11, 5);
    let (mut p, m) = portfolio(on);
    let base = |dir: &str, closed: Option<NaiveDate>| domain::Repo {
        id: Uuid::from_u128(if dir == "REPO" { 6 } else { 7 }),
        counterparty: "JPMorgan Chase".into(),
        kondor_id: None,
        ccy: "USD".into(),
        start: d(2026, 10, 1),
        end: d(2026, 10, 31),
        cash: 3_600_000.0,
        rate: 4.0,
        haircut_pct: 0.0,
        collateral: vec![],
        direction: dir.into(),
        closed_on: closed,
        origin: "INTERNAL".into(),
        collateral_isin: Some("XS0000000001".into()),
        collateral_face: Some(4_000_000.0),
        collateral_note: None,
    };
    p.repos = vec![base("REVERSE", None)];
    // 3.6m at 4% over 360 for the 30 days of October after the start: 12,000 dollars received.
    let run = eval::income(&m, &p, d(2026, 10, 1), d(2026, 10, 31));
    assert!(close(run.total.repo, -12_000.0 * 0.376, 1e-6), "{:?}", run.total);
    assert!(run.total.accounting_nii > run.total.eir_income + run.total.irs);
    // A repo closed on 16 Oct accrues 15 days only.
    p.repos = vec![base("REPO", Some(d(2026, 10, 16)))];
    let run = eval::income(&m, &p, d(2026, 10, 1), d(2026, 10, 31));
    assert!(close(run.total.repo, 3_600_000.0 * 0.04 / 360.0 * 15.0 * 0.376, 1e-6), "{:?}", run.total);
}
