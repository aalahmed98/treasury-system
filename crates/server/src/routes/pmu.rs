//! The PMU desk's bond portfolio: overview, positions, income, forecast, risk and funding, each
//! computed by the `pmu` crate from the rows the caller may see. The portfolio and its market are
//! loaded once per business date and data stamp and kept; each page's result is cached under the
//! same stamp, so a page opened twice in a day is served from memory until something changes.

use std::collections::BTreeMap;
use std::sync::Arc;

use alm::curve::Curve;
use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use domain::{
    Accounting, Bond, BondDayCount, BondRateType, Calendar, ForecastQuery, Lot, PeriodQueryPmu, PmuBondDetail, PmuBook, PmuForecast, PmuFunding, PmuIncome,
    PmuOverview, PmuPositions, PmuRisk, Repo, RepoQuote, Swap, SwapUnwind,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::db;
use crate::error::ApiError;
use crate::routes::alm::load_curves;
use crate::routes::ftp;
use crate::state::{AlmKey, AppState, PmuData};

fn f(d: Decimal) -> f64 {
    d.to_f64().unwrap_or(0.0)
}

pub async fn can_see_pmu(conn: &mut PgConnection) -> Result<bool, ApiError> {
    Ok(sqlx::query_scalar("SELECT can_see_pmu()").fetch_one(&mut *conn).await?)
}

/// What the cached portfolio depends on; any change to the rows changes it.
async fn stamp(conn: &mut PgConnection) -> Result<String, ApiError> {
    let s: Option<String> = sqlx::query_scalar(
        "SELECT concat_ws('|', \
            (SELECT max(updated_at)::text FROM bond_lots), (SELECT count(*) FROM bond_lots), \
            (SELECT max(updated_at)::text FROM bonds), (SELECT max(updated_at)::text FROM pmu_books), \
            (SELECT max(updated_at)::text FROM irs), (SELECT count(*) FROM irs), (SELECT count(*) FROM irs_unwinds), (SELECT count(*) FROM irs_hedges), \
            (SELECT max(updated_at)::text FROM repos), (SELECT count(*) FROM repos), (SELECT count(*) FROM repo_collateral), \
            (SELECT max(as_of)::text FROM bond_prices), (SELECT count(*) FROM bond_prices), (SELECT max(quoted_at)::text FROM repo_quotes), \
            (SELECT max(id) FROM ftp_publications), (SELECT max(as_of)::text FROM market_rates), (SELECT max(as_of)::text FROM fx_spots))",
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(s.unwrap_or_default())
}

#[allow(clippy::type_complexity)]
async fn load(conn: &mut PgConnection, on: NaiveDate) -> Result<PmuData, ApiError> {
    let books: Vec<(String, String, String, String)> = sqlx::query_as("SELECT code, name, accounting, ccy::text FROM pmu_books WHERE active ORDER BY code").fetch_all(&mut *conn).await?;
    let books = books
        .into_iter()
        .map(|(code, name, acc, ccy)| PmuBook { code, name, accounting: acc.parse().unwrap_or(Accounting::Afs), ccy })
        .collect();

    let rows: Vec<(String, String, String, String, String, Decimal, i16, String, NaiveDate, NaiveDate, String, Option<String>, Decimal, Option<String>)> = sqlx::query_as(
        "SELECT isin, name, issuer, issuer_type, ccy::text, coupon, coupon_freq, day_count, issue_date, maturity, rate_type, float_index, float_spread, hqla_level FROM bonds",
    )
    .fetch_all(&mut *conn)
    .await?;
    let bonds: BTreeMap<String, Bond> = rows
        .into_iter()
        .map(|(isin, name, issuer, issuer_type, ccy, coupon, freq, dc, issue_date, maturity, rt, float_index, fs, hqla_level)| {
            (
                isin.clone(),
                Bond {
                    isin,
                    name,
                    issuer,
                    issuer_type,
                    ccy,
                    coupon: f(coupon),
                    coupon_freq: freq as u32,
                    day_count: dc.parse().unwrap_or(BondDayCount::Thirty360),
                    issue_date,
                    maturity,
                    rate_type: rt.parse().unwrap_or(BondRateType::Fixed),
                    float_index,
                    float_spread: f(fs),
                    hqla_level,
                },
            )
        })
        .collect();

    let rows: Vec<(Uuid, String, String, NaiveDate, NaiveDate, Decimal, Decimal, Decimal, Option<String>, Decimal, Option<Decimal>, Option<i32>, String)> = sqlx::query_as(
        "SELECT id, book, isin, trade_date, settle_date, face, clean_price, accrued_paid, kondor_id, eir, ftp_rate, ftp_publication_id, status FROM bond_lots \
         WHERE trade_date <= $1 ORDER BY trade_date, id",
    )
    .bind(on)
    .fetch_all(&mut *conn)
    .await?;
    let lots: Vec<Lot> = rows
        .into_iter()
        .map(|(id, book, isin, trade_date, settle_date, face, clean, acc, kondor_id, eir, ftp_rate, ftp_publication_id, status)| Lot {
            id,
            book,
            isin,
            trade_date,
            settle_date,
            face: f(face),
            clean_price: f(clean),
            accrued_paid: f(acc),
            kondor_id,
            eir: f(eir),
            ftp_rate: ftp_rate.map(f),
            ftp_publication_id,
            status,
        })
        .collect();

    let rows: Vec<(Uuid, String, Option<String>, String, Decimal, NaiveDate, NaiveDate, bool, Decimal, i16, String, String, i16, Decimal, String)> = sqlx::query_as(
        "SELECT id, book, kondor_id, ccy::text, notional, start_date, maturity, pay_fixed, fixed_rate, fixed_freq, fixed_day_count, float_index, float_tenor_months, float_spread, status \
         FROM irs WHERE start_date <= $1",
    )
    .bind(on)
    .fetch_all(&mut *conn)
    .await?;
    let hedges: Vec<(Uuid, Uuid, Decimal)> = sqlx::query_as("SELECT irs_id, lot_id, notional FROM irs_hedges").fetch_all(&mut *conn).await?;
    let unwinds: Vec<(Uuid, NaiveDate, Decimal, Decimal)> = sqlx::query_as("SELECT irs_id, date, notional, settlement FROM irs_unwinds WHERE date <= $1 ORDER BY date").bind(on).fetch_all(&mut *conn).await?;
    let swaps: Vec<Swap> = rows
        .into_iter()
        .map(|(id, book, kondor_id, ccy, notional, start, maturity, pay_fixed, fixed, ff, fdc, fi, ft, fs, status)| Swap {
            id,
            book,
            kondor_id,
            ccy,
            notional: f(notional),
            start,
            maturity,
            pay_fixed,
            fixed_rate: f(fixed),
            fixed_freq: ff as u32,
            fixed_day_count: fdc.parse().unwrap_or(BondDayCount::Thirty360),
            float_index: fi,
            float_tenor_months: ft as u32,
            float_spread: f(fs),
            status,
            hedges: hedges.iter().filter(|h| h.0 == id).map(|h| (h.1, f(h.2))).collect(),
            unwinds: unwinds.iter().filter(|u| u.0 == id).map(|u| SwapUnwind { date: u.1, notional: f(u.2), settlement: f(u.3) }).collect(),
        })
        .collect();

    let rows: Vec<(Uuid, String, Option<String>, String, NaiveDate, NaiveDate, Decimal, Decimal, Decimal)> =
        sqlx::query_as("SELECT id, counterparty, kondor_id, ccy::text, start_date, end_date, cash, rate, haircut_pct FROM repos WHERE start_date <= $1").bind(on).fetch_all(&mut *conn).await?;
    let coll: Vec<(Uuid, Uuid, Decimal)> = sqlx::query_as("SELECT repo_id, lot_id, face FROM repo_collateral").fetch_all(&mut *conn).await?;
    let repos = rows
        .into_iter()
        .map(|(id, counterparty, kondor_id, ccy, start, end, cash, rate, haircut)| Repo {
            id,
            counterparty,
            kondor_id,
            ccy,
            start,
            end,
            cash: f(cash),
            rate: f(rate),
            haircut_pct: f(haircut),
            collateral: coll.iter().filter(|c| c.0 == id).map(|c| (c.1, f(c.2))).collect(),
        })
        .collect();

    let rows: Vec<(NaiveDate, String, String, i32, Decimal, DateTime<Utc>)> = sqlx::query_as(
        "SELECT as_of, counterparty, tenor, tenor_days, spread_bp, quoted_at FROM repo_quotes WHERE as_of BETWEEN $1 AND $2",
    )
    .bind(on - Duration::days(30))
    .bind(on)
    .fetch_all(&mut *conn)
    .await?;
    let quotes = rows.into_iter().map(|(as_of, counterparty, tenor, tenor_days, s, quoted_at)| RepoQuote { as_of, counterparty, tenor, tenor_days, spread_bp: f(s), quoted_at }).collect();

    // The market: spots and curves back to the earliest settlement, today's curves, ten days of prices.
    let earliest = lots.iter().map(|l| l.settle_date).min().unwrap_or(on) - Duration::days(10);
    let rows: Vec<(NaiveDate, String, Decimal)> = sqlx::query_as("SELECT as_of, pair, mid FROM fx_spots WHERE as_of BETWEEN $1 AND $2 AND pair LIKE '%/BHD'").bind(earliest).bind(on).fetch_all(&mut *conn).await?;
    let mut spots: pmu::market::Spots = BTreeMap::new();
    for (d, pair, mid) in rows {
        if let Some(ccy) = pair.strip_suffix("/BHD") {
            spots.entry(d).or_default().insert(ccy.to_string(), f(mid));
        }
    }
    // Past fixings are needed only where a coupon floats: floating bonds, swaps, and the dollar
    // (repo quotes are read against SOFR).
    let mut fixing_ccys: Vec<String> = bonds.values().filter(|b| b.rate_type == BondRateType::Float).map(|b| b.ccy.clone()).chain(swaps.iter().map(|s| s.ccy.clone())).collect();
    fixing_ccys.push("USD".into());
    fixing_ccys.sort();
    fixing_ccys.dedup();
    let rows: Vec<(NaiveDate, String, i32, Decimal)> = sqlx::query_as(
        "SELECT DISTINCT ON (as_of, ccy, tenor_days) as_of, ccy::text, tenor_days, rate FROM market_rates \
         WHERE as_of BETWEEN $1 AND $2 AND ccy = ANY($3) ORDER BY as_of, ccy, tenor_days, index_code",
    )
    .bind(earliest)
    .bind(on)
    .bind(&fixing_ccys)
    .fetch_all(&mut *conn)
    .await?;
    let mut hist: BTreeMap<String, BTreeMap<NaiveDate, Vec<(f64, f64)>>> = BTreeMap::new();
    for (d, ccy, days, rate) in rows {
        hist.entry(ccy.trim().to_string()).or_default().entry(d).or_default().push((days as f64 / 365.0, f(rate)));
    }
    let history = hist.into_iter().map(|(ccy, days)| (ccy.clone(), days.into_iter().map(|(d, points)| (d, Curve { ccy: ccy.clone(), points })).collect())).collect();
    let today = load_curves(conn, on).await?.iter().map(|c| (c.ccy.clone(), Curve::from_rate_curve(c))).collect();
    let rows: Vec<(String, NaiveDate, Decimal, Decimal)> =
        sqlx::query_as("SELECT isin, as_of, clean_price, ytm FROM bond_prices WHERE as_of BETWEEN $1 AND $2").bind(on - Duration::days(10)).bind(on).fetch_all(&mut *conn).await?;
    let mut prices: BTreeMap<String, BTreeMap<NaiveDate, (f64, f64)>> = BTreeMap::new();
    for (isin, d, c, y) in rows {
        prices.entry(isin).or_default().insert(d, (f(c), f(y)));
    }
    let pubs = ftp::all_current(conn, on).await?;
    let ftp = pmu::Ftp::from_publications(pubs.into_iter().map(|(d, (id, curves))| (d, id, curves)));

    Ok(PmuData { portfolio: pmu::Portfolio { books, bonds, lots, swaps, repos, quotes }, market: pmu::Market { as_of: on, spots, history, today, prices, ftp } })
}

/// The caller's business date, the data stamp, and the portfolio (from memory when unchanged).
/// 404 for a user who may not see the portfolio, as for ALM.
pub async fn context(state: &AppState, conn: &mut PgConnection) -> Result<(NaiveDate, String, Arc<PmuData>), ApiError> {
    if !can_see_pmu(conn).await? {
        return Err(ApiError::NotFound);
    }
    let on = db::business_now(conn).await?.date_naive();
    let stamp = stamp(conn).await?;
    let key = format!("{on}|{stamp}");
    if let Ok(g) = state.pmu.lock()
        && let Some((k, data)) = g.as_ref()
        && *k == key
    {
        return Ok((on, stamp, data.clone()));
    }
    let data = Arc::new(load(conn, on).await?);
    if let Ok(mut g) = state.pmu.lock() {
        *g = Some((key, data.clone()));
    }
    Ok((on, stamp, data))
}

fn epoch() -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH
}

/// Serves a page's result from the cache, or computes it off the async runtime and keeps it.
async fn cached<T, F>(state: &AppState, tool: &'static str, on: NaiveDate, key: String, data: Arc<PmuData>, compute: F) -> Result<T, ApiError>
where
    T: serde::Serialize + serde::de::DeserializeOwned + Send + 'static,
    F: FnOnce(&PmuData) -> T + Send + 'static,
{
    let k: AlmKey = (tool, on, epoch(), 0, key);
    if let Some(hit) = state.alm.get::<T>(&k) {
        return Ok(hit);
    }
    let v = tokio::task::spawn_blocking(move || compute(&data)).await.map_err(|e| ApiError::Internal(e.into()))?;
    state.alm.put(k, &v);
    Ok(v)
}

async fn previous_business_day(conn: &mut PgConnection, on: NaiveDate) -> Result<NaiveDate, ApiError> {
    let holidays: Vec<NaiveDate> = sqlx::query_scalar("SELECT day FROM calendar_holidays").fetch_all(&mut *conn).await?;
    Ok(Calendar::new(holidays).add_business_days(on, -1))
}

pub async fn overview_for(state: &AppState, conn: &mut PgConnection) -> Result<PmuOverview, ApiError> {
    let (on, stamp, data) = context(state, conn).await?;
    let prev = previous_business_day(conn, on).await?;
    cached(state, "pmu-overview", on, stamp, data, move |d| pmu::overview(&d.market, &d.portfolio, on, Some(prev))).await
}

pub async fn overview(State(state): State<AppState>, Extension(auth): Extension<AuthUser>) -> Result<Json<PmuOverview>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let r = overview_for(&state, &mut tx).await?;
    tx.commit().await?;
    Ok(Json(r))
}

fn period(q: &PeriodQueryPmu, on: NaiveDate) -> Result<(NaiveDate, NaiveDate), ApiError> {
    let to = q.to.unwrap_or(on).min(on);
    let from = q.from.unwrap_or_else(|| NaiveDate::from_ymd_opt(on.year(), on.month(), 1).unwrap_or(on));
    if from > to {
        return Err(ApiError::BadRequest("the period starts after it ends".into()));
    }
    if (to - from).num_days() > 3 * 366 {
        return Err(ApiError::BadRequest("a period can cover at most three years".into()));
    }
    Ok((from, to))
}

pub async fn positions(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Query(q): Query<PeriodQueryPmu>) -> Result<Json<PmuPositions>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let (on, stamp, data) = context(&state, &mut tx).await?;
    tx.commit().await?;
    let (from, to) = period(&q, on)?;
    let r = cached(&state, "pmu-positions", on, format!("{stamp}|{from}|{to}"), data, move |d| pmu::positions(&d.market, &d.portfolio, on, from, to)).await?;
    Ok(Json(r))
}

pub async fn income(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Query(q): Query<PeriodQueryPmu>) -> Result<Json<PmuIncome>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let (on, stamp, data) = context(&state, &mut tx).await?;
    tx.commit().await?;
    let (from, to) = period(&q, on)?;
    let r = cached(&state, "pmu-income", on, format!("{stamp}|{from}|{to}"), data, move |d| pmu::income_view(&d.market, &d.portfolio, from, to)).await?;
    Ok(Json(r))
}

pub async fn forecast(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Query(q): Query<ForecastQuery>) -> Result<Json<PmuForecast>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let (on, stamp, data) = context(&state, &mut tx).await?;
    tx.commit().await?;
    let reinvest = q.reinvest.unwrap_or(false);
    let r = cached(&state, "pmu-forecast", on, format!("{stamp}|{reinvest}"), data, move |d| pmu::forecast_view(&d.market, &d.portfolio, on, reinvest)).await?;
    Ok(Json(r))
}

pub async fn risk(State(state): State<AppState>, Extension(auth): Extension<AuthUser>) -> Result<Json<PmuRisk>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let (on, stamp, data) = context(&state, &mut tx).await?;
    tx.commit().await?;
    let r = cached(&state, "pmu-risk", on, stamp, data, move |d| pmu::risk(&d.market, &d.portfolio, on)).await?;
    Ok(Json(r))
}

pub async fn funding(State(state): State<AppState>, Extension(auth): Extension<AuthUser>) -> Result<Json<PmuFunding>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let (on, stamp, data) = context(&state, &mut tx).await?;
    tx.commit().await?;
    let r = cached(&state, "pmu-funding", on, stamp, data, move |d| pmu::funding(&d.market, &d.portfolio, on)).await?;
    Ok(Json(r))
}

pub async fn bond(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Path(isin): Path<String>) -> Result<Json<PmuBondDetail>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let (on, _, data) = context(&state, &mut tx).await?;
    tx.commit().await?;
    let isin = isin.trim().to_uppercase();
    let r = tokio::task::spawn_blocking(move || pmu::bond_detail(&data.market, &data.portfolio, on, &isin)).await.map_err(|e| ApiError::Internal(e.into()))?;
    r.map(Json).ok_or(ApiError::NotFound)
}
