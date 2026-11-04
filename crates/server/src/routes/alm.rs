//! The ALM workbench: the bank's parameter set, the banking-book snapshots, and the calculators
//! run on one snapshot with either the saved parameters or an unsaved what-if set. Every handler
//! checks `can_see_alm()` first and answers 404 otherwise, so an empty book never computes a ratio
//! of nothing and never fills the cache with one.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::State;
use axum::{Extension, Json};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{AlmParams, AlmParamsResponse, AlmParamsVersion, AlmRequest, AlmSnapshot, BsPosition, LcrResult};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use sqlx::PgConnection;
use sqlx::types::Json as Jsonb;

use crate::auth::AuthUser;
use crate::db;
use crate::error::ApiError;
use crate::state::AppState;

async fn entitled(conn: &mut PgConnection) -> Result<(), ApiError> {
    let ok: bool = sqlx::query_scalar("SELECT can_see_alm()").fetch_one(&mut *conn).await?;
    if ok { Ok(()) } else { Err(ApiError::NotFound) }
}

#[derive(sqlx::FromRow)]
struct VersionRow {
    version: i32,
    created_at: DateTime<Utc>,
    created_by: Option<String>,
    note: Option<String>,
    is_active: bool,
}

impl From<VersionRow> for AlmParamsVersion {
    fn from(r: VersionRow) -> Self {
        AlmParamsVersion { version: r.version, created_at: r.created_at, created_by: r.created_by, note: r.note, is_active: r.is_active }
    }
}

macro_rules! version_cols {
    () => {
        "p.version, p.created_at, u.display_name AS created_by, p.note, p.is_active"
    };
}

/// The active parameter set. Version 1 is stored as an empty document, which deserialises to
/// the application's Basel defaults; every section has defaults so older versions keep parsing.
pub async fn active_params(conn: &mut PgConnection) -> Result<(AlmParamsVersion, AlmParams), ApiError> {
    let row: Option<(i32, DateTime<Utc>, Option<String>, Option<String>, bool, Jsonb<AlmParams>)> = sqlx::query_as(concat!(
        "SELECT ",
        version_cols!(),
        ", p.params FROM alm_parameters p LEFT JOIN users u ON u.id = p.created_by WHERE p.is_active"
    ))
    .fetch_optional(&mut *conn)
    .await?;
    let (version, created_at, created_by, note, is_active, Jsonb(params)) = row.ok_or_else(|| ApiError::Internal(anyhow::anyhow!("no active ALM parameter set")))?;
    Ok((AlmParamsVersion { version, created_at, created_by, note, is_active }, params))
}

pub async fn params(State(state): State<AppState>, Extension(auth): Extension<AuthUser>) -> Result<Json<AlmParamsResponse>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    let (active, params) = active_params(&mut tx).await?;
    let history: Vec<VersionRow> = sqlx::query_as(concat!(
        "SELECT ",
        version_cols!(),
        " FROM alm_parameters p LEFT JOIN users u ON u.id = p.created_by ORDER BY p.version DESC LIMIT 20"
    ))
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(AlmParamsResponse { active, params, history: history.into_iter().map(Into::into).collect() }))
}

pub async fn snapshots(State(state): State<AppState>, Extension(auth): Extension<AuthUser>) -> Result<Json<Vec<AlmSnapshot>>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    let rows: Vec<(NaiveDate, NaiveDate, DateTime<Utc>, i64)> = sqlx::query_as(
        "SELECT s.as_of, s.business_date, s.generated_at, (SELECT count(*) FROM bs_positions b WHERE b.as_of = s.as_of) \
         FROM bs_snapshots s WHERE s.complete ORDER BY s.as_of DESC LIMIT 60",
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(rows.into_iter().map(|(as_of, business_date, generated_at, positions)| AlmSnapshot { as_of, business_date, generated_at, positions }).collect()))
}

/// The snapshot to compute on (date and generation time): the one asked for if it is complete,
/// else the newest complete one on or before the business date.
pub async fn resolve_as_of(conn: &mut PgConnection, requested: Option<NaiveDate>) -> Result<(NaiveDate, DateTime<Utc>), ApiError> {
    let found: Option<(NaiveDate, DateTime<Utc>)> = match requested {
        Some(d) => {
            sqlx::query_as("SELECT as_of, generated_at FROM bs_snapshots WHERE as_of = $1 AND complete").bind(d).fetch_optional(&mut *conn).await?
        }
        None => {
            let today = db::business_now(conn).await?.date_naive();
            sqlx::query_as("SELECT as_of, generated_at FROM bs_snapshots WHERE complete AND as_of <= $1 ORDER BY as_of DESC LIMIT 1")
                .bind(today)
                .fetch_optional(&mut *conn)
                .await?
        }
    };
    found.ok_or_else(|| ApiError::BadRequest("no complete banking-book snapshot is available for that date".into()))
}

#[derive(sqlx::FromRow)]
struct PositionRow {
    as_of: NaiveDate,
    position_id: String,
    side: String,
    book: String,
    segment: String,
    product: String,
    ccy: String,
    balance: Decimal,
    rate: Decimal,
    rate_type: String,
    index_code: Option<String>,
    spread: Decimal,
    reset_months: Option<i16>,
    pay_months: Option<i16>,
    amort: String,
    next_repricing: Option<NaiveDate>,
    maturity: Option<NaiveDate>,
    origination: Option<NaiveDate>,
    day_count: String,
    hqla_level: Option<String>,
    encumbered: bool,
    lcr_class: Option<String>,
    nsfr_class: Option<String>,
    beta: Decimal,
    behavioural_months: Option<i16>,
    undrawn: Decimal,
}

fn f(d: Decimal) -> f64 {
    d.to_f64().unwrap_or(0.0)
}

impl TryFrom<PositionRow> for BsPosition {
    type Error = anyhow::Error;

    fn try_from(r: PositionRow) -> Result<Self, Self::Error> {
        Ok(BsPosition {
            as_of: r.as_of,
            position_id: r.position_id,
            side: r.side.parse().map_err(anyhow::Error::msg)?,
            book: r.book,
            segment: r.segment,
            product: r.product,
            ccy: r.ccy.trim().to_string(),
            balance: f(r.balance),
            rate: f(r.rate),
            rate_type: r.rate_type.parse().map_err(anyhow::Error::msg)?,
            index_code: r.index_code,
            spread: f(r.spread),
            reset_months: r.reset_months,
            pay_months: r.pay_months,
            amort: r.amort.parse().map_err(anyhow::Error::msg)?,
            next_repricing: r.next_repricing,
            maturity: r.maturity,
            origination: r.origination,
            day_count: r.day_count.parse().map_err(anyhow::Error::msg)?,
            hqla_level: r.hqla_level.map(|h| h.parse().map_err(anyhow::Error::msg)).transpose()?,
            encumbered: r.encumbered,
            lcr_class: r.lcr_class,
            nsfr_class: r.nsfr_class,
            beta: f(r.beta),
            behavioural_months: r.behavioural_months,
            undrawn: f(r.undrawn),
        })
    }
}

pub async fn load_positions(conn: &mut PgConnection, as_of: NaiveDate) -> Result<Vec<BsPosition>, ApiError> {
    let rows: Vec<PositionRow> = sqlx::query_as(
        "SELECT as_of, position_id, side, book, segment, product, ccy::text AS ccy, balance, rate, rate_type, index_code, spread, reset_months, \
                pay_months, amort, next_repricing, maturity, origination, day_count, hqla_level, encumbered, lcr_class, nsfr_class, beta, \
                behavioural_months, undrawn \
         FROM bs_positions WHERE as_of = $1 ORDER BY position_id",
    )
    .bind(as_of)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(BsPosition::try_from).collect::<Result<_, _>>()?)
}

/// Units of BHD per one unit of each currency, from the latest spot on or before `as_of`.
pub async fn load_spots(conn: &mut PgConnection, as_of: NaiveDate) -> Result<BTreeMap<String, f64>, ApiError> {
    let rows: Vec<(String, Decimal)> =
        sqlx::query_as("SELECT DISTINCT ON (pair) pair, mid FROM fx_spots WHERE as_of <= $1 ORDER BY pair, as_of DESC").bind(as_of).fetch_all(&mut *conn).await?;
    Ok(rows.into_iter().filter_map(|(pair, mid)| pair.split_once('/').map(|(base, _)| (base.to_string(), f(mid)))).collect())
}

pub async fn lcr(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Json(req): Json<AlmRequest>) -> Result<Json<LcrResult>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    let (as_of, generated_at) = resolve_as_of(&mut tx, req.as_of).await?;
    let (version, params) = match req.params {
        Some(p) => (None, p),
        None => {
            let (v, p) = active_params(&mut tx).await?;
            (Some(v.version), p)
        }
    };
    if let Some(v) = version {
        if let Some(hit) = state.alm.lcr.read().expect("alm cache").get(&(as_of, generated_at, v)) {
            return Ok(Json((**hit).clone()));
        }
    }
    let positions = load_positions(&mut tx, as_of).await?;
    let spots = load_spots(&mut tx, as_of).await?;
    tx.commit().await?;
    let result = alm::lcr(&positions, &params, &spots, as_of, version);
    if let Some(v) = version {
        state.alm.lcr.write().expect("alm cache").insert((as_of, generated_at, v), Arc::new(result.clone()));
    }
    Ok(Json(result))
}
