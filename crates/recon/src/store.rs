//! Row types and the SQL the pipeline needs. Everything here runs as the reconciler role inside
//! the caller's transaction; no identity is set because the recon policies are unconditional.

use chrono::{DateTime, NaiveDate, Utc};
use domain::{DealStatus, LinkKind, Product, Side, Source};
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use std::str::FromStr;
use uuid::Uuid;

use crate::normalise::NormalisedDeal;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EventRow {
    pub id: i64,
    pub source: String,
    pub source_ref: String,
    pub event_kind: String,
    pub source_seq: Option<i64>,
    pub source_ts: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub payload: Value,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SourceDealRow {
    pub source: String,
    pub source_ref: String,
    pub deal_id: Option<Uuid>,
    pub link_kind: Option<String>,
    pub lifecycle: String,
    pub raw_state: Option<String>,
    pub applied_event_id: i64,
    pub applied_seq: Option<i64>,
    pub applied_ts: DateTime<Utc>,
    pub first_seen_at: DateTime<Utc>,
    pub desk_id: i16,
    pub cross_refs: Value,
    pub product: String,
    pub side: String,
    pub ccy1: String,
    pub ccy2: Option<String>,
    pub notional: Decimal,
    pub rate: Decimal,
    pub trade_date: NaiveDate,
    pub value_date: NaiveDate,
    pub maturity_date: Option<NaiveDate>,
    pub counterparty_raw: String,
    pub counterparty: String,
    pub trader_ref: Option<String>,
}

impl SourceDealRow {
    pub fn source(&self) -> Source {
        Source::from_str(&self.source).expect("source codes are constrained by the schema")
    }

    pub fn lifecycle(&self) -> DealStatus {
        DealStatus::from_str(&self.lifecycle).expect("constrained by the schema")
    }

    /// The same image as a normalised deal, so an unlinked row can be re-matched.
    pub fn as_norm(&self) -> NormalisedDeal {
        let mut cross_refs = std::collections::BTreeMap::new();
        if let Value::Object(m) = &self.cross_refs {
            for (k, v) in m {
                if let (Ok(s), Some(r)) = (Source::from_str(k), v.as_str()) {
                    cross_refs.insert(s, r.to_string());
                }
            }
        }
        NormalisedDeal {
            product: Product::from_str(&self.product).expect("constrained"),
            side: Side::from_str(&self.side).expect("constrained"),
            ccy1: self.ccy1.trim().to_string(),
            ccy2: self.ccy2.as_deref().map(|c| c.trim().to_string()),
            notional: self.notional,
            rate: self.rate,
            trade_date: self.trade_date,
            value_date: self.value_date,
            maturity_date: self.maturity_date,
            counterparty_raw: self.counterparty_raw.clone(),
            counterparty: self.counterparty.clone(),
            trader_ref: self.trader_ref.clone(),
            lifecycle: self.lifecycle(),
            raw_state: self.raw_state.clone(),
            desk_code: None,
            desk_id: Some(self.desk_id),
            cross_refs,
            self_deal_id: None,
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DealRow {
    pub id: Uuid,
    pub deal_ref: String,
    pub desk_id: i16,
    pub product: String,
    pub side: String,
    pub ccy1: String,
    pub ccy2: Option<String>,
    pub notional: Decimal,
    pub rate: Decimal,
    pub trade_date: NaiveDate,
    pub value_date: NaiveDate,
    pub maturity_date: Option<NaiveDate>,
    pub counterparty: String,
    pub status: String,
    pub origin: String,
    pub match_status: String,
    pub confirmed: bool,
    pub merged_into: Option<Uuid>,
}

impl DealRow {
    pub fn status(&self) -> DealStatus {
        DealStatus::from_str(&self.status).expect("constrained")
    }

    pub fn origin(&self) -> Source {
        Source::from_str(&self.origin).expect("constrained")
    }

    pub fn product(&self) -> Product {
        Product::from_str(&self.product).expect("constrained")
    }
}

pub const DEAL_COLS: &str = "id, deal_ref, desk_id, product, side, ccy1::text AS ccy1, ccy2::text AS ccy2, notional, rate, \
                             trade_date, value_date, maturity_date, counterparty, status, origin, match_status, confirmed, merged_into";

pub const SD_COLS: &str = "source, source_ref, deal_id, link_kind, lifecycle, raw_state, applied_event_id, applied_seq, applied_ts, \
                           first_seen_at, desk_id, cross_refs, product, side, ccy1::text AS ccy1, ccy2::text AS ccy2, notional, rate, \
                           trade_date, value_date, maturity_date, counterparty_raw, counterparty, trader_ref";

#[derive(Debug, Clone)]
pub struct SourceSystem {
    pub code: Source,
    pub may_originate: bool,
    pub confirms: bool,
    pub cancels: bool,
    pub precedence: i16,
}

pub async fn load_source_systems(pool: &PgPool) -> sqlx::Result<Vec<SourceSystem>> {
    let rows: Vec<(String, bool, bool, bool, i16)> =
        sqlx::query_as("SELECT code, may_originate, confirms, cancels, precedence FROM source_systems").fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .filter_map(|(c, o, cf, cn, p)| {
            Source::from_str(&c).ok().map(|code| SourceSystem { code, may_originate: o, confirms: cf, cancels: cn, precedence: p })
        })
        .collect())
}

pub async fn pending_events(tx: &mut PgConnection, limit: i64) -> sqlx::Result<Vec<EventRow>> {
    sqlx::query_as(
        "SELECT id, source, source_ref, event_kind, source_seq, source_ts, observed_at, payload \
         FROM source_events WHERE processed_at IS NULL ORDER BY id LIMIT $1 FOR UPDATE",
    )
    .bind(limit)
    .fetch_all(tx)
    .await
}

pub async fn source_deal(tx: &mut PgConnection, source: Source, source_ref: &str) -> sqlx::Result<Option<SourceDealRow>> {
    sqlx::query_as(format!("SELECT {SD_COLS} FROM source_deals WHERE source = $1 AND source_ref = $2 FOR UPDATE").leak_static())
        .bind(source.as_str())
        .bind(source_ref)
        .fetch_optional(tx)
        .await
}

pub async fn source_deals_of(tx: &mut PgConnection, deal_id: Uuid) -> sqlx::Result<Vec<SourceDealRow>> {
    sqlx::query_as(format!("SELECT {SD_COLS} FROM source_deals WHERE deal_id = $1 ORDER BY source").leak_static())
        .bind(deal_id)
        .fetch_all(tx)
        .await
}

pub async fn deal(tx: &mut PgConnection, id: Uuid) -> sqlx::Result<Option<DealRow>> {
    sqlx::query_as(format!("SELECT {DEAL_COLS} FROM deals WHERE id = $1 FOR UPDATE").leak_static()).bind(id).fetch_optional(tx).await
}

/// Golden deals in the same economic bucket as a normalised image.
pub async fn bucket_deals(
    tx: &mut PgConnection,
    n: &NormalisedDeal,
    notional_lo: Decimal,
    notional_hi: Decimal,
    value_lo: NaiveDate,
    value_hi: NaiveDate,
) -> sqlx::Result<Vec<(DealRow, Vec<String>)>> {
    #[derive(sqlx::FromRow)]
    struct Row {
        #[sqlx(flatten)]
        deal: DealRow,
        linked: Vec<String>,
    }
    let rows: Vec<Row> = sqlx::query_as(
        format!(
            "SELECT {DEAL_COLS}, array(SELECT s.source FROM source_deals s WHERE s.deal_id = d.id) AS linked \
             FROM deals d \
             WHERE d.product = $1 AND d.ccy1 = $2 AND d.ccy2 IS NOT DISTINCT FROM $3 AND d.trade_date = $4 AND d.side = $5 \
               AND d.notional BETWEEN $6 AND $7 AND d.value_date BETWEEN $8 AND $9 \
               AND d.status <> 'CANCELLED' AND d.merged_into IS NULL"
        )
        .replace("SELECT id,", "SELECT d.id,")
        .leak_static(),
    )
    .bind(n.product.as_str())
    .bind(&n.ccy1)
    .bind(&n.ccy2)
    .bind(n.trade_date)
    .bind(n.side.as_str())
    .bind(notional_lo)
    .bind(notional_hi)
    .bind(value_lo)
    .bind(value_hi)
    .fetch_all(tx)
    .await?;
    Ok(rows.into_iter().map(|r| (r.deal, r.linked)).collect())
}

/// Unlinked source rows in the same bucket as a golden deal.
pub async fn unlinked_in_bucket(
    tx: &mut PgConnection,
    d: &DealRow,
    notional_lo: Decimal,
    notional_hi: Decimal,
    value_lo: NaiveDate,
    value_hi: NaiveDate,
) -> sqlx::Result<Vec<SourceDealRow>> {
    sqlx::query_as(
        format!(
            "SELECT {SD_COLS} FROM source_deals \
             WHERE deal_id IS NULL AND product = $1 AND ccy1 = $2 AND ccy2 IS NOT DISTINCT FROM $3 AND trade_date = $4 AND side = $5 \
               AND notional BETWEEN $6 AND $7 AND value_date BETWEEN $8 AND $9 AND lifecycle <> 'CANCELLED' \
             ORDER BY first_seen_at, source, source_ref FOR UPDATE"
        )
        .leak_static(),
    )
    .bind(&d.product)
    .bind(d.ccy1.trim())
    .bind(d.ccy2.as_deref().map(str::trim))
    .bind(d.trade_date)
    .bind(&d.side)
    .bind(notional_lo)
    .bind(notional_hi)
    .bind(value_lo)
    .bind(value_hi)
    .fetch_all(tx)
    .await
}

/// A linked source row whose cross references name this (source, ref).
pub async fn deal_by_reverse_cross_ref(tx: &mut PgConnection, source: Source, source_ref: &str) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar(
        "SELECT deal_id FROM source_deals WHERE deal_id IS NOT NULL AND cross_refs @> jsonb_build_object($1::text, $2::text) \
         ORDER BY linked_at LIMIT 1",
    )
    .bind(source.as_str())
    .bind(source_ref)
    .fetch_optional(tx)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_source_deal(
    tx: &mut PgConnection,
    source: Source,
    source_ref: &str,
    n: &NormalisedDeal,
    event: &EventRow,
    desk_id: i16,
    now: DateTime<Utc>,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO source_deals (source, source_ref, lifecycle, raw_state, applied_event_id, applied_seq, applied_ts, first_seen_at, \
                                   desk_id, cross_refs, product, side, ccy1, ccy2, notional, rate, trade_date, value_date, maturity_date, \
                                   counterparty_raw, counterparty, trader_ref, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23)",
    )
    .bind(source.as_str())
    .bind(source_ref)
    .bind(n.lifecycle.as_str())
    .bind(&n.raw_state)
    .bind(event.id)
    .bind(event.source_seq)
    .bind(event.source_ts)
    .bind(event.observed_at)
    .bind(desk_id)
    .bind(serde_json::to_value(&n.cross_refs).unwrap_or(Value::Null))
    .bind(n.product.as_str())
    .bind(n.side.as_str())
    .bind(&n.ccy1)
    .bind(&n.ccy2)
    .bind(n.notional)
    .bind(n.rate)
    .bind(n.trade_date)
    .bind(n.value_date)
    .bind(n.maturity_date)
    .bind(&n.counterparty_raw)
    .bind(&n.counterparty)
    .bind(&n.trader_ref)
    .bind(now)
    .execute(tx)
    .await?;
    Ok(())
}

pub async fn update_source_deal_image(
    tx: &mut PgConnection,
    source: Source,
    source_ref: &str,
    n: &NormalisedDeal,
    event: &EventRow,
    now: DateTime<Utc>,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE source_deals SET lifecycle = $3, raw_state = $4, applied_event_id = $5, applied_seq = $6, applied_ts = $7, \
                cross_refs = cross_refs || $8, product = $9, side = $10, ccy1 = $11, ccy2 = $12, notional = $13, rate = $14, \
                trade_date = $15, value_date = $16, maturity_date = $17, counterparty_raw = $18, counterparty = $19, \
                trader_ref = $20, updated_at = $21 \
         WHERE source = $1 AND source_ref = $2",
    )
    .bind(source.as_str())
    .bind(source_ref)
    .bind(n.lifecycle.as_str())
    .bind(&n.raw_state)
    .bind(event.id)
    .bind(event.source_seq)
    .bind(event.source_ts)
    .bind(serde_json::to_value(&n.cross_refs).unwrap_or(Value::Null))
    .bind(n.product.as_str())
    .bind(n.side.as_str())
    .bind(&n.ccy1)
    .bind(&n.ccy2)
    .bind(n.notional)
    .bind(n.rate)
    .bind(n.trade_date)
    .bind(n.value_date)
    .bind(n.maturity_date)
    .bind(&n.counterparty_raw)
    .bind(&n.counterparty)
    .bind(&n.trader_ref)
    .bind(now)
    .execute(tx)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn link_source_deal(
    tx: &mut PgConnection,
    source: Source,
    source_ref: &str,
    deal_id: Uuid,
    kind: LinkKind,
    event_id: i64,
    by: Option<Uuid>,
    score: Option<i16>,
    now: DateTime<Utc>,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE source_deals SET deal_id = $3, link_kind = $4, link_event_id = $5, linked_by = $6, linked_at = $7, link_score = $8, \
                desk_id = (SELECT desk_id FROM deals WHERE id = $3), updated_at = $7 \
         WHERE source = $1 AND source_ref = $2",
    )
    .bind(source.as_str())
    .bind(source_ref)
    .bind(deal_id)
    .bind(kind.as_str())
    .bind(event_id)
    .bind(by)
    .bind(now)
    .bind(score)
    .bind(now)
    .execute(tx)
    .await?;
    Ok(())
}

pub async fn create_golden(
    tx: &mut PgConnection,
    n: &NormalisedDeal,
    desk_id: i16,
    desk_code: &str,
    origin: Source,
    created_at: DateTime<Utc>,
) -> sqlx::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, trade_date, value_date, maturity_date, \
                            counterparty, trader_id, status, origin, match_status, created_at, updated_at) \
         VALUES ($1, $2 || '-' || lpad(nextval('deal_ref_seq')::text, 6, '0'), $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, \
                 $13, NULL, $14, $15, 'UNMATCHED', $16, $16)",
    )
    .bind(id)
    .bind(desk_code)
    .bind(desk_id)
    .bind(n.product.as_str())
    .bind(n.side.as_str())
    .bind(&n.ccy1)
    .bind(&n.ccy2)
    .bind(n.notional)
    .bind(n.rate)
    .bind(n.trade_date)
    .bind(n.value_date)
    .bind(n.maturity_date)
    .bind(&n.counterparty)
    .bind(n.lifecycle.as_str())
    .bind(origin.as_str())
    .bind(created_at)
    .execute(tx)
    .await?;
    Ok(id)
}

/// Reconciliation-owned columns of a golden deal (economics only change for external-origin deals).
pub struct GoldenUpdate<'a> {
    pub status: DealStatus,
    pub confirmed: bool,
    pub match_status: domain::MatchStatus,
    pub economics: Option<&'a NormalisedDeal>,
    pub merged_into: Option<Uuid>,
}

pub async fn update_golden(tx: &mut PgConnection, id: Uuid, u: &GoldenUpdate<'_>) -> sqlx::Result<()> {
    match u.economics {
        Some(n) => {
            sqlx::query(
                "UPDATE deals SET status = $2, confirmed = $3, match_status = $4, merged_into = coalesce($5, merged_into), \
                        product = $6, side = $7, ccy1 = $8, ccy2 = $9, notional = $10, rate = $11, trade_date = $12, \
                        value_date = $13, maturity_date = $14, counterparty = $15 \
                 WHERE id = $1 AND (status IS DISTINCT FROM $2 OR confirmed IS DISTINCT FROM $3 OR match_status IS DISTINCT FROM $4 \
                        OR merged_into IS DISTINCT FROM coalesce($5, merged_into) OR product IS DISTINCT FROM $6 \
                        OR side IS DISTINCT FROM $7 OR ccy1 IS DISTINCT FROM $8 OR ccy2 IS DISTINCT FROM $9 \
                        OR notional IS DISTINCT FROM $10 OR rate IS DISTINCT FROM $11 OR trade_date IS DISTINCT FROM $12 \
                        OR value_date IS DISTINCT FROM $13 OR maturity_date IS DISTINCT FROM $14 OR counterparty IS DISTINCT FROM $15)",
            )
            .bind(id)
            .bind(u.status.as_str())
            .bind(u.confirmed)
            .bind(u.match_status.as_str())
            .bind(u.merged_into)
            .bind(n.product.as_str())
            .bind(n.side.as_str())
            .bind(&n.ccy1)
            .bind(&n.ccy2)
            .bind(n.notional)
            .bind(n.rate)
            .bind(n.trade_date)
            .bind(n.value_date)
            .bind(n.maturity_date)
            .bind(&n.counterparty)
            .execute(tx)
            .await?;
        }
        None => {
            sqlx::query(
                "UPDATE deals SET status = $2, confirmed = $3, match_status = $4, merged_into = coalesce($5, merged_into) \
                 WHERE id = $1 AND (status IS DISTINCT FROM $2 OR confirmed IS DISTINCT FROM $3 OR match_status IS DISTINCT FROM $4 \
                        OR merged_into IS DISTINCT FROM coalesce($5, merged_into))",
            )
            .bind(id)
            .bind(u.status.as_str())
            .bind(u.confirmed)
            .bind(u.match_status.as_str())
            .bind(u.merged_into)
            .execute(tx)
            .await?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BreakRow {
    pub id: i64,
    pub kind: String,
    pub deal_id: Option<Uuid>,
    pub source: Option<String>,
    pub source_ref: Option<String>,
    pub field: Option<String>,
    pub desk_id: i16,
    pub details: Value,
    pub status: String,
}

/// Raise a break unless an identical open one exists, or an identical one was dismissed by a user
/// (a dismissal stands until the observation changes).
#[allow(clippy::too_many_arguments)]
pub async fn raise_break(
    tx: &mut PgConnection,
    kind: domain::BreakKind,
    deal_id: Option<Uuid>,
    source: Option<Source>,
    source_ref: Option<&str>,
    field: Option<&str>,
    desk_id: i16,
    details: Value,
    now: DateTime<Utc>,
) -> sqlx::Result<Option<i64>> {
    let dismissed: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM breaks WHERE kind = $1 AND deal_id IS NOT DISTINCT FROM $2 AND source IS NOT DISTINCT FROM $3 \
           AND source_ref IS NOT DISTINCT FROM $4 AND field IS NOT DISTINCT FROM $5 AND resolution = 'DISMISSED' \
           AND details->>'observed' IS NOT DISTINCT FROM $6->>'observed')",
    )
    .bind(kind.as_str())
    .bind(deal_id)
    .bind(source.map(|s| s.as_str()))
    .bind(source_ref)
    .bind(field)
    .bind(&details)
    .fetch_one(&mut *tx)
    .await?;
    if dismissed {
        return Ok(None);
    }
    sqlx::query_scalar(
        "INSERT INTO breaks (kind, deal_id, source, source_ref, field, desk_id, details, detected_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8) \
         ON CONFLICT (break_key) WHERE status <> 'RESOLVED' DO NOTHING RETURNING id",
    )
    .bind(kind.as_str())
    .bind(deal_id)
    .bind(source.map(|s| s.as_str()))
    .bind(source_ref)
    .bind(field)
    .bind(desk_id)
    .bind(details)
    .bind(now)
    .fetch_optional(tx)
    .await
}

/// Resolve every open break matching the filter. Returns how many were closed.
pub async fn resolve_breaks(
    tx: &mut PgConnection,
    kind: Option<domain::BreakKind>,
    deal_id: Option<Uuid>,
    source: Option<Source>,
    source_ref: Option<&str>,
    field: Option<&str>,
    resolution: domain::Resolution,
    by: Option<Uuid>,
    extra: Value,
    now: DateTime<Utc>,
) -> sqlx::Result<u64> {
    let r = sqlx::query(
        "UPDATE breaks SET status = 'RESOLVED', resolution = $6, resolved_by = $7, resolved_at = $8, updated_at = $8, \
                details = details || $9 \
         WHERE status <> 'RESOLVED' \
           AND ($1::text IS NULL OR kind = $1) AND ($2::uuid IS NULL OR deal_id = $2) \
           AND ($3::text IS NULL OR source = $3) AND ($4::text IS NULL OR source_ref = $4) AND ($5::text IS NULL OR field = $5)",
    )
    .bind(kind.map(|k| k.as_str()))
    .bind(deal_id)
    .bind(source.map(|s| s.as_str()))
    .bind(source_ref)
    .bind(field)
    .bind(resolution.as_str())
    .bind(by)
    .bind(now)
    .bind(extra)
    .execute(tx)
    .await?;
    Ok(r.rows_affected())
}

/// Close MISSING breaks for a source that has now arrived, recording how late it was.
pub async fn resolve_missing(tx: &mut PgConnection, deal_id: Uuid, source: Source, now: DateTime<Utc>) -> sqlx::Result<u64> {
    let r = sqlx::query(
        "UPDATE breaks SET status = 'RESOLVED', resolution = 'AUTO', resolved_at = $3, updated_at = $3, \
                details = details || jsonb_build_object('late_by_secs', \
                    greatest(0, extract(epoch FROM ($3 - (details->>'deadline')::timestamptz)))::bigint) \
         WHERE status <> 'RESOLVED' AND kind = 'MISSING' AND deal_id = $1 AND source = $2",
    )
    .bind(deal_id)
    .bind(source.as_str())
    .bind(now)
    .execute(tx)
    .await?;
    Ok(r.rows_affected())
}

pub async fn resolve_break_by_id(
    tx: &mut PgConnection,
    id: i64,
    resolution: domain::Resolution,
    by: Option<Uuid>,
    note: Option<&str>,
    now: DateTime<Utc>,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE breaks SET status = 'RESOLVED', resolution = $2, resolved_by = $3, resolved_at = $4, updated_at = $4, note = coalesce($5, note) \
         WHERE id = $1",
    )
    .bind(id)
    .bind(resolution.as_str())
    .bind(by)
    .bind(now)
    .bind(note)
    .execute(tx)
    .await?;
    Ok(())
}

pub async fn open_break_count(tx: &mut PgConnection, deal_id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar("SELECT count(*) FROM breaks WHERE deal_id = $1 AND status <> 'RESOLVED'").bind(deal_id).fetch_one(tx).await
}

pub async fn break_by_id(tx: &mut PgConnection, id: i64) -> sqlx::Result<Option<BreakRow>> {
    sqlx::query_as("SELECT id, kind, deal_id, source, source_ref, field, desk_id, details, status FROM breaks WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_optional(tx)
        .await
}

pub async fn mark_event(
    tx: &mut PgConnection,
    id: i64,
    now: DateTime<Utc>,
    outcome: &str,
    detail: Value,
    norm: Option<&NormalisedDeal>,
    desk_id: Option<i16>,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE source_events SET processed_at = $2, outcome = $3, outcome_detail = $4, \
                norm = coalesce($5, norm), normaliser_version = CASE WHEN $5 IS NULL THEN normaliser_version ELSE $6 END, \
                desk_id = coalesce($7, desk_id) \
         WHERE id = $1",
    )
    .bind(id)
    .bind(now)
    .bind(outcome)
    .bind(detail)
    .bind(norm.map(|n| serde_json::to_value(n).unwrap_or(Value::Null)))
    .bind(crate::normalise::NORMALISER_VERSION)
    .bind(desk_id)
    .execute(tx)
    .await?;
    Ok(())
}

/// sqlx 0.9 wants static SQL for its safe-string check; these are built from constants only.
trait LeakStatic {
    fn leak_static(&self) -> &'static str;
}

impl LeakStatic for String {
    fn leak_static(&self) -> &'static str {
        static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, &'static str>>> = std::sync::OnceLock::new();
        let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
        let mut c = cache.lock().unwrap();
        if let Some(s) = c.get(self) {
            return s;
        }
        let leaked: &'static str = Box::leak(self.clone().into_boxed_str());
        c.insert(self.clone(), leaked);
        leaked
    }
}
