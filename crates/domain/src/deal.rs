use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

text_enum! {
    /// Instrument type. The text form is what the database stores.
    Product {
        FxSpot => "FX_SPOT",
        FxFwd => "FX_FWD",
        MmDepo => "MM_DEPO",
        MmLoan => "MM_LOAN",
    }
}

text_enum! {
    /// Direction from the bank's point of view. FX uses Buy/Sell of ccy1; money market uses Lend/Borrow.
    Side {
        Buy => "BUY",
        Sell => "SELL",
        Lend => "LEND",
        Borrow => "BORROW",
    }
}

text_enum! {
    /// Deal lifecycle, both for a golden deal and for what one source currently reports.
    DealStatus {
        Executed => "EXECUTED",
        Booked => "BOOKED",
        Confirmed => "CONFIRMED",
        Settled => "SETTLED",
        Cancelled => "CANCELLED",
    }
}

text_enum! {
    /// How far a golden deal has been reconciled across the sources expected to carry it.
    MatchStatus {
        Unmatched => "UNMATCHED",
        Partial => "PARTIAL",
        Matched => "MATCHED",
        Break => "BREAK",
    }
}

text_enum! {
    /// A system that observes deals. `User` is the pseudo-source for manual resolutions.
    Source {
        Kondor => "KONDOR",
        Bloomberg => "BLOOMBERG",
        Quantum => "QUANTUM",
        Internal => "INTERNAL",
        User => "USER",
    }
}

impl Product {
    pub fn is_fx(self) -> bool {
        matches!(self, Product::FxSpot | Product::FxFwd)
    }
}

impl Side {
    /// Positive means the bank's position in ccy1 increases.
    pub fn sign(self) -> i8 {
        match self {
            Side::Buy | Side::Lend => 1,
            Side::Sell | Side::Borrow => -1,
        }
    }

    pub fn flipped(self) -> Side {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
            Side::Lend => Side::Borrow,
            Side::Borrow => Side::Lend,
        }
    }
}

impl DealStatus {
    /// Ordering along the lifecycle; CANCELLED sits outside it.
    pub fn rank(self) -> u8 {
        match self {
            DealStatus::Executed => 0,
            DealStatus::Booked => 1,
            DealStatus::Confirmed => 2,
            DealStatus::Settled => 3,
            DealStatus::Cancelled => 9,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, DealStatus::Settled | DealStatus::Cancelled)
    }
}

impl Source {
    pub fn short(self) -> &'static str {
        match self {
            Source::Kondor => "K",
            Source::Bloomberg => "B",
            Source::Quantum => "Q",
            Source::Internal => "I",
            Source::User => "U",
        }
    }
}

/// A golden deal as seen by an entitled user. Desk and trader names are joined in by the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Deal {
    pub id: Uuid,
    pub deal_ref: String,
    pub desk_id: i16,
    pub desk_code: String,
    pub product: Product,
    pub side: Side,
    pub ccy1: String,
    pub ccy2: Option<String>,
    pub notional: Decimal,
    pub rate: Decimal,
    pub trade_date: NaiveDate,
    pub value_date: NaiveDate,
    pub maturity_date: Option<NaiveDate>,
    pub counterparty: String,
    /// None for deals the reconciler created from an external source.
    pub trader_id: Option<Uuid>,
    pub trader_name: String,
    pub status: DealStatus,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Which source first produced this golden deal.
    pub origin: Source,
    pub match_status: MatchStatus,
    /// A confirming source (back office) has reached CONFIRMED or beyond.
    pub confirmed: bool,
    /// Sources currently linked to this deal, sorted.
    pub sources: Vec<Source>,
}

/// Net position per desk and currency, computed by the database.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub desk_id: i16,
    pub desk_code: String,
    pub ccy: String,
    pub net_notional: Decimal,
    pub deal_count: i64,
}
