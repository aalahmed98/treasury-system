//! Reconciliation types: what one source says about a deal, breaks, and feed freshness.

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Deal, DealStatus, Product, Side, Source};

text_enum! {
    BreakKind {
        Missing => "MISSING",
        Mismatch => "MISMATCH",
        Orphan => "ORPHAN",
        Ambiguous => "AMBIGUOUS",
        Duplicate => "DUPLICATE",
    }
}

text_enum! {
    BreakStatus {
        Open => "OPEN",
        Acknowledged => "ACKNOWLEDGED",
        Resolved => "RESOLVED",
    }
}

text_enum! {
    Resolution {
        Auto => "AUTO",
        Linked => "LINKED",
        Dismissed => "DISMISSED",
        Superseded => "SUPERSEDED",
    }
}

text_enum! {
    LinkKind {
        SelfRef => "SELF",
        CrossRef => "CROSS_REF",
        Auto => "AUTO",
        Manual => "MANUAL",
    }
}

text_enum! {
    EventKind {
        New => "NEW",
        Amend => "AMEND",
        Cancel => "CANCEL",
        Status => "STATUS",
        User => "USER",
    }
}

text_enum! {
    ResolveAction {
        Link => "LINK",
        Dismiss => "DISMISS",
    }
}

/// The latest image one source holds for a deal, and how it is linked to the golden record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceDealView {
    pub source: Source,
    pub source_ref: String,
    pub deal_id: Option<Uuid>,
    pub link_kind: Option<LinkKind>,
    pub linked_at: Option<DateTime<Utc>>,
    pub lifecycle: DealStatus,
    pub raw_state: Option<String>,
    pub applied_ts: DateTime<Utc>,
    pub first_seen_at: DateTime<Utc>,
    pub desk_id: i16,
    pub product: Product,
    pub side: Side,
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
    pub cross_refs: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BreakView {
    pub id: i64,
    pub kind: BreakKind,
    pub deal_id: Option<Uuid>,
    pub deal_ref: Option<String>,
    pub source: Option<Source>,
    pub source_ref: Option<String>,
    pub field: Option<String>,
    pub desk_id: i16,
    pub desk_code: String,
    pub details: serde_json::Value,
    pub status: BreakStatus,
    pub detected_at: DateTime<Utc>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolution: Option<Resolution>,
    pub note: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BreakActionView {
    pub at: DateTime<Utc>,
    pub user: Option<String>,
    pub action: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventSummary {
    pub id: i64,
    pub source: Source,
    pub source_ref: String,
    pub event_kind: EventKind,
    pub source_ts: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub processed_at: Option<DateTime<Utc>>,
    pub outcome: Option<String>,
}

/// Everything known about one golden deal across sources.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceResponse {
    pub deal: Deal,
    pub sources: Vec<SourceDealView>,
    pub breaks: Vec<BreakView>,
    pub events: Vec<EventSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BreakDetail {
    pub break_: BreakView,
    pub actions: Vec<BreakActionView>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BreaksQuery {
    /// "open" (default) or "all".
    pub status: Option<String>,
    pub desk_id: Option<i16>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveRequest {
    pub action: ResolveAction,
    pub deal_id: Option<Uuid>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedStatus {
    pub source: Source,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub last_event_at: Option<DateTime<Utc>>,
    pub last_source_ts: Option<DateTime<Utc>>,
    pub events_total: i64,
    pub duplicates_dropped: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReconStatus {
    pub name: String,
    /// The reconciler's clock, which is the one to age feeds against.
    pub now: DateTime<Utc>,
    pub last_tick_at: DateTime<Utc>,
    pub pending: i64,
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedsResponse {
    pub feeds: Vec<FeedStatus>,
    pub recon: Option<ReconStatus>,
    pub server_time: DateTime<Utc>,
}
