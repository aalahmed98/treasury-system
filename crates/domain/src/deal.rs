use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! text_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name {
            $(#[serde(rename = $text)] $variant),+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $text),+ }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = String;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($text => Ok($name::$variant),)+
                    other => Err(format!("unknown {}: {other}", stringify!($name))),
                }
            }
        }
    };
}

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
    /// Deal lifecycle. Mirrors the reconciliation states agreed in the design discussion.
    DealStatus {
        Executed => "EXECUTED",
        Booked => "BOOKED",
        Confirmed => "CONFIRMED",
        Settled => "SETTLED",
        Cancelled => "CANCELLED",
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
}

/// A deal as seen by an entitled user. Desk and trader names are joined in by the server.
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
    pub trader_id: Uuid,
    pub trader_name: String,
    pub status: DealStatus,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
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
