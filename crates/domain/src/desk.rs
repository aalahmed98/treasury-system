use serde::{Deserialize, Serialize};

/// A trading or oversight desk. `id` is the database key, `code` is the short label shown in the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Desk {
    pub id: i16,
    pub code: String,
    pub name: String,
}

/// One row of a user's entitlement matrix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeskAccess {
    pub desk_id: i16,
    pub code: String,
    pub name: String,
    pub can_book: bool,
}
