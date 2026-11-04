//! ALM additions of phase 2F: the liquidity maturity ladder, the early-warning indicators with the
//! contingency plan's stage, and the monthly ALCO pack's commentary and sign-off.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use alm::ewi::{self, Series};
use alm::schedule::cash_flows;
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::{Extension, Json};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use domain::{
    AlcoNote, AlcoPack, AlcoSignoff, AlmParams, AlmRequest, BsSide, EwiPoint, EwiResult, LiquidityGapResult, PackNoteRequest, PackSignRequest, PeriodQuery,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use sqlx::PgConnection;

use crate::auth::AuthUser;
use crate::db;
use crate::error::ApiError;
use crate::routes::alm::{self as almr, active_params, entitled, load_book, load_positions, load_spots};
use crate::routes::audit::{self, AuditCtx};
use crate::routes::risk::is_head;
use crate::state::{AlmKey, AppState};

fn f(d: Decimal) -> f64 {
    d.to_f64().unwrap_or(0.0)
}

async fn params_for(conn: &mut PgConnection, req: &AlmRequest) -> Result<(Option<i32>, AlmParams), ApiError> {
    match &req.params {
        Some(p) => Ok((None, p.clone())),
        None => {
            let (v, p) = active_params(conn).await?;
            Ok((Some(v.version), p))
        }
    }
}

pub async fn liquidity_gap(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Json(req): Json<AlmRequest>) -> Result<Json<LiquidityGapResult>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    let book = load_book(&mut tx, req.as_of).await?;
    let (version, params) = params_for(&mut tx, &req).await?;
    tx.commit().await?;
    let compute = || alm::liquidity::compute(&book.positions, &params, &book.spots, book.as_of, version);
    Ok(Json(match version {
        Some(v) => {
            let key: AlmKey = ("liquidity_gap", book.as_of, book.generated_at, v, String::new());
            match state.alm.get(&key) {
                Some(hit) => hit,
                None => {
                    let r = compute();
                    state.alm.put(key, &r);
                    r
                }
            }
        }
        None => compute(),
    }))
}

/// The indicators over the daily snapshots of the last few weeks.
pub async fn build_ewi(state: &AppState, conn: &mut PgConnection, req: &AlmRequest) -> Result<EwiResult, ApiError> {
    let book = load_book(conn, req.as_of).await?;
    let (version, params) = params_for(conn, req).await?;
    if let Some(v) = version {
        let key: AlmKey = ("ewi", book.as_of, book.generated_at, v, String::new());
        if let Some(hit) = state.alm.get::<EwiResult>(&key) {
            return Ok(hit);
        }
    }
    let snaps: Vec<NaiveDate> = sqlx::query_scalar("SELECT as_of FROM bs_snapshots WHERE complete AND as_of <= $1 AND as_of > $2 ORDER BY as_of")
        .bind(book.as_of)
        .bind(book.as_of - Duration::days(36))
        .fetch_all(&mut *conn)
        .await?;

    // Deposits, retail deposits, wholesale maturing within 30 days, total funding, per snapshot.
    let mut deposits = Vec::new();
    let mut retail = Vec::new();
    let mut wholesale_30d = Vec::new();
    let mut top10 = Vec::new();
    for as_of in &snaps {
        let positions = load_positions(conn, *as_of).await?;
        let spots = load_spots(conn, *as_of).await?;
        let fx = |ccy: &str| if ccy == params.reporting_ccy { 1.0 } else { spots.get(ccy).copied().unwrap_or(1.0) };
        let (mut dep, mut ret, mut funding, mut due) = (0.0, 0.0, 0.0, 0.0);
        for p in positions.iter().filter(|p| p.side == BsSide::Liability && p.book != "EQUITY") {
            let rate = fx(&p.ccy);
            funding += p.balance * rate;
            if p.book == "DEPOSITS" {
                dep += p.balance * rate;
                if p.segment == "RETAIL" {
                    ret += p.balance * rate;
                }
            }
            let wholesale = p.book == "INTERBANK" || p.book == "TERM_FUNDING" || (p.book == "DEPOSITS" && p.segment == "FI");
            if wholesale {
                let limit = *as_of + Duration::days(30);
                due += cash_flows(p, *as_of).iter().filter(|c| c.date <= limit).map(|c| c.principal).sum::<f64>() * rate;
            }
        }
        deposits.push((*as_of, dep));
        retail.push((*as_of, ret));
        wholesale_30d.push(EwiPoint { as_of: *as_of, value: (funding > 0.0).then(|| due / funding * 100.0) });
        let groups: Vec<(String, String, Decimal)> = sqlx::query_as("SELECT group_name, ccy::text, balance FROM bs_depositors WHERE as_of = $1").bind(as_of).fetch_all(&mut *conn).await?;
        let mut by: BTreeMap<String, f64> = BTreeMap::new();
        for (g, ccy, bal) in groups {
            *by.entry(g).or_default() += f(bal) * fx(ccy.trim());
        }
        let mut v: Vec<f64> = by.into_values().collect();
        v.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        top10.push(EwiPoint { as_of: *as_of, value: (dep > 0.0).then(|| v.iter().take(10).sum::<f64>() / dep * 100.0) });
    }

    // LCR per snapshot through the cache, and the 3-month BHIBOR and SOFR.
    let lcr_points = match version {
        Some(v) => almr::trend_points_pub(state, conn, "lcr", 36, v, &params).await?,
        None => {
            let mut out = Vec::new();
            for as_of in &snaps {
                let positions = load_positions(conn, *as_of).await?;
                let spots = load_spots(conn, *as_of).await?;
                out.push(domain::TrendPoint { as_of: *as_of, value: alm::lcr(&positions, &params, &spots, *as_of, None).total.ratio });
            }
            out
        }
    };
    let lcr: Vec<(NaiveDate, Option<f64>)> = snaps.iter().map(|d| (*d, lcr_points.iter().find(|p| p.as_of == *d).and_then(|p| p.value))).collect();
    let rates: Vec<(NaiveDate, String, Decimal)> = sqlx::query_as(
        "SELECT as_of, ccy::text, rate FROM market_rates WHERE tenor = '3M' AND ccy IN ('BHD', 'USD') AND as_of <= $1 AND as_of > $2 ORDER BY as_of",
    )
    .bind(book.as_of)
    .bind(book.as_of - Duration::days(45))
    .fetch_all(&mut *conn)
    .await?;
    let rate_on = |ccy: &str, d: NaiveDate| rates.iter().rev().find(|r| r.1.trim() == ccy && r.0 <= d).map(|r| f(r.2));
    let bhibor: Vec<(NaiveDate, Option<f64>)> = snaps.iter().map(|d| (*d, rate_on("BHD", *d))).collect();
    let spread: Vec<(NaiveDate, Option<f64>)> = snaps.iter().map(|d| (*d, rate_on("BHD", *d).zip(rate_on("USD", *d)).map(|(b, u)| b - u))).collect();

    // Survival under the liquidity stress, today only.
    let stress = alm::stress::compute(&book.positions, &params, &book.spots, &book.curves, book.as_of, None, version);
    let survival = vec![EwiPoint { as_of: book.as_of, value: Some(stress.liquidity.survival_days.map(f64::from).unwrap_or(91.0)) }];

    let series = vec![
        Series { key: "deposits_5d", label: "Deposits, 5-day change", what: "All customer and bank deposits against five snapshots earlier.", unit: "%", rising_is_worse: false, points: ewi::pct_change(&deposits, 5) },
        Series { key: "retail_5d", label: "Retail deposits, 5-day change", what: "The stickiest funding; a fall here is the earliest sign of a run.", unit: "%", rising_is_worse: false, points: ewi::pct_change(&retail, 5) },
        Series { key: "wholesale_30d", label: "Wholesale funding due in 30 days", what: "Interbank, term funding and bank deposits maturing within a month, as a share of funding.", unit: "%", rising_is_worse: true, points: wholesale_30d },
        Series { key: "top10", label: "Ten largest depositors", what: "The share of deposits held by the ten largest depositor groups.", unit: "%", rising_is_worse: true, points: top10 },
        Series { key: "lcr_5d", label: "LCR, 5-day change", what: "Percentage points the ratio moved against five snapshots earlier.", unit: "pp", rising_is_worse: false, points: ewi::change(&lcr, 5, 1.0) },
        Series { key: "bhibor_5d", label: "3M BHIBOR, 5-day change", what: "A sharp rise in the dinar interbank rate signals tight local liquidity.", unit: "bp", rising_is_worse: true, points: ewi::change(&bhibor, 5, 100.0) },
        Series { key: "peg_spread_5d", label: "BHIBOR over SOFR, 5-day change", what: "The dinar's premium over the dollar widening is the market pricing peg or liquidity pressure.", unit: "bp", rising_is_worse: true, points: ewi::change(&spread, 5, 100.0) },
        Series { key: "survival", label: "Survival under stress", what: "Days the liquid assets cover the stressed outflows; 91 means beyond the 90-day horizon.", unit: "days", rising_is_worse: false, points: survival },
    ];
    let result = ewi::compute(series, &params, book.as_of, version);
    if let Some(v) = version {
        state.alm.put(("ewi", book.as_of, book.generated_at, v, String::new()), &result);
    }
    Ok(result)
}

pub async fn ewi(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Json(req): Json<AlmRequest>) -> Result<Json<EwiResult>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    let r = build_ewi(&state, &mut tx, &req).await?;
    tx.commit().await?;
    Ok(Json(r))
}

// ---------------------------------------------------------------------------------------------
// The ALCO pack
// ---------------------------------------------------------------------------------------------

fn month_of(d: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap_or(d)
}

pub async fn pack(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Query(q): Query<PeriodQuery>) -> Result<Json<AlcoPack>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    let period = match q.period {
        Some(p) => month_of(p),
        None => month_of(db::business_now(&mut tx).await?.date_naive()),
    };
    let rows: Vec<(String, String, Option<String>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT n.section, n.body, u.display_name, n.updated_at FROM alco_notes n LEFT JOIN users u ON u.id = n.updated_by WHERE n.period = $1",
    )
    .bind(period)
    .fetch_all(&mut *tx)
    .await?;
    let signoff: Option<(Option<String>, DateTime<Utc>, Option<String>)> =
        sqlx::query_as("SELECT u.display_name, s.signed_at, s.note FROM alco_signoffs s LEFT JOIN users u ON u.id = s.signed_by WHERE s.period = $1").bind(period).fetch_optional(&mut *tx).await?;
    let can_sign = is_head(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(AlcoPack {
        period,
        notes: rows.into_iter().map(|(s, body, by, at)| (s, AlcoNote { body, updated_by: by, updated_at: at })).collect(),
        signoff: signoff.map(|(signed_by, signed_at, note)| AlcoSignoff { signed_by, signed_at, note }),
        can_sign,
    }))
}

pub async fn pack_note(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Json(req): Json<PackNoteRequest>) -> Result<StatusCode, ApiError> {
    let section = req.section.trim();
    if section.is_empty() || section.len() > 60 || req.body.chars().count() > 4000 {
        return Err(ApiError::BadRequest("a section name and commentary of up to 4,000 characters".into()));
    }
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    sqlx::query("SELECT alco_note_save($1, $2, $3)").bind(month_of(req.period)).bind(section).bind(req.body.trim()).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn pack_sign(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<PackSignRequest>,
) -> Result<StatusCode, ApiError> {
    let note = req.note.as_deref().map(str::trim).unwrap_or_default();
    if note.chars().count() > 500 {
        return Err(ApiError::BadRequest("the sign-off note is limited to 500 characters".into()));
    }
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    if !is_head(&mut tx).await? {
        return Err(ApiError::Forbidden("only the treasury head may sign off the ALCO pack".into()));
    }
    let period = month_of(req.period);
    sqlx::query("SELECT alco_sign($1, $2)").bind(period).bind(note).execute(&mut *tx).await?;
    tx.commit().await?;
    let ctx = AuditCtx::new(Some(peer), &headers);
    audit::write(&state.pool, Some(auth.user.id), &auth.user.username, "alco_sign", &ctx, Some(serde_json::json!({ "period": period }))).await;
    Ok(StatusCode::NO_CONTENT)
}
