//! Liquidity ladder, nostro movements, amendments and deal history.

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{DealStatus, MatchStatus};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LadderQuery {
    pub ccy: Option<String>,
    /// First date shown; defaults to two days before today so just-settled flows stay visible.
    pub from: Option<NaiveDate>,
    /// Number of days from `from`; default 30, at most 120.
    pub days: Option<i64>,
}

/// One value date of one currency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LadderRow {
    pub date: NaiveDate,
    pub confirmed_in: Decimal,
    pub confirmed_out: Decimal,
    pub provisional_in: Decimal,
    pub provisional_out: Decimal,
    pub net_expected: Decimal,
    pub cumulative_expected: Decimal,
    pub deals: i64,
    /// Nostro actuals, only for users entitled to bank-wide liquidity data.
    pub actual_in: Option<Decimal>,
    pub actual_out: Option<Decimal>,
    pub actual_net: Option<Decimal>,
    /// actual_net minus net_expected where actuals exist.
    pub difference: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LadderOpening {
    pub balance: Decimal,
    pub statement_date: NaiveDate,
    pub kind: String,
    pub received_at: DateTime<Utc>,
    pub bank: String,
    pub account_no: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LadderResponse {
    pub ccy: String,
    pub currencies: Vec<String>,
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// Whether this user may see nostro statements and movements.
    pub liquidity_access: bool,
    pub opening: Option<LadderOpening>,
    pub rows: Vec<LadderRow>,
    pub generated_at: DateTime<Utc>,
}

/// A single expected cash flow, for drilling into a ladder row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CashFlow {
    pub deal_id: Uuid,
    pub deal_ref: String,
    pub desk_code: String,
    pub flow_date: NaiveDate,
    pub ccy: String,
    pub amount: Decimal,
    pub leg: String,
    pub confirmed: bool,
    pub status: DealStatus,
    pub match_status: MatchStatus,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FlowsQuery {
    pub ccy: String,
    pub date: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NostroMovement {
    pub id: i64,
    pub ccy: String,
    pub value_date: NaiveDate,
    pub booked_at: DateTime<Utc>,
    pub amount: Decimal,
    pub reference: String,
    pub counterparty: Option<String>,
    pub deal_ref_hint: Option<String>,
    pub source: String,
}

/// Fields a booker may change on an internal deal. Everything omitted stays as it is.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AmendDeal {
    pub counterparty: Option<String>,
    pub rate: Option<Decimal>,
    pub notional: Option<Decimal>,
    pub value_date: Option<NaiveDate>,
    pub maturity_date: Option<NaiveDate>,
    pub status: Option<DealStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldChange {
    pub field: String,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub version: i32,
    pub op: String,
    pub changed_at: DateTime<Utc>,
    pub changed_by: Option<String>,
    pub changed_by_role: String,
    pub changes: Vec<FieldChange>,
}
