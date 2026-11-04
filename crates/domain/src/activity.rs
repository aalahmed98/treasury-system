//! Recent activity on the caller's desks and what needs attention, both computed by the server.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivityRow {
    pub at: DateTime<Utc>,
    pub kind: String,
    pub actor: String,
    pub summary: String,
    pub deal_id: Option<Uuid>,
    pub break_id: Option<i64>,
    pub desk_code: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActivityQuery {
    pub limit: Option<i64>,
}

text_enum! {
    AttentionSeverity {
        Critical => "CRITICAL",
        Warning => "WARNING",
        Info => "INFO",
    }
}

/// Where the client goes when an item is clicked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AttentionTarget {
    Deal { id: Uuid },
    Break { id: i64 },
    Breaks,
    Provisional,
    Overview,
    Alm { tool: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttentionItem {
    pub severity: AttentionSeverity,
    pub title: String,
    pub detail: String,
    pub since: Option<DateTime<Utc>>,
    pub target: AttentionTarget,
}
