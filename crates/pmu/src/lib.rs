//! The bond portfolio engine of the PMU desk: effective-interest accrual and amortisation of each
//! purchase lot, the FTP charge locked at purchase, swap carry and valuation, repo interest, DV01
//! and rate shocks, cash flows, and the income forecast. Pure functions over the portfolio and the
//! market, like the `alm` crate; the server loads the rows and serves what these return.

pub mod bond;
pub mod booking;
pub mod eval;
pub mod ledger;
pub mod market;

use chrono::{Duration, NaiveDate};
use domain::{CashFlowRow, PmuBondDetail, PmuForecast, PmuFunding, PmuIncome, PmuOverview, PmuPositions, PmuRisk, RowLevel};

pub use eval::{Portfolio, Snapshot};
pub use market::{Ftp, Market};

use eval::{daily_series, forecast, income, income_by_book, income_by_bond, month_start};

pub fn overview(m: &Market, p: &Portfolio, on: NaiveDate, previous: Option<NaiveDate>) -> PmuOverview {
    let snap = Snapshot::new(m, p, on);
    let inc = income(m, p, month_start(on), on);
    let mtd = inc.total;
    let fc = forecast(m, p, on, false);
    let headline_prev = previous.map(|d| {
        let s = Snapshot::new(m, p, d);
        s.headline(&income(m, p, month_start(d), d).total)
    });
    let receipts: f64 = snap.receipts().iter().map(|x| x.a + x.b).sum();
    let mut concentration: Vec<_> = snap.rows(&inc).into_iter().filter(|r| r.level == RowLevel::Bond).collect();
    concentration.sort_by(|a, b| b.face_bhd.total_cmp(&a.face_bhd));
    concentration.truncate(12);
    PmuOverview {
        as_of: on,
        previous,
        headline: snap.headline(&mtd),
        headline_prev,
        by_book: snap.book_figures(),
        monthly: fc.monthly,
        maturity_now: snap.maturity_buckets(false),
        maturity_year_end: snap.maturity_buckets(true),
        concentration,
        repo: snap.repo_summary(receipts),
        warnings: snap.warnings(),
    }
}

pub fn positions(m: &Market, p: &Portfolio, on: NaiveDate, from: NaiveDate, to: NaiveDate) -> PmuPositions {
    let snap = Snapshot::new(m, p, on);
    let inc = income(m, p, from, to);
    let mtd = income(m, p, month_start(on), on).total;
    PmuPositions { as_of: on, from, to, rows: snap.rows(&inc), swaps: snap.swap_views(), headline: snap.headline(&mtd) }
}

pub fn income_view(m: &Market, p: &Portfolio, from: NaiveDate, to: NaiveDate) -> PmuIncome {
    let run = income(m, p, from, to);
    PmuIncome { from, to, total: run.total, daily: daily_series(&run, from, to), by_bond: income_by_bond(p, &run), by_book: income_by_book(p, &run) }
}

pub fn forecast_view(m: &Market, p: &Portfolio, on: NaiveDate, reinvest: bool) -> PmuForecast {
    let fc = forecast(m, p, on, reinvest);
    let snap = Snapshot::new(m, p, on);
    let (face, hedged) = snap.year_end_face();
    PmuForecast {
        as_of: on,
        reinvest,
        full_year: fc.full_year,
        year_to_date: fc.year_to_date,
        rest_of_year: fc.rest_of_year,
        monthly: fc.monthly,
        periods: fc.periods,
        by_book: fc.by_book,
        year_end_face_bhd: face,
        year_end_hedged_face_bhd: hedged,
    }
}

pub fn risk(m: &Market, p: &Portfolio, on: NaiveDate) -> PmuRisk {
    let snap = Snapshot::new(m, p, on);
    let inc = income(m, p, month_start(on), on);
    let (books, shocks) = snap.shocks();
    let unhedged = snap.rows(&inc).into_iter().filter(|r| r.level == RowLevel::Bond && r.hedged_face_bhd < 0.5).collect();
    let (face, hedged) = snap.year_end_face();
    PmuRisk {
        as_of: on,
        hedge_pairs: snap.hedge_pairs(),
        unhedged,
        dv01_buckets: snap.dv01_buckets(),
        books,
        shocks,
        by_book: snap.book_figures(),
        yield_by_year: snap.yield_by_year(),
        hedged_by_ccy: snap.hedged_by_ccy(),
        maturity_now: snap.maturity_buckets(false),
        maturity_year_end: snap.maturity_buckets(true),
        headline: snap.headline(&inc.total),
        year_end_face_bhd: face,
        year_end_hedged_face_bhd: hedged,
    }
}

pub fn funding(m: &Market, p: &Portfolio, on: NaiveDate) -> PmuFunding {
    let snap = Snapshot::new(m, p, on);
    let receipts = snap.receipts();
    let total: f64 = receipts.iter().map(|x| x.a + x.b).sum();
    PmuFunding {
        as_of: on,
        summary: snap.repo_summary(total),
        repos: snap.repo_views(),
        by_remaining: snap.repo_by_remaining(),
        funding_flows: snap.funding_flows(),
        collateral_by_book: snap.collateral_by_book(),
        quotes: snap.quotes(),
        receipts,
        hedge_cash: snap.hedge_cash(),
        by_book_year: forecast(m, p, on, false).by_book,
    }
}

pub fn bond_detail(m: &Market, p: &Portfolio, on: NaiveDate, isin: &str) -> Option<PmuBondDetail> {
    let bond = p.bonds.get(isin)?.clone();
    let snap = Snapshot::new(m, p, on);
    let inc = income(m, p, month_start(on), on);
    let lots = snap.rows(&inc).into_iter().filter(|r| r.level == RowLevel::Lot && r.isin.as_deref() == Some(isin)).collect();
    let swaps = snap.swap_views().into_iter().filter(|s| s.isins.iter().any(|x| x == isin)).collect();
    let repos = snap.repo_views().into_iter().filter(|r| r.isins.iter().any(|x| x == isin)).collect();
    let cash_flows = eval::bond_cash_flows(m, p, isin, on, on + Duration::days(3 * 365))
        .into_iter()
        .map(|(date, kind, amount_local, amount_bhd)| CashFlowRow { date, kind, amount_local, amount_bhd })
        .collect();
    let sales = booking::sale_views(m, p, isin);
    Some(PmuBondDetail { bond, lots, sales, swaps, repos, cash_flows })
}
