//! Reconciliation read APIs and the two user actions on breaks. Everything is desk-scoped by the
//! policies on source_deals, breaks and source_events; acknowledging and resolving go through
//! definer-rights functions that check `can_resolve`.

use std::net::SocketAddr;
use std::str::FromStr;

use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::{Extension, Json};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    BreakActionView, BreakDetail, BreakView, BreaksQuery, Deal, EventSummary, FeedStatus, FeedsResponse, ProvenanceResponse, ReconStatus,
    ResolveRequest, SourceDealView,
};
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::db::{self, DealRow};
use crate::deal_select;
use crate::error::ApiError;
use crate::routes::audit::{self, AuditCtx};
use crate::state::AppState;

const ONE_DEAL: &str = concat!(deal_select!(), " WHERE d.id = $1");

#[derive(sqlx::FromRow)]
struct SourceDealRow {
    source: String,
    source_ref: String,
    deal_id: Option<Uuid>,
    link_kind: Option<String>,
    linked_at: Option<DateTime<Utc>>,
    lifecycle: String,
    raw_state: Option<String>,
    applied_ts: DateTime<Utc>,
    first_seen_at: DateTime<Utc>,
    desk_id: i16,
    product: String,
    side: String,
    ccy1: String,
    ccy2: Option<String>,
    notional: Decimal,
    rate: Decimal,
    trade_date: NaiveDate,
    value_date: NaiveDate,
    maturity_date: Option<NaiveDate>,
    counterparty_raw: String,
    counterparty: String,
    trader_ref: Option<String>,
    cross_refs: Value,
    updated_at: DateTime<Utc>,
}

impl TryFrom<SourceDealRow> for SourceDealView {
    type Error = anyhow::Error;
    fn try_from(r: SourceDealRow) -> Result<Self, Self::Error> {
        Ok(SourceDealView {
            source: r.source.parse().map_err(anyhow::Error::msg)?,
            source_ref: r.source_ref,
            deal_id: r.deal_id,
            link_kind: r.link_kind.as_deref().map(FromStr::from_str).transpose().map_err(anyhow::Error::msg)?,
            linked_at: r.linked_at,
            lifecycle: r.lifecycle.parse().map_err(anyhow::Error::msg)?,
            raw_state: r.raw_state,
            applied_ts: r.applied_ts,
            first_seen_at: r.first_seen_at,
            desk_id: r.desk_id,
            product: r.product.parse().map_err(anyhow::Error::msg)?,
            side: r.side.parse().map_err(anyhow::Error::msg)?,
            ccy1: r.ccy1.trim().to_string(),
            ccy2: r.ccy2.map(|c| c.trim().to_string()),
            notional: r.notional,
            rate: r.rate,
            trade_date: r.trade_date,
            value_date: r.value_date,
            maturity_date: r.maturity_date,
            counterparty_raw: r.counterparty_raw,
            counterparty: r.counterparty,
            trader_ref: r.trader_ref,
            cross_refs: r.cross_refs,
            updated_at: r.updated_at,
        })
    }
}

const SOURCE_DEAL_COLS: &str = "s.source, s.source_ref, s.deal_id, s.link_kind, s.linked_at, s.lifecycle, s.raw_state, s.applied_ts, \
                                s.first_seen_at, s.desk_id, s.product, s.side, s.ccy1::text AS ccy1, s.ccy2::text AS ccy2, s.notional, s.rate, \
                                s.trade_date, s.value_date, s.maturity_date, s.counterparty_raw, s.counterparty, s.trader_ref, s.cross_refs, s.updated_at";

#[derive(sqlx::FromRow)]
struct BreakRow {
    id: i64,
    kind: String,
    deal_id: Option<Uuid>,
    deal_ref: Option<String>,
    source: Option<String>,
    source_ref: Option<String>,
    field: Option<String>,
    desk_id: i16,
    desk_code: String,
    details: Value,
    status: String,
    detected_at: DateTime<Utc>,
    acknowledged_at: Option<DateTime<Utc>>,
    resolved_at: Option<DateTime<Utc>>,
    resolution: Option<String>,
    note: Option<String>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<BreakRow> for BreakView {
    type Error = anyhow::Error;
    fn try_from(r: BreakRow) -> Result<Self, Self::Error> {
        Ok(BreakView {
            id: r.id,
            kind: r.kind.parse().map_err(anyhow::Error::msg)?,
            deal_id: r.deal_id,
            deal_ref: r.deal_ref,
            source: r.source.as_deref().map(FromStr::from_str).transpose().map_err(anyhow::Error::msg)?,
            source_ref: r.source_ref,
            field: r.field,
            desk_id: r.desk_id,
            desk_code: r.desk_code,
            details: r.details,
            status: r.status.parse().map_err(anyhow::Error::msg)?,
            detected_at: r.detected_at,
            acknowledged_at: r.acknowledged_at,
            resolved_at: r.resolved_at,
            resolution: r.resolution.as_deref().map(FromStr::from_str).transpose().map_err(anyhow::Error::msg)?,
            note: r.note,
            updated_at: r.updated_at,
        })
    }
}

const BREAK_SELECT: &str = "SELECT b.id, b.kind, b.deal_id, d.deal_ref, b.source, b.source_ref, b.field, b.desk_id, k.code AS desk_code, \
                                   b.details, b.status, b.detected_at, b.acknowledged_at, b.resolved_at, b.resolution, b.note, b.updated_at \
                            FROM breaks b JOIN desks k ON k.id = b.desk_id LEFT JOIN deals d ON d.id = b.deal_id";

async fn load_break(tx: &mut PgConnection, id: i64) -> Result<BreakView, ApiError> {
    let row: Option<BreakRow> = sqlx::query_as(concat!(
        "SELECT b.id, b.kind, b.deal_id, d.deal_ref, b.source, b.source_ref, b.field, b.desk_id, k.code AS desk_code, \
                b.details, b.status, b.detected_at, b.acknowledged_at, b.resolved_at, b.resolution, b.note, b.updated_at \
         FROM breaks b JOIN desks k ON k.id = b.desk_id LEFT JOIN deals d ON d.id = b.deal_id",
        " WHERE b.id = $1"
    ))
    .bind(id)
    .fetch_optional(tx)
    .await?;
    BreakView::try_from(row.ok_or(ApiError::NotFound)?).map_err(ApiError::from)
}

// ---------------------------------------------------------------------------------------------

pub async fn provenance(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<ProvenanceResponse>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let deal: Option<DealRow> = sqlx::query_as(ONE_DEAL).bind(id).fetch_optional(&mut *tx).await?;
    let deal = Deal::try_from(deal.ok_or(ApiError::NotFound)?)?;

    let sources: Vec<SourceDealRow> = sqlx::query_as(concat!(
        "SELECT s.source, s.source_ref, s.deal_id, s.link_kind, s.linked_at, s.lifecycle, s.raw_state, s.applied_ts, \
                s.first_seen_at, s.desk_id, s.product, s.side, s.ccy1::text AS ccy1, s.ccy2::text AS ccy2, s.notional, s.rate, \
                s.trade_date, s.value_date, s.maturity_date, s.counterparty_raw, s.counterparty, s.trader_ref, s.cross_refs, s.updated_at",
        " FROM source_deals s WHERE s.deal_id = $1 ORDER BY s.source"
    ))
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    let _ = SOURCE_DEAL_COLS;

    let breaks: Vec<BreakRow> = sqlx::query_as(concat!(
        "SELECT b.id, b.kind, b.deal_id, d.deal_ref, b.source, b.source_ref, b.field, b.desk_id, k.code AS desk_code, \
                b.details, b.status, b.detected_at, b.acknowledged_at, b.resolved_at, b.resolution, b.note, b.updated_at \
         FROM breaks b JOIN desks k ON k.id = b.desk_id LEFT JOIN deals d ON d.id = b.deal_id",
        " WHERE b.deal_id = $1 ORDER BY b.status = 'RESOLVED', b.id DESC"
    ))
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    let _ = BREAK_SELECT;

    #[derive(sqlx::FromRow)]
    struct Ev {
        id: i64,
        source: String,
        source_ref: String,
        event_kind: String,
        source_ts: DateTime<Utc>,
        observed_at: DateTime<Utc>,
        processed_at: Option<DateTime<Utc>>,
        outcome: Option<String>,
    }
    let events: Vec<Ev> = sqlx::query_as(
        "SELECT e.id, e.source, e.source_ref, e.event_kind, e.source_ts, e.observed_at, e.processed_at, e.outcome \
         FROM source_events e JOIN source_deals s ON s.source = e.source AND s.source_ref = e.source_ref \
         WHERE s.deal_id = $1 ORDER BY e.id DESC LIMIT 100",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(Json(ProvenanceResponse {
        deal,
        sources: sources.into_iter().map(SourceDealView::try_from).collect::<Result<_, _>>()?,
        breaks: breaks.into_iter().map(BreakView::try_from).collect::<Result<_, _>>()?,
        events: events
            .into_iter()
            .map(|e| {
                Ok::<_, anyhow::Error>(EventSummary {
                    id: e.id,
                    source: e.source.parse().map_err(anyhow::Error::msg)?,
                    source_ref: e.source_ref,
                    event_kind: e.event_kind.parse().map_err(anyhow::Error::msg)?,
                    source_ts: e.source_ts,
                    observed_at: e.observed_at,
                    processed_at: e.processed_at,
                    outcome: e.outcome,
                })
            })
            .collect::<Result<_, _>>()?,
    }))
}

pub async fn list_breaks(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Query(q): Query<BreaksQuery>,
) -> Result<Json<Vec<BreakView>>, ApiError> {
    let all = q.status.as_deref() == Some("all");
    let limit = q.limit.unwrap_or(500).clamp(1, 5000);
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let rows: Vec<BreakRow> = sqlx::query_as(concat!(
        "SELECT b.id, b.kind, b.deal_id, d.deal_ref, b.source, b.source_ref, b.field, b.desk_id, k.code AS desk_code, \
                b.details, b.status, b.detected_at, b.acknowledged_at, b.resolved_at, b.resolution, b.note, b.updated_at \
         FROM breaks b JOIN desks k ON k.id = b.desk_id LEFT JOIN deals d ON d.id = b.deal_id",
        " WHERE ($1 OR b.status <> 'RESOLVED') AND ($2::smallint IS NULL OR b.desk_id = $2) \
          ORDER BY b.status = 'RESOLVED', b.detected_at DESC, b.id DESC LIMIT $3"
    ))
    .bind(all)
    .bind(q.desk_id)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(rows.into_iter().map(BreakView::try_from).collect::<Result<_, _>>()?))
}

pub async fn get_break(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<Json<BreakDetail>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let break_ = load_break(&mut tx, id).await?;
    let actions: Vec<(DateTime<Utc>, Option<String>, String, Option<String>)> = sqlx::query_as(
        "SELECT a.at, u.display_name, a.action, a.note FROM break_actions a LEFT JOIN users u ON u.id = a.user_id \
         WHERE a.break_id = $1 ORDER BY a.id",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(BreakDetail {
        break_,
        actions: actions.into_iter().map(|(at, user, action, note)| BreakActionView { at, user, action, note }).collect(),
    }))
}

pub async fn ack_break(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<(StatusCode, Json<BreakView>), ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    sqlx::query("SELECT break_ack($1)").bind(id).execute(&mut *tx).await?;
    let view = load_break(&mut tx, id).await?;
    tx.commit().await?;
    let ctx = AuditCtx::new(Some(peer), &headers);
    audit::write(&state.pool, Some(auth.user.id), &auth.user.username, "break_ack", &ctx, Some(serde_json::json!({ "break_id": id }))).await;
    Ok((StatusCode::ACCEPTED, Json(view)))
}

pub async fn resolve_break(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<i64>,
    Json(req): Json<ResolveRequest>,
) -> Result<(StatusCode, Json<BreakView>), ApiError> {
    if req.note.as_deref().is_some_and(|n| n.chars().count() > 500) {
        return Err(ApiError::BadRequest("note is too long (500 characters)".into()));
    }
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let event_id: i64 = sqlx::query_scalar("SELECT break_resolve($1, $2, $3, $4)")
        .bind(id)
        .bind(req.action.as_str())
        .bind(req.deal_id)
        .bind(req.note.as_deref().map(str::trim))
        .fetch_one(&mut *tx)
        .await?;
    let view = load_break(&mut tx, id).await?;
    tx.commit().await?;
    let ctx = AuditCtx::new(Some(peer), &headers);
    audit::write(
        &state.pool,
        Some(auth.user.id),
        &auth.user.username,
        "break_resolve",
        &ctx,
        Some(serde_json::json!({ "break_id": id, "action": req.action.as_str(), "deal_id": req.deal_id, "event_id": event_id })),
    )
    .await;
    Ok((StatusCode::ACCEPTED, Json(view)))
}

pub async fn feeds(State(state): State<AppState>, Extension(auth): Extension<AuthUser>) -> Result<Json<FeedsResponse>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let feeds: Vec<(String, Option<DateTime<Utc>>, Option<DateTime<Utc>>, Option<DateTime<Utc>>, i64, i64)> = sqlx::query_as(
        "SELECT source, last_heartbeat_at, last_event_at, last_source_ts, events_total, duplicates_dropped FROM feed_status ORDER BY source",
    )
    .fetch_all(&mut *tx)
    .await?;
    let recon: Option<(String, DateTime<Utc>, DateTime<Utc>, i64, Value)> =
        sqlx::query_as("SELECT name, now, last_tick_at, pending, detail FROM recon_status WHERE name = 'worker'").fetch_optional(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(FeedsResponse {
        feeds: feeds
            .into_iter()
            .filter_map(|(s, hb, ev, ts, n, d)| {
                Some(FeedStatus { source: s.parse().ok()?, last_heartbeat_at: hb, last_event_at: ev, last_source_ts: ts, events_total: n, duplicates_dropped: d })
            })
            .collect(),
        recon: recon.map(|(name, now, last_tick_at, pending, detail)| ReconStatus { name, now, last_tick_at, pending, detail }),
        server_time: Utc::now(),
    }))
}
