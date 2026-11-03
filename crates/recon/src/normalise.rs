//! From a source's raw image to one canonical shape. Versioned: bump `NORMALISER_VERSION` when
//! the mapping changes so stored `norm` columns can be recomputed.

use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

use chrono::NaiveDate;
use domain::{DealStatus, Product, Side, Source};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const NORMALISER_VERSION: i16 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalisedDeal {
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
    /// Canonical name after cleaning and alias lookup; what matching compares.
    pub counterparty: String,
    pub trader_ref: Option<String>,
    pub lifecycle: DealStatus,
    pub raw_state: Option<String>,
    /// Source-side book / portfolio / account code; the pipeline maps it to a desk.
    pub desk_code: Option<String>,
    /// Internal bookings carry the desk id directly.
    pub desk_id: Option<i16>,
    pub cross_refs: BTreeMap<Source, String>,
    /// Internal bookings are self-identifying: the golden deal id.
    pub self_deal_id: Option<Uuid>,
}

#[derive(Debug, thiserror::Error)]
pub enum NormaliseError {
    #[error("missing field {0}")]
    Missing(&'static str),
    #[error("bad value for {0}: {1}")]
    Bad(&'static str, String),
    #[error("{0} is not a deal source")]
    Unsupported(Source),
}

pub struct Normaliser {
    aliases: HashMap<String, String>,
}

impl Normaliser {
    pub fn new(aliases: impl IntoIterator<Item = (String, String)>) -> Self {
        Self { aliases: aliases.into_iter().collect() }
    }

    pub fn normalise(&self, source: Source, payload: &Value) -> Result<NormalisedDeal, NormaliseError> {
        let mut n = match source {
            Source::Kondor => self.kondor(payload)?,
            Source::Bloomberg => self.bloomberg(payload)?,
            Source::Quantum => self.quantum(payload)?,
            Source::Internal => self.internal(payload)?,
            Source::User => return Err(NormaliseError::Unsupported(source)),
        };
        n.counterparty = self.canonical_counterparty(&n.counterparty_raw);
        if let Some(c2) = n.ccy2.clone() {
            if should_flip(&n.ccy1, &c2) {
                flip_pair(&mut n);
            }
        }
        Ok(n)
    }

    pub fn canonical_counterparty(&self, raw: &str) -> String {
        let norm = normalise_name(raw);
        self.aliases.get(&norm).cloned().unwrap_or(norm)
    }

    fn kondor(&self, p: &Value) -> Result<NormalisedDeal, NormaliseError> {
        let product = match str_field(p, "instrument")? {
            "FXSPOT" => Product::FxSpot,
            "FXFWD" => Product::FxFwd,
            "DEPOSIT" => Product::MmDepo,
            "LOAN" => Product::MmLoan,
            other => return Err(NormaliseError::Bad("instrument", other.into())),
        };
        let state = str_field(p, "status")?;
        let lifecycle = match state {
            "NEW" | "VERIFIED" => DealStatus::Booked,
            "CANCELLED" => DealStatus::Cancelled,
            other => return Err(NormaliseError::Bad("status", other.into())),
        };
        Ok(NormalisedDeal {
            product,
            side: side_field(p, "direction")?,
            ccy1: str_field(p, "ccy1")?.to_string(),
            ccy2: opt_str(p, "ccy2"),
            notional: dec_field(p, "amount")?,
            rate: dec_field(p, "rate")?,
            trade_date: date_field(p, "trade_date", "%Y-%m-%d")?,
            value_date: date_field(p, "value_date", "%Y-%m-%d")?,
            maturity_date: opt_date(p, "maturity_date", "%Y-%m-%d")?,
            counterparty_raw: str_field(p, "counterparty")?.to_string(),
            counterparty: String::new(),
            trader_ref: opt_str(p, "trader"),
            lifecycle,
            raw_state: Some(state.to_string()),
            desk_code: opt_str(p, "portfolio"),
            desk_id: None,
            cross_refs: refs(p.get("refs")),
            self_deal_id: None,
        })
    }

    fn bloomberg(&self, p: &Value) -> Result<NormalisedDeal, NormaliseError> {
        let product = match str_field(p, "product")? {
            "SPOT" => Product::FxSpot,
            "FWD" => Product::FxFwd,
            "DEPO" => Product::MmDepo,
            "LOAN" => Product::MmLoan,
            other => return Err(NormaliseError::Bad("product", other.into())),
        };
        let (ccy1, ccy2) = if product.is_fx() {
            let pair = str_field(p, "pair")?;
            let (a, b) = pair.split_once('/').ok_or_else(|| NormaliseError::Bad("pair", pair.into()))?;
            (a.to_string(), Some(b.to_string()))
        } else {
            (str_field(p, "ccy")?.to_string(), None)
        };
        let side = match str_field(p, "side")? {
            "B" => Side::Buy,
            "S" => Side::Sell,
            "L" => Side::Lend,
            "BO" => Side::Borrow,
            other => return Err(NormaliseError::Bad("side", other.into())),
        };
        let state = str_field(p, "state")?;
        let lifecycle = match state {
            "DONE" => DealStatus::Executed,
            "CXL" => DealStatus::Cancelled,
            other => return Err(NormaliseError::Bad("state", other.into())),
        };
        Ok(NormalisedDeal {
            product,
            side,
            ccy1,
            ccy2,
            notional: dec_field(p, "qty")?,
            rate: dec_field(p, "price")?,
            trade_date: date_field(p, "trade_dt", "%Y%m%d")?,
            value_date: date_field(p, "settle_dt", "%Y%m%d")?,
            maturity_date: opt_date(p, "maturity_dt", "%Y%m%d")?,
            counterparty_raw: str_field(p, "cpty")?.to_string(),
            counterparty: String::new(),
            trader_ref: opt_str(p, "trader"),
            lifecycle,
            raw_state: Some(state.to_string()),
            desk_code: opt_str(p, "account"),
            desk_id: None,
            cross_refs: refs(p.get("refs")),
            self_deal_id: None,
        })
    }

    fn quantum(&self, p: &Value) -> Result<NormalisedDeal, NormaliseError> {
        let product = Product::from_str(str_field(p, "type")?).map_err(|e| NormaliseError::Bad("type", e))?;
        let state = str_field(p, "status")?;
        let lifecycle = match state {
            "PENDING" => DealStatus::Booked,
            "CONFIRMED" => DealStatus::Confirmed,
            "SETTLED" => DealStatus::Settled,
            "CANCELLED" => DealStatus::Cancelled,
            other => return Err(NormaliseError::Bad("status", other.into())),
        };
        let mut cross_refs = refs(p.get("refs"));
        if let Some(k) = opt_str(p, "front_ref") {
            cross_refs.insert(Source::Kondor, k);
        }
        Ok(NormalisedDeal {
            product,
            side: side_field(p, "direction")?,
            ccy1: str_field(p, "ccy1")?.to_string(),
            ccy2: opt_str(p, "ccy2"),
            notional: dec_field(p, "amount")?,
            rate: dec_field(p, "rate")?,
            trade_date: date_field(p, "trade_date", "%Y-%m-%d")?,
            value_date: date_field(p, "value_date", "%Y-%m-%d")?,
            maturity_date: opt_date(p, "maturity_date", "%Y-%m-%d")?,
            counterparty_raw: str_field(p, "counterparty")?.to_string(),
            counterparty: String::new(),
            trader_ref: None,
            lifecycle,
            raw_state: Some(state.to_string()),
            desk_code: opt_str(p, "book"),
            desk_id: None,
            cross_refs,
            self_deal_id: None,
        })
    }

    /// The image the deals table writes for internal bookings (`to_jsonb(deals)` minus annotations).
    fn internal(&self, p: &Value) -> Result<NormalisedDeal, NormaliseError> {
        let product = Product::from_str(str_field(p, "product")?).map_err(|e| NormaliseError::Bad("product", e))?;
        let status = str_field(p, "status")?;
        let lifecycle = DealStatus::from_str(status).map_err(|e| NormaliseError::Bad("status", e))?;
        let id = Uuid::parse_str(str_field(p, "id")?).map_err(|e| NormaliseError::Bad("id", e.to_string()))?;
        Ok(NormalisedDeal {
            product,
            side: side_field(p, "side")?,
            ccy1: str_field(p, "ccy1")?.trim().to_string(),
            ccy2: opt_str(p, "ccy2").map(|c| c.trim().to_string()),
            notional: dec_field(p, "notional")?,
            rate: dec_field(p, "rate")?,
            trade_date: date_field(p, "trade_date", "%Y-%m-%d")?,
            value_date: date_field(p, "value_date", "%Y-%m-%d")?,
            maturity_date: opt_date(p, "maturity_date", "%Y-%m-%d")?,
            counterparty_raw: str_field(p, "counterparty")?.to_string(),
            counterparty: String::new(),
            trader_ref: opt_str(p, "trader_id"),
            lifecycle,
            raw_state: Some(status.to_string()),
            desk_code: None,
            desk_id: p.get("desk_id").and_then(Value::as_i64).map(|d| d as i16),
            cross_refs: refs(p.get("refs")),
            self_deal_id: Some(id),
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Pair convention: the more "major" currency is ccy1. A source quoting the other way round is
// re-expressed, which flips the side, inverts the rate and converts the notional.
// ---------------------------------------------------------------------------------------------

fn priority(ccy: &str) -> u8 {
    match ccy {
        "EUR" => 0,
        "GBP" => 1,
        "AUD" => 2,
        "NZD" => 3,
        "USD" => 4,
        "CAD" => 5,
        "CHF" => 6,
        "JPY" => 7,
        _ => 100,
    }
}

pub fn should_flip(ccy1: &str, ccy2: &str) -> bool {
    let (a, b) = (priority(ccy1), priority(ccy2));
    a > b || (a == b && ccy1 > ccy2)
}

fn flip_pair(n: &mut NormalisedDeal) {
    let Some(c2) = n.ccy2.take() else { return };
    let amount_in_c2 = (n.notional * n.rate).round_dp(2);
    let inverted = if n.rate.is_zero() { Decimal::ZERO } else { (Decimal::ONE / n.rate).round_dp(8) };
    n.ccy2 = Some(std::mem::replace(&mut n.ccy1, c2));
    n.notional = amount_in_c2;
    n.rate = inverted;
    n.side = n.side.flipped();
}

// ---------------------------------------------------------------------------------------------
// Counterparty names: upper-case, punctuation to spaces, trailing legal suffixes removed.
// ---------------------------------------------------------------------------------------------

const SUFFIXES: &[&str] = &["BSC", "BSCC", "PLC", "LTD", "LIMITED", "LLC", "NA", "SA", "AG", "INC", "WLL", "SAL", "PJSC", "QPSC", "SAOG"];

pub fn normalise_name(raw: &str) -> String {
    let upper = raw.to_uppercase().replace("N.A.", "NA").replace("B.S.C.", "BSC").replace("B.S.C", "BSC");
    let cleaned: String = upper.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' }).collect();
    let mut tokens: Vec<&str> = cleaned.split_whitespace().collect();
    while tokens.len() > 1 && SUFFIXES.contains(tokens.last().unwrap()) {
        tokens.pop();
    }
    tokens.join(" ")
}

// ---------------------------------------------------------------------------------------------
// Field helpers
// ---------------------------------------------------------------------------------------------

fn str_field<'a>(p: &'a Value, key: &'static str) -> Result<&'a str, NormaliseError> {
    match p.get(key) {
        Some(Value::String(s)) if !s.is_empty() => Ok(s),
        Some(Value::Null) | None => Err(NormaliseError::Missing(key)),
        Some(other) => Err(NormaliseError::Bad(key, other.to_string())),
    }
}

fn opt_str(p: &Value, key: &str) -> Option<String> {
    match p.get(key) {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

fn dec_field(p: &Value, key: &'static str) -> Result<Decimal, NormaliseError> {
    match p.get(key) {
        Some(Value::Number(n)) => Decimal::from_str(&n.to_string()).map_err(|e| NormaliseError::Bad(key, e.to_string())),
        Some(Value::String(s)) => Decimal::from_str(s).map_err(|e| NormaliseError::Bad(key, e.to_string())),
        Some(Value::Null) | None => Err(NormaliseError::Missing(key)),
        Some(other) => Err(NormaliseError::Bad(key, other.to_string())),
    }
}

fn date_field(p: &Value, key: &'static str, fmt: &str) -> Result<NaiveDate, NormaliseError> {
    let s = str_field(p, key)?;
    NaiveDate::parse_from_str(s, fmt).map_err(|e| NormaliseError::Bad(key, format!("{s}: {e}")))
}

fn opt_date(p: &Value, key: &'static str, fmt: &str) -> Result<Option<NaiveDate>, NormaliseError> {
    match p.get(key) {
        Some(Value::String(s)) if !s.is_empty() => {
            NaiveDate::parse_from_str(s, fmt).map(Some).map_err(|e| NormaliseError::Bad(key, format!("{s}: {e}")))
        }
        _ => Ok(None),
    }
}

fn side_field(p: &Value, key: &'static str) -> Result<Side, NormaliseError> {
    Side::from_str(str_field(p, key)?).map_err(|e| NormaliseError::Bad(key, e))
}

fn refs(v: Option<&Value>) -> BTreeMap<Source, String> {
    let mut out = BTreeMap::new();
    if let Some(Value::Object(m)) = v {
        for (k, v) in m {
            if let (Ok(src), Some(s)) = (Source::from_str(k), v.as_str()) {
                out.insert(src, s.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dec(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn normaliser() -> Normaliser {
        Normaliser::new([("GIB".to_string(), "GULF INTERNATIONAL BANK".to_string())])
    }

    #[test]
    fn names_are_cleaned_and_aliased() {
        let n = normaliser();
        assert_eq!(normalise_name("Gulf International Bank B.S.C."), "GULF INTERNATIONAL BANK");
        assert_eq!(normalise_name("Citibank N.A."), "CITIBANK");
        assert_eq!(normalise_name("JPMorgan Chase"), "JPMORGAN CHASE");
        assert_eq!(normalise_name("Arab Banking Corporation"), "ARAB BANKING CORPORATION");
        assert_eq!(n.canonical_counterparty("GIB"), "GULF INTERNATIONAL BANK");
        assert_eq!(n.canonical_counterparty("gib"), "GULF INTERNATIONAL BANK");
        assert_eq!(n.canonical_counterparty("Unknown Bank Ltd"), "UNKNOWN BANK");
    }

    #[test]
    fn kondor_and_quantum_map_to_the_same_shape() {
        let n = normaliser();
        let k = n
            .normalise(
                Source::Kondor,
                &json!({"deal_id":"K1","portfolio":"FX-BOOK","instrument":"FXSPOT","direction":"BUY","ccy1":"USD","ccy2":"BHD",
                        "amount":1000000,"rate":0.376,"trade_date":"2026-10-05","value_date":"2026-10-07","maturity_date":null,
                        "counterparty":"Gulf Intl Bank B.S.C.","trader":"FXT1","status":"NEW","refs":{"BLOOMBERG":"VCON9"}}),
            )
            .unwrap();
        let q = n
            .normalise(
                Source::Quantum,
                &json!({"trade_no":"Q1","book":"TRS-FX","type":"FX_SPOT","direction":"BUY","ccy1":"USD","ccy2":"BHD","amount":"1000000.00",
                        "rate":"0.37600","trade_date":"2026-10-05","value_date":"2026-10-07","counterparty":"GULF INTL BANK","status":"CONFIRMED","front_ref":"K1"}),
            )
            .unwrap();
        assert_eq!((k.product, k.side, k.ccy1.as_str(), k.ccy2.as_deref()), (Product::FxSpot, Side::Buy, "USD", Some("BHD")));
        assert_eq!(k.notional, q.notional);
        assert_eq!(k.rate, q.rate);
        assert_eq!(k.counterparty, "GULF INTL BANK", "no alias row for this variant yet");
        assert_eq!(k.lifecycle, DealStatus::Booked);
        assert_eq!(q.lifecycle, DealStatus::Confirmed);
        assert_eq!(k.cross_refs.get(&Source::Bloomberg).map(String::as_str), Some("VCON9"));
        assert_eq!(q.cross_refs.get(&Source::Kondor).map(String::as_str), Some("K1"));
        assert_eq!(k.desk_code.as_deref(), Some("FX-BOOK"));
    }

    #[test]
    fn reversed_bloomberg_quote_is_re_expressed() {
        let n = normaliser();
        // We bought USD 1,000,000 against BHD at 0.37600. Bloomberg shows it as selling BHD 376,000 at 2.65957 USD per BHD.
        let b = n
            .normalise(
                Source::Bloomberg,
                &json!({"ticket":"VCON9","account":"NBB-FX","product":"SPOT","side":"S","pair":"BHD/USD","qty":376000,"price":2.65957,
                        "trade_dt":"20261005","settle_dt":"20261007","cpty":"GIB","state":"DONE"}),
            )
            .unwrap();
        assert_eq!((b.ccy1.as_str(), b.ccy2.as_deref(), b.side), ("USD", Some("BHD"), Side::Buy));
        assert_eq!(b.notional, dec("999998.32"), "qty x price, with the quote's rounding");
        assert_eq!(b.rate, dec("0.37600063"));
        assert_eq!(b.lifecycle, DealStatus::Executed);
        assert_eq!(b.counterparty, "GULF INTERNATIONAL BANK");
    }

    #[test]
    fn internal_image_is_self_identifying() {
        let n = normaliser();
        let id = Uuid::now_v7();
        let i = n
            .normalise(
                Source::Internal,
                &json!({"id":id.to_string(),"deal_ref":"FX-000001","desk_id":1,"product":"FX_FWD","side":"SELL","ccy1":"EUR","ccy2":"USD",
                        "notional":2000000.0000,"rate":1.085,"trade_date":"2026-10-05","value_date":"2026-11-05","maturity_date":null,
                        "counterparty":"HSBC Bahrain","trader_id":"u1","status":"BOOKED","created_at":"2026-10-05T08:00:00Z","origin":"INTERNAL"}),
            )
            .unwrap();
        assert_eq!(i.self_deal_id, Some(id));
        assert_eq!(i.desk_id, Some(1));
        assert_eq!(i.notional, dec("2000000"));
        assert_eq!(i.rate, dec("1.085"));
    }

    #[test]
    fn missing_fields_are_errors_not_panics() {
        let n = normaliser();
        assert!(matches!(n.normalise(Source::Kondor, &json!({"deal_id":"K1"})), Err(NormaliseError::Missing("instrument"))));
        assert!(n.normalise(Source::User, &json!({})).is_err());
    }
}
