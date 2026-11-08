//! The ALM workbench: the bank's parameter set, the banking-book snapshots, and the calculators
//! run on one snapshot with either the saved parameters or an unsaved what-if set. Every handler
//! checks `can_see_alm()` first and answers 404 otherwise, so an empty book never computes a ratio
//! of nothing and never fills the cache with one. Official results (saved parameters) are cached
//! per tool, snapshot, generation time and parameter version; what-ifs are never cached.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use alm::curve::Curve;
use alm::funding::DepositorGroup;
use alm::lcr::Spots;
use alm::nii::Curves;
use alm::schedule::add_months;
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::{Extension, Json};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use domain::{
    AlmOverview, AlmParams, AlmParamsResponse, OverviewPrevious, AlmParamsVersion, AlmRequest, AlmSnapshot, AsOfQuery, AttributionResult, BsPosition, CompareTo, CurvePoint,
    EveResult, FundingResult, GapResult, LcrResult, LimitRow, NiiPeriodRow, NiiResult, NsfrResult, PeriodKind, PeriodLine, PositionsSummaryRow, RateCurve,
    RateScenario, SaveParamsRequest, StressResult, TrendPoint, TrendQuery, TrendResponse,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::PgConnection;
use sqlx::types::Json as Jsonb;

use crate::auth::AuthUser;
use crate::db;
use crate::error::ApiError;
use crate::routes::audit::{self, AuditCtx};
use crate::state::{AlmKey, AppState};

pub async fn entitled(conn: &mut PgConnection) -> Result<(), ApiError> {
    let ok: bool = sqlx::query_scalar("SELECT can_see_alm()").fetch_one(&mut *conn).await?;
    if ok { Ok(()) } else { Err(ApiError::NotFound) }
}

pub async fn can_see_alm(conn: &mut PgConnection) -> Result<bool, ApiError> {
    Ok(sqlx::query_scalar("SELECT can_see_alm()").fetch_one(&mut *conn).await?)
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

async fn history(conn: &mut PgConnection) -> Result<Vec<AlmParamsVersion>, ApiError> {
    let rows: Vec<VersionRow> = sqlx::query_as(concat!(
        "SELECT ",
        version_cols!(),
        " FROM alm_parameters p LEFT JOIN users u ON u.id = p.created_by ORDER BY p.version DESC LIMIT 20"
    ))
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn params(State(state): State<AppState>, Extension(auth): Extension<AuthUser>) -> Result<Json<AlmParamsResponse>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    let (active, params) = active_params(&mut tx).await?;
    let history = history(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(AlmParamsResponse { active, params, history }))
}

// ---------------------------------------------------------------------------------------------
// Saving a new parameter version
// ---------------------------------------------------------------------------------------------

fn validate(p: &AlmParams) -> Result<(), ApiError> {
    let bad = |m: String| ApiError::BadRequest(m);
    let pct = |name: &str, v: f64| -> Result<(), ApiError> {
        if !v.is_finite() || !(0.0..=100.0).contains(&v) { Err(bad(format!("{name} must be between 0 and 100"))) } else { Ok(()) }
    };
    for (k, v) in &p.lcr.haircuts {
        pct(&format!("haircut {k}"), *v)?;
    }
    for (k, v) in &p.lcr.outflow_rates {
        pct(&format!("run-off rate {k}"), *v)?;
    }
    for (k, v) in &p.lcr.inflow_rates {
        pct(&format!("inflow rate {k}"), *v)?;
    }
    pct("inflow cap", p.lcr.inflow_cap_pct)?;
    if !(0.0..100.0).contains(&p.lcr.cap_l2_total_pct) || !(0.0..100.0).contains(&p.lcr.cap_l2b_pct) {
        return Err(bad("Level 2 caps must be between 0 and 99".into()));
    }
    for v in [p.lcr.minimum, p.lcr.internal_minimum, p.lcr.early_warning, p.nsfr.minimum, p.nsfr.internal_minimum] {
        if !v.is_finite() || v <= 0.0 || v > 1000.0 {
            return Err(bad("ratio thresholds must be positive percentages".into()));
        }
    }
    for (k, f) in p.nsfr.asf.iter().chain(p.nsfr.rsf.iter()) {
        for v in f {
            pct(&format!("stable funding factor {k}"), *v)?;
        }
    }
    for (k, v) in &p.nsfr.hqla_rsf {
        pct(&format!("required stable funding {k}"), *v)?;
    }
    pct("off-balance required stable funding", p.nsfr.off_balance_rsf_pct)?;
    pct("encumbered required stable funding", p.nsfr.encumbered_rsf_pct)?;
    for (k, v) in &p.gap.cumulative_gap_limit_pct_assets {
        pct(&format!("gap limit {k}"), *v)?;
    }
    pct("EVE limit", p.gap.eve_limit_pct_tier1)?;
    pct("NII sensitivity limit", p.gap.nii_sensitivity_limit_pct)?;
    for (k, v) in &p.gap.nmd_behavioural_months {
        if !v.is_finite() || *v < 1.0 || *v > 240.0 {
            return Err(bad(format!("behavioural life {k} must be 1 to 240 months")));
        }
    }
    for (k, s) in &p.stress.shocks {
        for v in [s.parallel, s.short, s.long] {
            if !v.is_finite() || v.abs() > 2000.0 {
                return Err(bad(format!("shock {k} must be within 2000 basis points")));
            }
        }
    }
    if !p.tier1_capital.is_finite() || p.tier1_capital <= 0.0 {
        return Err(bad("Tier 1 capital must be positive".into()));
    }
    if !(1..=60).contains(&p.nii.horizon_months) {
        return Err(bad("NII horizon must be 1 to 60 months".into()));
    }
    pct("default deposit beta", p.nii.default_beta * 100.0)?;
    if p.nii.betas.len() > 100 {
        return Err(bad("too many deposit betas".into()));
    }
    for (k, v) in &p.nii.betas {
        if k.is_empty() || k.len() > 64 || !k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_' || c == '.') {
            return Err(bad(format!("'{k}' is not a product (or product.segment) code")));
        }
        pct(&format!("deposit beta for {k}"), v * 100.0)?;
    }
    pct("largest depositor limit", p.funding.top1_limit_pct)?;
    pct("ten largest depositors limit", p.funding.top10_limit_pct)?;
    pct("wholesale funding limit", p.funding.wholesale_share_limit_pct)?;
    if p.reporting_ccy.len() != 3 {
        return Err(bad("reporting currency must be a three-letter code".into()));
    }
    Ok(())
}

pub async fn save_params(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<SaveParamsRequest>,
) -> Result<(StatusCode, Json<AlmParamsVersion>), ApiError> {
    validate(&req.params)?;
    let note = req.note.trim();
    if note.is_empty() || note.chars().count() > 500 || note.chars().any(char::is_control) {
        return Err(ApiError::BadRequest("a note of up to 500 printable characters is required".into()));
    }
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    let version: i32 = sqlx::query_scalar("SELECT alm_parameters_save($1, $2)").bind(Jsonb(&req.params)).bind(note).fetch_one(&mut *tx).await?;
    let (active, _) = active_params(&mut tx).await?;
    tx.commit().await?;
    let ctx = AuditCtx::new(Some(peer), &headers);
    audit::write(&state.pool, Some(auth.user.id), &auth.user.username, "alm_params_save", &ctx, Some(serde_json::json!({ "version": version, "note": note }))).await;
    Ok((StatusCode::CREATED, Json(active)))
}

// ---------------------------------------------------------------------------------------------
// Snapshots and the book
// ---------------------------------------------------------------------------------------------

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

/// Units of the reporting currency per one unit of each currency, from the latest spot on or
/// before `as_of`.
pub async fn load_spots(conn: &mut PgConnection, as_of: NaiveDate) -> Result<Spots, ApiError> {
    let rows: Vec<(String, Decimal)> =
        sqlx::query_as("SELECT DISTINCT ON (pair) pair, mid FROM fx_spots WHERE as_of <= $1 ORDER BY pair, as_of DESC").bind(as_of).fetch_all(&mut *conn).await?;
    Ok(rows.into_iter().filter_map(|(pair, mid)| pair.split_once('/').map(|(base, _)| (base.to_string(), f(mid)))).collect())
}

/// The latest daily close of every curve on or before `as_of`.
pub async fn load_curves(conn: &mut PgConnection, as_of: NaiveDate) -> Result<Vec<RateCurve>, ApiError> {
    let rows: Vec<(String, String, String, i32, Decimal, NaiveDate)> = sqlx::query_as(
        "SELECT DISTINCT ON (ccy, tenor) ccy::text, index_code, tenor, tenor_days, rate, as_of FROM market_rates WHERE as_of <= $1 \
         ORDER BY ccy, tenor, as_of DESC",
    )
    .bind(as_of)
    .fetch_all(&mut *conn)
    .await?;
    Ok(group_curves(rows))
}

pub fn group_curves(rows: Vec<(String, String, String, i32, Decimal, NaiveDate)>) -> Vec<RateCurve> {
    let mut by: BTreeMap<String, RateCurve> = BTreeMap::new();
    for (ccy, index_code, tenor, tenor_days, rate, as_of) in rows {
        let ccy = ccy.trim().to_string();
        let c = by.entry(ccy.clone()).or_insert(RateCurve { ccy, index_code, as_of, points: Vec::new() });
        c.points.push(CurvePoint { tenor, tenor_days, rate: f(rate) });
    }
    by.into_values()
        .map(|mut c| {
            c.points.sort_by_key(|p| p.tenor_days);
            c
        })
        .collect()
}

pub struct Book {
    pub as_of: NaiveDate,
    pub generated_at: DateTime<Utc>,
    pub positions: Vec<BsPosition>,
    pub spots: Spots,
    pub curves: Curves,
    pub depositors: Vec<DepositorGroup>,
}

pub async fn load_book(conn: &mut PgConnection, requested: Option<NaiveDate>) -> Result<Book, ApiError> {
    let (as_of, generated_at) = resolve_as_of(conn, requested).await?;
    let positions = load_positions(conn, as_of).await?;
    let spots = load_spots(conn, as_of).await?;
    let curves = load_curves(conn, as_of).await?.iter().map(|c| (c.ccy.clone(), Curve::from_rate_curve(c))).collect();
    let rows: Vec<(String, String, String, Decimal)> =
        sqlx::query_as("SELECT group_name, segment, ccy::text, balance FROM bs_depositors WHERE as_of = $1 ORDER BY group_name").bind(as_of).fetch_all(&mut *conn).await?;
    let depositors = rows.into_iter().map(|(name, segment, ccy, balance)| DepositorGroup { name, segment, ccy: ccy.trim().to_string(), balance: f(balance) }).collect();
    Ok(Book { as_of, generated_at, positions, spots, curves, depositors })
}

/// The parameters to run with: the what-if from the request (never cached), else the active
/// saved version.
async fn choose_params(conn: &mut PgConnection, req: &AlmRequest) -> Result<(Option<i32>, AlmParams), ApiError> {
    match &req.params {
        Some(p) => Ok((None, p.clone())),
        None => {
            let (v, p) = active_params(conn).await?;
            Ok((Some(v.version), p))
        }
    }
}

/// Compute an official result through the cache, or a what-if directly.
fn official<T: Serialize + DeserializeOwned>(state: &AppState, tool: &'static str, book: &Book, version: Option<i32>, extra: String, compute: impl FnOnce() -> T) -> T {
    let Some(v) = version else { return compute() };
    let key: AlmKey = (tool, book.as_of, book.generated_at, v, extra);
    if let Some(hit) = state.alm.get::<T>(&key) {
        return hit;
    }
    let result = compute();
    state.alm.put(key, &result);
    result
}

async fn prepare(state: &AppState, auth: &AuthUser, req: &AlmRequest) -> Result<(Book, Option<i32>, AlmParams), ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    let book = load_book(&mut tx, req.as_of).await?;
    let (version, params) = choose_params(&mut tx, req).await?;
    tx.commit().await?;
    Ok((book, version, params))
}

// ---------------------------------------------------------------------------------------------
// The tools
// ---------------------------------------------------------------------------------------------

/// What each lever applies to in the snapshot, so the pages can show the ones with nothing to act on.
pub async fn coverage(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Json(req): Json<AlmRequest>) -> Result<Json<domain::AlmCoverage>, ApiError> {
    let (book, _version, params) = prepare(&state, &auth, &req).await?;
    let amounts = alm::coverage::compute(&book.positions, &params, &book.spots, book.as_of);
    Ok(Json(domain::AlmCoverage { as_of: book.as_of, reporting_ccy: params.reporting_ccy.clone(), amounts }))
}

pub async fn lcr(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Json(req): Json<AlmRequest>) -> Result<Json<LcrResult>, ApiError> {
    let (book, version, params) = prepare(&state, &auth, &req).await?;
    Ok(Json(official(&state, "lcr", &book, version, String::new(), || alm::lcr(&book.positions, &params, &book.spots, book.as_of, version))))
}

pub async fn nsfr(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Json(req): Json<AlmRequest>) -> Result<Json<NsfrResult>, ApiError> {
    let (book, version, params) = prepare(&state, &auth, &req).await?;
    Ok(Json(official(&state, "nsfr", &book, version, String::new(), || alm::nsfr::compute(&book.positions, &params, &book.spots, book.as_of, version))))
}

pub async fn gap(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Json(req): Json<AlmRequest>) -> Result<Json<GapResult>, ApiError> {
    let (book, version, params) = prepare(&state, &auth, &req).await?;
    Ok(Json(official(&state, "gap", &book, version, String::new(), || alm::gap::compute(&book.positions, &params, &book.spots, book.as_of, version))))
}

pub async fn funding(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Json(req): Json<AlmRequest>) -> Result<Json<FundingResult>, ApiError> {
    let (book, version, params) = prepare(&state, &auth, &req).await?;
    Ok(Json(official(&state, "funding", &book, version, String::new(), || {
        alm::funding::compute(&book.positions, &book.depositors, &params, &book.spots, book.as_of, version)
    })))
}

fn nii_for(book: &Book, params: &AlmParams, scenario: RateScenario, bp: f64, version: Option<i32>) -> NiiResult {
    let mut r = alm::nii::compute(&book.positions, params, &book.spots, &book.curves, book.as_of, scenario, bp, version);
    if scenario != RateScenario::Base {
        r.base_nii = Some(alm::nii::compute(&book.positions, params, &book.spots, &book.curves, book.as_of, RateScenario::Base, 0.0, version).nii);
    }
    r
}

pub async fn nii(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Json(req): Json<AlmRequest>) -> Result<Json<NiiResult>, ApiError> {
    let (book, version, params) = prepare(&state, &auth, &req).await?;
    let scenario = req.scenario.unwrap_or(RateScenario::Base);
    let bp = req.custom_shock_bp.unwrap_or(0.0);
    Ok(Json(official(&state, "nii", &book, version, format!("{scenario}:{bp}"), || nii_for(&book, &params, scenario, bp, version))))
}

pub async fn eve(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Json(req): Json<AlmRequest>) -> Result<Json<EveResult>, ApiError> {
    let (book, version, params) = prepare(&state, &auth, &req).await?;
    let scenario = req.scenario.unwrap_or(RateScenario::ParallelUp);
    let bp = req.custom_shock_bp.unwrap_or(0.0);
    Ok(Json(official(&state, "eve", &book, version, format!("{scenario}:{bp}"), || {
        alm::eve::compute(&book.positions, &params, &book.spots, &book.curves, book.as_of, scenario, bp, version)
    })))
}

pub async fn stress(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Json(req): Json<AlmRequest>) -> Result<Json<StressResult>, ApiError> {
    let (book, version, params) = prepare(&state, &auth, &req).await?;
    let bp = req.custom_shock_bp;
    Ok(Json(official(&state, "stress", &book, version, format!("{bp:?}"), || {
        alm::stress::compute(&book.positions, &params, &book.spots, &book.curves, book.as_of, bp, version)
    })))
}

pub async fn attribution(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Json(req): Json<AlmRequest>) -> Result<Json<AttributionResult>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    let (_, params) = choose_params(&mut tx, &req).await?;
    let compare = req.compare_to.unwrap_or(CompareTo::Prior);
    let latest: Option<NaiveDate> = sqlx::query_scalar("SELECT max(period) FROM alm_period WHERE kind = 'ACTUAL'").fetch_one(&mut *tx).await?;
    let period_b = req.period.or(latest).ok_or_else(|| ApiError::BadRequest("no income periods are available yet".into()))?;
    let (period_a, kind_a) = match compare {
        CompareTo::Prior => (add_months(period_b, -1), PeriodKind::Actual),
        CompareTo::Budget => (period_b, PeriodKind::Budget),
    };
    let rows: Vec<(NaiveDate, String, String, String, String, Decimal, Decimal)> = sqlx::query_as(
        "SELECT period, book, side, ccy::text, kind, nii, avg_balance FROM alm_period \
         WHERE (period = $1 AND kind = $2) OR (period = $3 AND kind = 'ACTUAL') ORDER BY period, book, side, ccy",
    )
    .bind(period_a)
    .bind(kind_a.as_str())
    .bind(period_b)
    .fetch_all(&mut *tx)
    .await?;
    let (as_of, _) = resolve_as_of(&mut tx, None).await?;
    let spots = load_spots(&mut tx, as_of).await?;
    tx.commit().await?;
    let lines: Vec<PeriodLine> = rows
        .into_iter()
        .map(|(period, book, side, ccy, kind, nii, avg)| {
            Ok::<_, anyhow::Error>(PeriodLine {
                period,
                book,
                side: side.parse().map_err(anyhow::Error::msg)?,
                ccy: ccy.trim().to_string(),
                kind: kind.parse().map_err(anyhow::Error::msg)?,
                nii: f(nii),
                avg_balance: f(avg),
            })
        })
        .collect::<Result<_, _>>()?;
    let a: Vec<PeriodLine> = lines.iter().filter(|l| l.period == period_a && l.kind == kind_a).cloned().collect();
    let b: Vec<PeriodLine> = lines.iter().filter(|l| l.period == period_b && l.kind == PeriodKind::Actual).cloned().collect();
    if a.is_empty() || b.is_empty() {
        return Err(ApiError::BadRequest(format!("no income lines for {} against {}", period_b.format("%b %Y"), period_a.format("%b %Y"))));
    }
    Ok(Json(alm::attribution::compute(&a, &b, period_a, period_b, compare, &params.reporting_ccy, &spots)))
}

pub async fn positions(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Query(q): Query<AsOfQuery>) -> Result<Json<Vec<PositionsSummaryRow>>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    let (as_of, _) = resolve_as_of(&mut tx, q.as_of).await?;
    let rows: Vec<(String, String, String, String, Decimal, Option<Decimal>, i64)> = sqlx::query_as(
        "SELECT book, side, ccy::text, rate_type, sum(balance), sum(balance * rate) / nullif(sum(balance), 0), count(*) \
         FROM bs_positions WHERE as_of = $1 GROUP BY book, side, ccy, rate_type ORDER BY side, book, ccy, rate_type",
    )
    .bind(as_of)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(
        rows.into_iter()
            .map(|(book, side, ccy, rate_type, balance, avg, cohorts)| {
                Ok::<_, anyhow::Error>(PositionsSummaryRow {
                    book,
                    side: side.parse().map_err(anyhow::Error::msg)?,
                    ccy: ccy.trim().to_string(),
                    rate_type: rate_type.parse().map_err(anyhow::Error::msg)?,
                    balance: f(balance),
                    avg_rate: avg.map(f).unwrap_or(0.0),
                    cohorts,
                })
            })
            .collect::<Result<_, _>>()?,
    ))
}

// ---------------------------------------------------------------------------------------------
// Trends and the overview block
// ---------------------------------------------------------------------------------------------

/// Official values of one metric per complete snapshot over the last `days`, through the cache.
async fn trend_points(state: &AppState, conn: &mut PgConnection, metric: &str, days: i64, version: i32, params: &AlmParams) -> Result<Vec<TrendPoint>, ApiError> {
    let today = db::business_now(conn).await?.date_naive();
    let snaps: Vec<(NaiveDate, DateTime<Utc>)> =
        sqlx::query_as("SELECT as_of, generated_at FROM bs_snapshots WHERE complete AND as_of <= $1 AND as_of > $2 ORDER BY as_of")
            .bind(today)
            .bind(today - Duration::days(days))
            .fetch_all(&mut *conn)
            .await?;
    let mut points = Vec::with_capacity(snaps.len());
    for (as_of, generated_at) in snaps {
        let key: AlmKey = (metric_tool(metric), as_of, generated_at, version, if metric == "nii" { "BASE:0".into() } else { String::new() });
        let value = match metric {
            "lcr" => match state.alm.get::<LcrResult>(&key) {
                Some(r) => r.total.ratio,
                None => {
                    let positions = load_positions(conn, as_of).await?;
                    let spots = load_spots(conn, as_of).await?;
                    let r = alm::lcr(&positions, params, &spots, as_of, Some(version));
                    state.alm.put(key, &r);
                    r.total.ratio
                }
            },
            "nsfr" => match state.alm.get::<NsfrResult>(&key) {
                Some(r) => r.ratio,
                None => {
                    let positions = load_positions(conn, as_of).await?;
                    let spots = load_spots(conn, as_of).await?;
                    let r = alm::nsfr::compute(&positions, params, &spots, as_of, Some(version));
                    state.alm.put(key, &r);
                    r.ratio
                }
            },
            _ => match state.alm.get::<NiiResult>(&key) {
                Some(r) => Some(r.nii),
                None => {
                    let positions = load_positions(conn, as_of).await?;
                    let spots = load_spots(conn, as_of).await?;
                    let curves: Curves = load_curves(conn, as_of).await?.iter().map(|c| (c.ccy.clone(), Curve::from_rate_curve(c))).collect();
                    let r = alm::nii::compute(&positions, params, &spots, &curves, as_of, RateScenario::Base, 0.0, Some(version));
                    state.alm.put(key, &r);
                    Some(r.nii)
                }
            },
        };
        points.push(TrendPoint { as_of, value });
    }
    Ok(points)
}

pub async fn trend_points_pub(state: &AppState, conn: &mut PgConnection, metric: &str, days: i64, version: i32, params: &AlmParams) -> Result<Vec<TrendPoint>, ApiError> {
    trend_points(state, conn, metric, days, version, params).await
}

fn metric_tool(metric: &str) -> &'static str {
    match metric {
        "lcr" => "lcr",
        "nsfr" => "nsfr",
        _ => "nii",
    }
}

pub async fn trend(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Query(q): Query<TrendQuery>) -> Result<Json<TrendResponse>, ApiError> {
    let metric = q.metric.as_deref().unwrap_or("lcr").to_lowercase();
    if !matches!(metric.as_str(), "lcr" | "nsfr" | "nii") {
        return Err(ApiError::BadRequest("metric must be lcr, nsfr or nii".into()));
    }
    let days = q.days.unwrap_or(30).clamp(1, 90);
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    let (version, params) = active_params(&mut tx).await?;
    let points = trend_points(&state, &mut tx, &metric, days, version.version, &params).await?;
    tx.commit().await?;
    Ok(Json(TrendResponse { metric, points }))
}

/// The ALM block of the overview, or `None` when there is no snapshot yet. The caller has
/// already checked entitlement.
pub async fn build_overview(state: &AppState, conn: &mut PgConnection) -> Result<Option<AlmOverview>, ApiError> {
    let book = match load_book(conn, None).await {
        Ok(b) => b,
        Err(ApiError::BadRequest(_)) => return Ok(None),
        Err(e) => return Err(e),
    };
    let (active, params) = active_params(conn).await?;
    let v = Some(active.version);
    let lcr = official(state, "lcr", &book, v, String::new(), || alm::lcr(&book.positions, &params, &book.spots, book.as_of, v));
    let nsfr = official(state, "nsfr", &book, v, String::new(), || alm::nsfr::compute(&book.positions, &params, &book.spots, book.as_of, v));
    let gap = official(state, "gap", &book, v, String::new(), || alm::gap::compute(&book.positions, &params, &book.spots, book.as_of, v));
    let funding = official(state, "funding", &book, v, String::new(), || alm::funding::compute(&book.positions, &book.depositors, &params, &book.spots, book.as_of, v));
    let stress = official(state, "stress", &book, v, "None".into(), || alm::stress::compute(&book.positions, &params, &book.spots, &book.curves, book.as_of, None, v));
    let nii = official(state, "nii", &book, v, "BASE:0".into(), || nii_for(&book, &params, RateScenario::Base, 0.0, v));

    let today = db::business_now(conn).await?.date_naive();
    let month = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
    let year_start = NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap_or(month);
    let rows: Vec<(NaiveDate, String, String, Decimal)> = sqlx::query_as(
        "SELECT period, kind, ccy::text, nii FROM alm_period WHERE period > $1 AND period <= $2 ORDER BY period",
    )
    .bind(add_months(month, -12))
    .bind(month)
    .fetch_all(&mut *conn)
    .await?;
    let fx = |ccy: &str| if ccy == params.reporting_ccy { 1.0 } else { book.spots.get(ccy).copied().unwrap_or(1.0) };
    let mut by_period: BTreeMap<NaiveDate, (f64, f64)> = BTreeMap::new();
    for (period, kind, ccy, nii) in &rows {
        let e = by_period.entry(*period).or_insert((0.0, 0.0));
        let v = f(*nii) * fx(ccy.trim());
        if kind == "ACTUAL" { e.0 += v } else { e.1 += v }
    }
    let (nii_mtd, budget_mtd) = by_period.get(&month).copied().unwrap_or((0.0, 0.0));
    let (nii_ytd, budget_ytd) = by_period.iter().filter(|(p, _)| **p >= year_start).fold((0.0, 0.0), |acc, (_, (a, b))| (acc.0 + a, acc.1 + b));
    let nii_months = by_period.iter().map(|(p, (a, b))| NiiPeriodRow { period: *p, actual: *a, budget: *b }).collect();

    let worst = stress.worst_eve.and_then(|s| stress.rate_scenarios.iter().find(|r| r.scenario == s));
    let worst_nii = stress.rate_scenarios.iter().filter_map(|r| r.nii_delta_pct).fold(0.0f64, f64::min);
    let gap_1y = gap.buckets.iter().find(|b| b.label == "1Y").map(|b| b.cumulative_pct_assets).unwrap_or(0.0);
    let floor = |metric: &str, value: Option<f64>, limit: f64, trigger: Option<f64>| LimitRow {
        metric: metric.into(),
        value,
        unit: "%".into(),
        limit,
        trigger,
        floor: true,
        breach: value.is_some_and(|x| x < limit),
        warn: value.is_some_and(|x| x >= limit && trigger.is_some_and(|t| x < t)),
    };
    let ceiling = |metric: &str, value: f64, limit: f64| LimitRow {
        metric: metric.into(),
        value: Some(value),
        unit: "%".into(),
        limit,
        trigger: None,
        floor: false,
        breach: value > limit,
        warn: value > limit * 0.9 && value <= limit,
    };
    let limits = vec![
        floor("LCR", lcr.total.ratio, params.lcr.internal_minimum.max(params.lcr.minimum), Some(params.lcr.early_warning)),
        floor("NSFR", nsfr.ratio, params.nsfr.internal_minimum.max(params.nsfr.minimum), None),
        ceiling("Cumulative gap 1Y (% assets)", gap_1y.abs(), params.gap.cumulative_gap_limit_pct_assets.get("1Y").copied().unwrap_or(30.0)),
        ceiling("EVE worst loss (% Tier 1)", worst.and_then(|w| w.eve_loss_pct_tier1).unwrap_or(0.0), params.gap.eve_limit_pct_tier1),
        ceiling("NII worst sensitivity (%)", worst_nii.abs(), params.gap.nii_sensitivity_limit_pct),
        ceiling("Largest depositor (% deposits)", funding.top1_pct, params.funding.top1_limit_pct),
        ceiling("Ten largest depositors (% deposits)", funding.top10_pct, params.funding.top10_limit_pct),
        ceiling("Wholesale funding (% funding)", funding.wholesale_pct, params.funding.wholesale_share_limit_pct),
    ];
    let lcr_trend = trend_points(state, conn, "lcr", 30, active.version, &params).await?;
    let ratios = alm::ratios::compute(&book.positions, &params, &book.spots, funding.wholesale_pct);

    // The same headline figures on the previous snapshot, for "since yesterday".
    let prev_date: Option<NaiveDate> = sqlx::query_scalar("SELECT max(as_of) FROM bs_snapshots WHERE complete AND as_of < $1").bind(book.as_of).fetch_one(&mut *conn).await?;
    let previous = match prev_date {
        Some(d) => {
            let pb = load_book(conn, Some(d)).await?;
            let plcr = official(state, "lcr", &pb, v, String::new(), || alm::lcr(&pb.positions, &params, &pb.spots, pb.as_of, v));
            let pnsfr = official(state, "nsfr", &pb, v, String::new(), || alm::nsfr::compute(&pb.positions, &params, &pb.spots, pb.as_of, v));
            let pfund = official(state, "funding", &pb, v, String::new(), || alm::funding::compute(&pb.positions, &pb.depositors, &params, &pb.spots, pb.as_of, v));
            let pstress = official(state, "stress", &pb, v, "None".into(), || alm::stress::compute(&pb.positions, &params, &pb.spots, &pb.curves, pb.as_of, None, v));
            let pnii = official(state, "nii", &pb, v, "BASE:0".into(), || nii_for(&pb, &params, RateScenario::Base, 0.0, v));
            let pworst = pstress.worst_eve.and_then(|s| pstress.rate_scenarios.iter().find(|r| r.scenario == s)).and_then(|w| w.eve_loss_pct_tier1);
            Some(OverviewPrevious { as_of: d, lcr: plcr.total.ratio, nsfr: pnsfr.ratio, worst_eve_loss_pct_tier1: pworst, top10_pct: pfund.top10_pct, nii_12m: pnii.nii })
        }
        None => None,
    };
    Ok(Some(AlmOverview {
        previous,
        ratios: Some(ratios),
        as_of: book.as_of,
        params_version: active.version,
        reporting_ccy: params.reporting_ccy.clone(),
        lcr: lcr.total.ratio,
        nsfr: nsfr.ratio,
        nii_12m: nii.nii,
        nim: nii.nim,
        nii_mtd,
        budget_mtd,
        nii_ytd,
        budget_ytd,
        gap_1y_pct: gap_1y,
        worst_eve_scenario: stress.worst_eve,
        worst_eve_loss_pct_tier1: worst.and_then(|w| w.eve_loss_pct_tier1),
        top10_pct: funding.top10_pct,
        wholesale_pct: funding.wholesale_pct,
        limits,
        lcr_trend,
        nii_months,
        gap_buckets: gap.buckets,
    }))
}

pub async fn overview(State(state): State<AppState>, Extension(auth): Extension<AuthUser>) -> Result<Json<AlmOverview>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    let o = build_overview(&state, &mut tx).await?;
    tx.commit().await?;
    o.map(Json).ok_or_else(|| ApiError::BadRequest("no complete banking-book snapshot is available yet".into()))
}
