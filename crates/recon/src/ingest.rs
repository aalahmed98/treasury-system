//! Appending observations to the event log. Used by adapters, the synthetic feed and tests.

use chrono::{DateTime, Utc};
use domain::{EventKind, Source};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgExecutor;

pub fn dedup_hash(source: Source, source_ref: &str, kind: EventKind, payload: &Value) -> Vec<u8> {
    let text = format!("{}|{}|{}|{}", source.as_str(), source_ref, kind.as_str(), payload);
    Sha256::digest(text.as_bytes()).to_vec()
}

/// Returns the new event id, or None when an identical event was already in the log.
#[allow(clippy::too_many_arguments)]
pub async fn insert_event<'e>(
    exec: impl PgExecutor<'e>,
    source: Source,
    source_ref: &str,
    kind: EventKind,
    source_seq: Option<i64>,
    source_ts: DateTime<Utc>,
    observed_at: DateTime<Utc>,
    payload: &Value,
    desk_id: i16,
) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar(
        "INSERT INTO source_events (source, source_ref, event_kind, source_seq, source_ts, observed_at, payload, desk_id, dedup_hash) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
         ON CONFLICT (source, dedup_hash) DO NOTHING RETURNING id",
    )
    .bind(source.as_str())
    .bind(source_ref)
    .bind(kind.as_str())
    .bind(source_seq)
    .bind(source_ts)
    .bind(observed_at)
    .bind(payload)
    .bind(desk_id)
    .bind(dedup_hash(source, source_ref, kind, payload))
    .fetch_optional(exec)
    .await
}

pub async fn unassigned_desk<'e>(exec: impl PgExecutor<'e>) -> sqlx::Result<i16> {
    sqlx::query_scalar("SELECT id FROM desks WHERE code = 'UNASSIGNED'").fetch_one(exec).await
}
