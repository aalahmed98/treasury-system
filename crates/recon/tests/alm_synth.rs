use chrono::NaiveDate;
use domain::{BsSide, RateType};
use recon::alm_synth::{AlmCfg, Book, Market, digest, generate, snapshot_dates};

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn book(seed: u64) -> (Book, Market) {
    let cfg = AlmCfg { daily_history: 5, month_ends: 3, rate_history_days: 60, ..AlmCfg::default() };
    let business = d("2026-11-04");
    let market = Market::generate(seed, business, cfg.rate_history_days);
    (generate(&cfg, &market, business, seed), market)
}

#[test]
fn same_seed_same_book() {
    let (a, _) = book(42);
    let (b, _) = book(42);
    assert_eq!(digest(&a), digest(&b));
    assert_eq!(a.positions.len(), b.positions.len());
    let (c, _) = book(43);
    assert_ne!(digest(&a), digest(&c), "a different seed moves the balances");
}

#[test]
fn snapshots_and_cohorts_are_well_formed() {
    let cfg = AlmCfg { daily_history: 5, month_ends: 3, ..AlmCfg::default() };
    let dates = snapshot_dates(d("2026-11-04"), &cfg);
    assert_eq!(dates.last(), Some(&d("2026-11-04")));
    assert!(dates.contains(&d("2026-10-30")), "daily history");
    assert!(dates.contains(&d("2026-09-30")), "a month-end before the daily window");
    assert!(dates.contains(&d("2026-07-31")));
    assert!(dates.windows(2).all(|w| w[0] < w[1]), "sorted and unique");

    let (book, market) = book(7);
    assert_eq!(book.dates, dates);
    let last = book.positions.iter().filter(|p| p.as_of == d("2026-11-04")).collect::<Vec<_>>();
    assert!(last.len() > 60, "{} cohorts", last.len());
    // Balance sheet balances within a few percent in BHD terms (the random walk moves cohorts independently).
    let bhd = |side: BsSide| last.iter().filter(|p| p.side == side).map(|p| p.balance * market.spot(&p.ccy, p.as_of)).sum::<f64>();
    let assets = bhd(BsSide::Asset);
    let funding = bhd(BsSide::Liability);
    assert!((assets - 5.6e9).abs() / 5.6e9 < 0.08, "assets {assets}");
    assert!((assets - funding).abs() / assets < 0.1, "assets {assets} vs funding {funding}");
    for p in &last {
        assert!(p.balance >= 0.0 && p.undrawn >= 0.0);
        if let Some(m) = p.maturity {
            assert!(m > p.as_of, "{} matures in the past", p.position_id);
            assert!(p.origination.is_some_and(|o| o < m));
        }
        if p.rate_type == RateType::Float {
            let n = p.next_repricing.expect("float cohorts reprice");
            assert!(n > p.as_of && p.maturity.is_none_or(|m| n <= m), "{}: reset {n} vs maturity {:?}", p.position_id, p.maturity);
            assert!(p.index_code.is_some());
        }
        if p.side == BsSide::OffBalance {
            assert_eq!(p.balance, 0.0);
            assert!(p.undrawn > 0.0);
        }
    }
    assert!(last.iter().any(|p| p.hqla_level.is_some()), "liquid assets exist");
    assert!(book.depositors.iter().filter(|x| x.as_of == d("2026-11-04")).count() == 20);
    // Curves are positive, upward sloping on average, and spots keep the pegs.
    assert!(market.rates.iter().all(|r| r.rate > 0.0));
    assert!(market.rate("BHD", d("2026-11-04"), 3650) > market.rate("BHD", d("2026-11-04"), 1));
    assert!((market.spot("USD", d("2026-11-04")) - 0.376).abs() < 1e-9);
    assert!(market.spot("EUR", d("2026-11-04")) > 0.3 && market.spot("EUR", d("2026-11-04")) < 0.55);
}
