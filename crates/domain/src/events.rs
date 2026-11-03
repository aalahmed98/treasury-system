use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DealOp {
    Insert,
    Update,
}

/// What the server pushes over the event stream. Deliberately carries no deal data:
/// the client fetches the deal by id so the database re-checks entitlement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Deal {
        op: DealOp,
        id: Uuid,
        desk_id: i16,
        version: i32,
    },
    /// The server may have missed notifications; clients should reload everything.
    Resync,
}
