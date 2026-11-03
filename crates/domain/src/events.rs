use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::BreakStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DealOp {
    Insert,
    Update,
}

/// What the server pushes over the event stream. Deliberately carries no deal or break data:
/// the client fetches by id so the database re-checks entitlement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Deal {
        op: DealOp,
        id: Uuid,
        desk_id: i16,
        version: i32,
    },
    Break {
        id: i64,
        desk_id: i16,
        deal_id: Option<Uuid>,
        status: BreakStatus,
    },
    /// The server may have missed notifications; clients should reload everything.
    Resync,
}

impl ServerEvent {
    pub fn desk_id(&self) -> Option<i16> {
        match self {
            ServerEvent::Deal { desk_id, .. } | ServerEvent::Break { desk_id, .. } => Some(*desk_id),
            ServerEvent::Resync => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            ServerEvent::Deal { .. } => "deal",
            ServerEvent::Break { .. } => "break",
            ServerEvent::Resync => "resync",
        }
    }
}
