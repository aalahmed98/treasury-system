//! Market risk of the FX dealing book: positions, P&L, the forward ladder, counterparty exposure,
//! value at risk and the bank's FX limits, all computed on request from the deals the caller may
//! see (row-level security scopes them to the caller's desks) and the market data already held.
//! The limits are a versioned document set by the treasury head; breaches can be signed off.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use alm::curve::Curve;
use alm::fx::{self, FxDeal, FxMarket, RiskInputs, SpotHistory};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::{Extension, Json};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use domain::{
    Breach, DaysQuery, FxDealValue, FxHistory, FxRiskResult, LimitStatus, RiskLimits, RiskLimitsResponse, RiskLimitsVersion, SaveLimitsRequest, SignOffRequest,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use sqlx::PgConnection;
use sqlx::types::Json as Jsonb;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::db;
use crate::error::ApiError;
use crate::routes::alm::group_curves;
use crate::routes::audit::{self, AuditCtx};
use crate::state::AppState;

fn f(d: Decimal) -> f64 {
    d.to_f64().unwrap_or(0.0)
}

// ---------------------------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------------------------

pub async fn is_head(conn: &mut PgConnection) -> Result<bool, ApiError> {
    Ok(sqlx::query_scalar("SELECT is_treasury_head()").fetch_one(&mut *conn).await?)
}

pub async fn active_limits(conn: &mut PgConnection) -> Result<(RiskLimitsVersion, RiskLimits), ApiError> {
    let row: Option<(i32, DateTime<Utc>, Option<String>, Option<String>, bool, Jsonb<RiskLimits>)> = sqlx::query_as(
        "SELECT l.version, l.created_at, u.display_name, l.note, l.is_active, l.limits FROM risk_limits l LEFT JOIN users u ON u.id = l.created_by WHERE l.is_active",
    )
    .fetch_optional(&mut *conn)
    .await?;
    let (version, created_at, created_by, note, is_active, Jsonb(limits)) = row.ok_or_else(|| ApiError::Internal(anyhow::anyhow!("no active FX limits")))?;
    Ok((RiskLimitsVersion { version, created_at, created_by, note, is_active }, limits))
}

pub async fn limits(State(state): State<AppState>, Extension(auth): Extension<AuthUser>) -> Result<Json<RiskLimitsResponse>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let (active, limits) = active_limits(&mut tx).await?;
    let rows: Vec<(i32, DateTime<Utc>, Option<String>, Option<String>, bool)> = sqlx::query_as(
        "SELECT l.version, l.created_at, u.display_name, l.note, l.is_active FROM risk_limits l LEFT JOIN users u ON u.id = l.created_by ORDER BY l.version DESC LIMIT 20",
    )
    .fetch_all(&mut *tx)
    .await?;
    let can_edit = is_head(&mut tx).await?;
    tx.commit().await?;
    let history = rows.into_iter().map(|(version, created_at, created_by, note, is_active)| RiskLimitsVersion { version, created_at, created_by, note, is_active }).collect();
    Ok(Json(RiskLimitsResponse { active, limits, history, can_edit }))
}

fn validate(l: &RiskLimits) -> Result<(), ApiError> {
    let bad = |m: &str| Err(ApiError::BadRequest(m.into()));
    let amount = |v: f64| v.is_finite() && (0.0..=1e12).contains(&v);
    for v in [l.nop_overnight_total, l.nop_intraday_total, l.nop_default_floating, l.nop_default_pegged, l.dealer_nop, l.stop_loss_daily, l.stop_loss_monthly, l.var_limit] {
        if !amount(v) {
            return bad("limits must be amounts between zero and a trillion");
        }
    }
    if l.nop_intraday_total < l.nop_overnight_total {
        return bad("the intraday position limit cannot be below the overnight one");
    }
    if l.stop_loss_monthly < l.stop_loss_daily {
        return bad("the monthly stop-loss cannot be below the daily one");
    }
    for v in l.nop_overnight_ccy.values() {
        if !amount(*v) {
            return bad("currency limits must be amounts between zero and a trillion");
        }
    }
    for (name, line) in &l.counterparty_lines {
        if name.trim().is_empty() || !amount(line.pre_settlement) || !amount(line.settlement) {
            return bad("every counterparty line needs a name and two amounts");
        }
    }
    if !amount(l.default_line.pre_settlement) || !amount(l.default_line.settlement) {
        return bad("the default counterparty line needs two amounts");
    }
    for v in [l.off_market_bp_spot, l.off_market_bp_forward] {
        if !v.is_finite() || !(1.0..=1000.0).contains(&v) {
            return bad("off-market tolerances must be 1 to 1,000 basis points");
        }
    }
    if !(1.0..=100.0).contains(&l.warn_pct) {
        return bad("the watch level must be 1 to 100 percent of a limit");
    }
    if l.base_ccy.len() != 3 {
        return bad("the base currency must be a three-letter code");
    }
    Ok(())
}

pub async fn save_limits(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Extension(auth): Extension<AuthUser>,
    Json(mut req): Json<SaveLimitsRequest>,
) -> Result<(StatusCode, Json<RiskLimitsVersion>), ApiError> {
    // Lines are held under the counterparty's normalised name, so a lookup always finds them.
    req.limits.counterparty_lines = req.limits.counterparty_lines.into_iter().map(|(k, v)| (domain::normalise_counterparty(&k), v)).collect();
    validate(&req.limits)?;
    let note = req.note.trim();
    if note.is_empty() || note.chars().count() > 500 || note.chars().any(char::is_control) {
        return Err(ApiError::BadRequest("a note of up to 500 printable characters is required".into()));
    }
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    if !is_head(&mut tx).await? {
        return Err(ApiError::Forbidden("only the treasury head may change the FX limits".into()));
    }
    let version: i32 = sqlx::query_scalar("SELECT risk_limits_save($1, $2)").bind(Jsonb(&req.limits)).bind(note).fetch_one(&mut *tx).await?;
    let (active, _) = active_limits(&mut tx).await?;
    tx.commit().await?;
    let ctx = AuditCtx::new(Some(peer), &headers);
    audit::write(&state.pool, Some(auth.user.id), &auth.user.username, "risk_limits_save", &ctx, Some(serde_json::json!({ "version": version, "note": note }))).await;
    Ok((StatusCode::CREATED, Json(active)))
}

// ---------------------------------------------------------------------------------------------
// Deals and market data
// ---------------------------------------------------------------------------------------------

/// The FX deals the caller may see, live (not cancelled, not merged into another), in booking order.
pub async fn load_fx_deals(conn: &mut PgConnection) -> Result<Vec<FxDeal>, ApiError> {
    #[allow(clippy::type_complexity)]
    let rows: Vec<(Uuid, String, String, String, bool, String, String, String, String, Option<String>, Decimal, Decimal, NaiveDate, NaiveDate)> = sqlx::query_as(
        "SELECT d.id, d.deal_ref, k.code, coalesce(u.display_name, 'From ' || initcap(d.origin) || ', no dealer'), u.id IS NOT NULL, d.counterparty, d.product, d.side, d.ccy1::text, d.ccy2::text, d.notional, d.rate, \
                d.trade_date, d.value_date \
         FROM deals d JOIN desks k ON k.id = d.desk_id LEFT JOIN users u ON u.id = d.trader_id \
         WHERE d.product IN ('FX_SPOT', 'FX_FWD') AND d.status <> 'CANCELLED' AND d.merged_into IS NULL AND d.ccy2 IS NOT NULL \
         ORDER BY d.created_at, d.deal_ref",
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .enumerate()
        .map(|(i, (id, deal_ref, desk_code, trader, has_dealer, counterparty, product, side, ccy1, ccy2, notional, rate, trade_date, value_date))| FxDeal {
            id,
            deal_ref,
            desk_code,
            trader,
            counterparty,
            forward: product == "FX_FWD",
            sign: if side == "BUY" { 1.0 } else { -1.0 },
            ccy1: ccy1.trim().to_string(),
            ccy2: ccy2.unwrap_or_default().trim().to_string(),
            notional: f(notional),
            rate: f(rate),
            trade_date,
            value_date,
            seq: i as i64,
            has_dealer,
        })
        .collect())
}

/// Daily spots and curves over a range, each day filled forward from the last close before it.
pub struct MarketData {
    pub spots: SpotHistory,
    curves: BTreeMap<NaiveDate, BTreeMap<String, Curve>>,
}

impl MarketData {
    pub fn market(&self, on: NaiveDate) -> FxMarket {
        FxMarket {
            date: on,
            spots: self.spots.range(..=on).next_back().map(|(_, s)| s.clone()).unwrap_or_default(),
            curves: self.curves.range(..=on).next_back().map(|(_, c)| c.clone()).unwrap_or_default(),
        }
    }
}

pub async fn load_market(conn: &mut PgConnection, from: NaiveDate, to: NaiveDate) -> Result<MarketData, ApiError> {
    let look_back = from - Duration::days(10);
    let rows: Vec<(NaiveDate, String, Decimal)> =
        sqlx::query_as("SELECT as_of, pair, mid FROM fx_spots WHERE as_of BETWEEN $1 AND $2 ORDER BY as_of").bind(look_back).bind(to).fetch_all(&mut *conn).await?;
    let mut by_day: BTreeMap<NaiveDate, BTreeMap<String, f64>> = BTreeMap::new();
    for (as_of, pair, mid) in rows {
        if let Some((ccy, _)) = pair.split_once('/') {
            by_day.entry(as_of).or_default().insert(ccy.to_string(), f(mid));
        }
    }
    // One entry per calendar day, so returns are daily and a missing close repeats the last one.
    let mut spots: SpotHistory = BTreeMap::new();
    let mut last: BTreeMap<String, f64> = BTreeMap::new();
    let mut d = look_back;
    while d <= to {
        if let Some(s) = by_day.get(&d) {
            last.extend(s.iter().map(|(k, v)| (k.clone(), *v)));
        }
        if !last.is_empty() && d >= from {
            spots.insert(d, last.clone());
        }
        d += Duration::days(1);
    }
    let rows: Vec<(NaiveDate, String, String, String, i32, Decimal)> = sqlx::query_as(
        "SELECT as_of, ccy::text, index_code, tenor, tenor_days, rate FROM market_rates WHERE as_of BETWEEN $1 AND $2 ORDER BY as_of, ccy, tenor_days",
    )
    .bind(look_back)
    .bind(to)
    .fetch_all(&mut *conn)
    .await?;
    let mut grouped: BTreeMap<NaiveDate, Vec<(String, String, String, i32, Decimal, NaiveDate)>> = BTreeMap::new();
    for (as_of, ccy, index, tenor, days, rate) in rows {
        grouped.entry(as_of).or_default().push((ccy, index, tenor, days, rate, as_of));
    }
    let curves = grouped.into_iter().map(|(d, rows)| (d, group_curves(rows).iter().map(|c| (c.ccy.clone(), Curve::from_rate_curve(c))).collect())).collect();
    Ok(MarketData { spots, curves })
}

/// How far back the market is loaded: a full VaR window plus the stressed windows before it.
const HISTORY_DAYS: i64 = 420;

pub struct Book {
    pub today: NaiveDate,
    pub deals: Vec<FxDeal>,
    pub limits_version: i32,
    pub limits: RiskLimits,
    pub market: MarketData,
    pub cancelled_30d: u32,
}

pub async fn load_book(conn: &mut PgConnection, as_of: Option<NaiveDate>, history_days: i64) -> Result<Book, ApiError> {
    let today = match as_of {
        Some(d) => d,
        None => db::business_now(conn).await?.date_naive(),
    };
    let deals = load_fx_deals(conn).await?;
    let (version, limits) = active_limits(conn).await?;
    let first_trade = deals.iter().map(|d| d.trade_date).min().unwrap_or(today);
    let (_, _, year_open) = fx::period_opens(today);
    let from = (today - Duration::days(history_days)).min(first_trade).min(year_open) - Duration::days(2);
    let market = load_market(conn, from, today).await?;
    let cancelled: i64 = sqlx::query_scalar("SELECT count(*) FROM deals WHERE product IN ('FX_SPOT', 'FX_FWD') AND status = 'CANCELLED' AND updated_at > now() - interval '30 days'")
        .fetch_one(&mut *conn)
        .await?;
    Ok(Book { today, deals, limits_version: version.version, limits, market, cancelled_30d: cancelled as u32 })
}

/// The risk view of the book on a day. `stressed` adds the stressed VaR search, which the
/// attention list and the breach history skip.
pub fn risk_on(book: &Book, on: NaiveDate, trade_markets: &BTreeMap<NaiveDate, FxMarket>, stressed: bool) -> FxRiskResult {
    let (prev, month, year) = fx::period_opens(on);
    let (m, p, mo, yo) = (book.market.market(on), book.market.market(prev), book.market.market(month), book.market.market(year));
    let history: SpotHistory = if stressed { book.market.spots.clone() } else { book.market.spots.range(on - Duration::days(fx::VAR_WINDOW as i64 + 5)..=on).map(|(d, s)| (*d, s.clone())).collect() };
    fx::risk(&RiskInputs {
        deals: &book.deals,
        today: &m,
        prev: &p,
        month_open: &mo,
        year_open: &yo,
        trade_markets,
        history: &history,
        limits: &book.limits,
        limits_version: Some(book.limits_version),
        cancelled_30d: book.cancelled_30d,
    })
}

pub fn trade_markets(book: &Book) -> BTreeMap<NaiveDate, FxMarket> {
    let mut out = BTreeMap::new();
    for d in &book.deals {
        out.entry(d.trade_date).or_insert_with(|| book.market.market(d.trade_date));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------------------------

pub async fn fx_risk(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Query(q): Query<DaysQuery>) -> Result<Json<FxRiskResult>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let book = load_book(&mut tx, q.as_of, HISTORY_DAYS).await?;
    tx.commit().await?;
    let tm = trade_markets(&book);
    Ok(Json(risk_on(&book, book.today, &tm, true)))
}

/// Every FX deal valued at the day's market, with its trade-date check.
pub async fn fx_deals(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Query(q): Query<DaysQuery>) -> Result<Json<Vec<FxDealValue>>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let book = load_book(&mut tx, q.as_of, 10).await?;
    tx.commit().await?;
    let base = book.limits.base_ccy.as_str();
    let m = book.market.market(book.today);
    let tm = trade_markets(&book);
    Ok(Json(
        book.deals
            .iter()
            .filter(|d| d.trade_date <= book.today)
            .map(|d| {
                let t = tm.get(&d.trade_date);
                let check = t.and_then(|t| fx::off_market_bp(d, t, base));
                let tolerance = if d.forward { book.limits.off_market_bp_forward } else { book.limits.off_market_bp_spot };
                let flagged = check.is_some_and(|c| c.1.abs() > tolerance);
                fx::deal_value(d, &m, t, base, check, flagged)
            })
            .collect(),
    ))
}

pub async fn fx_history(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Query(q): Query<DaysQuery>) -> Result<Json<FxHistory>, ApiError> {
    let days = q.days.unwrap_or(30).clamp(5, 120);
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let book = load_book(&mut tx, q.as_of, fx::VAR_WINDOW as i64 + days + 10).await?;
    tx.commit().await?;
    let range: Vec<NaiveDate> = (0..days).rev().map(|i| book.today - Duration::days(i)).collect();
    let mut markets = BTreeMap::new();
    for d in range.iter().chain(std::iter::once(&(book.today - Duration::days(days)))) {
        markets.insert(*d, book.market.market(*d));
    }
    Ok(Json(FxHistory { points: fx::history(&book.deals, &markets, &book.market.spots, &book.limits, &range) }))
}

/// Every day in the range a limit stood breached at the close, with any sign-off.
pub async fn breaches(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Query(q): Query<DaysQuery>) -> Result<Json<Vec<Breach>>, ApiError> {
    let days = q.days.unwrap_or(30).clamp(1, 90);
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let book = load_book(&mut tx, q.as_of, fx::VAR_WINDOW as i64 + days + 10).await?;
    let signed: Vec<(NaiveDate, String, String, Option<String>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT s.day, s.key, s.note, u.display_name, s.signed_at FROM limit_signoffs s LEFT JOIN users u ON u.id = s.signed_by WHERE s.day > $1",
    )
    .bind(book.today - Duration::days(days))
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let signed: BTreeMap<(NaiveDate, String), (String, Option<String>, DateTime<Utc>)> = signed.into_iter().map(|(d, k, n, by, at)| ((d, k), (n, by, at))).collect();
    let tm = trade_markets(&book);
    let mut out = Vec::new();
    for i in (0..days).rev() {
        let day = book.today - Duration::days(i);
        if !book.deals.iter().any(|d| d.trade_date <= day) {
            continue;
        }
        let r = risk_on(&book, day, &tm, false);
        for l in r.limits.into_iter().filter(|l| l.status == LimitStatus::Breach && l.key != "nop.intraday") {
            let s = signed.get(&(day, l.key.clone()));
            out.push(Breach {
                date: day,
                key: l.key,
                label: l.label,
                value: l.value,
                limit: l.limit,
                signed_by: s.and_then(|s| s.1.clone()),
                signed_at: s.map(|s| s.2),
                note: s.map(|s| s.0.clone()),
            });
        }
    }
    out.reverse();
    Ok(Json(out))
}

pub async fn sign_breach(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<SignOffRequest>,
) -> Result<StatusCode, ApiError> {
    let note = req.note.trim();
    if note.is_empty() || note.chars().count() > 500 || note.chars().any(char::is_control) || req.key.trim().is_empty() || req.key.len() > 200 {
        return Err(ApiError::BadRequest("a limit key and a note of up to 500 printable characters are required".into()));
    }
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    if !is_head(&mut tx).await? {
        return Err(ApiError::Forbidden("only the treasury head may sign off a limit breach".into()));
    }
    sqlx::query("SELECT limit_signoff($1, $2, $3)").bind(req.date).bind(req.key.trim()).bind(note).execute(&mut *tx).await?;
    tx.commit().await?;
    let ctx = AuditCtx::new(Some(peer), &headers);
    audit::write(&state.pool, Some(auth.user.id), &auth.user.username, "limit_signoff", &ctx, Some(serde_json::json!({ "date": req.date, "key": req.key, "note": note }))).await;
    Ok(StatusCode::NO_CONTENT)
}

/// For the attention list: today's breached and watched FX limits and the off-market deals of the
/// last two days. Light: no stressed VaR, a short market history.
pub async fn attention_view(conn: &mut PgConnection) -> Result<Option<FxRiskResult>, ApiError> {
    let book = load_book(conn, None, fx::VAR_WINDOW as i64 + 10).await?;
    if book.deals.is_empty() {
        return Ok(None);
    }
    let tm = trade_markets(&book);
    Ok(Some(risk_on(&book, book.today, &tm, false)))
}
