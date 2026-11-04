//! Market risk of the dealing book: the bank's FX limits (a versioned document like the ALM
//! parameters), and the shapes of the risk, P&L, exposure and breach views. Every amount is in the
//! base currency (BHD) unless a field says otherwise.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------------------------

/// A counterparty's credit lines for FX: the most the bank may be owed on unsettled deals if the
/// counterparty fails before settlement, and the most it may pay away on one value date before
/// receiving the other leg.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CounterpartyLine {
    pub pre_settlement: f64,
    pub settlement: f64,
}

impl Default for CounterpartyLine {
    fn default() -> Self {
        Self { pre_settlement: 1_000_000.0, settlement: 15_000_000.0 }
    }
}

/// The bank's dealing limits. Placeholders until the board-approved limits arrive; every field has
/// a default so an older stored version keeps parsing after keys are added.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RiskLimits {
    pub base_ccy: String,
    /// Currencies held to the dollar and so to the dinar; their open position carries peg risk only.
    pub pegged: Vec<String>,
    /// Aggregate net open position (shorthand method), end of day and at any moment of the day.
    pub nop_overnight_total: f64,
    pub nop_intraday_total: f64,
    /// Per currency overnight limits; currencies not listed take the pegged or floating default.
    pub nop_overnight_ccy: BTreeMap<String, f64>,
    pub nop_default_floating: f64,
    pub nop_default_pegged: f64,
    /// Aggregate open position one dealer may run.
    pub dealer_nop: f64,
    /// Losses that force the desk to square off: one day, and the month so far.
    pub stop_loss_daily: f64,
    pub stop_loss_monthly: f64,
    /// One-day 99% historical value at risk.
    pub var_limit: f64,
    /// Distance from the market mid, in basis points, beyond which a deal is off-market.
    pub off_market_bp_spot: f64,
    pub off_market_bp_forward: f64,
    pub counterparty_lines: BTreeMap<String, CounterpartyLine>,
    pub default_line: CounterpartyLine,
    /// Utilisation, percent, from which a limit shows as "watch".
    pub warn_pct: f64,
}

impl Default for RiskLimits {
    fn default() -> Self {
        let line = |p: f64, s: f64| CounterpartyLine { pre_settlement: p, settlement: s };
        Self {
            base_ccy: "BHD".into(),
            pegged: ["USD", "SAR", "AED", "QAR", "OMR", "BHD"].into_iter().map(String::from).collect(),
            nop_overnight_total: 120_000_000.0,
            nop_intraday_total: 160_000_000.0,
            nop_overnight_ccy: BTreeMap::new(),
            nop_default_floating: 30_000_000.0,
            nop_default_pegged: 80_000_000.0,
            dealer_nop: 90_000_000.0,
            stop_loss_daily: 150_000.0,
            stop_loss_monthly: 500_000.0,
            var_limit: 400_000.0,
            off_market_bp_spot: 50.0,
            off_market_bp_forward: 100.0,
            counterparty_lines: [
                ("CITIBANK N.A.", line(4_000_000.0, 60_000_000.0)),
                ("JPMORGAN CHASE", line(4_000_000.0, 60_000_000.0)),
                ("HSBC BAHRAIN", line(3_000_000.0, 40_000_000.0)),
                ("STANDARD CHARTERED", line(3_000_000.0, 40_000_000.0)),
                ("EMIRATES NBD", line(2_500_000.0, 30_000_000.0)),
                ("QATAR NATIONAL BANK", line(2_500_000.0, 30_000_000.0)),
                ("ARAB BANKING CORPORATION", line(2_000_000.0, 25_000_000.0)),
                ("GULF INTERNATIONAL BANK", line(2_000_000.0, 25_000_000.0)),
                ("AL RAJHI BANK", line(1_500_000.0, 20_000_000.0)),
                ("AHLI UNITED BANK", line(1_500_000.0, 20_000_000.0)),
                ("AL SALAM BANK", line(1_000_000.0, 15_000_000.0)),
                ("BBK", line(1_000_000.0, 15_000_000.0)),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
            default_line: CounterpartyLine::default(),
            warn_pct: 80.0,
        }
    }
}

impl RiskLimits {
    pub fn is_pegged(&self, ccy: &str) -> bool {
        self.pegged.iter().any(|p| p == ccy)
    }

    pub fn nop_limit(&self, ccy: &str) -> f64 {
        self.nop_overnight_ccy.get(ccy).copied().unwrap_or(if self.is_pegged(ccy) { self.nop_default_pegged } else { self.nop_default_floating })
    }

    pub fn line(&self, counterparty: &str) -> CounterpartyLine {
        self.counterparty_lines.get(&normalise_counterparty(counterparty)).copied().unwrap_or(self.default_line)
    }
}

/// The key a counterparty's lines are held under: upper case, single spaces.
pub fn normalise_counterparty(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ").to_uppercase()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskLimitsVersion {
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<String>,
    pub note: Option<String>,
    pub is_active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskLimitsResponse {
    pub active: RiskLimitsVersion,
    pub limits: RiskLimits,
    pub history: Vec<RiskLimitsVersion>,
    /// Whether the caller may save a new version (the treasury head).
    pub can_edit: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveLimitsRequest {
    pub limits: RiskLimits,
    pub note: String,
}

// ---------------------------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------------------------

text_enum! {
    /// Where a figure stands against its limit.
    LimitStatus {
        Ok => "OK",
        Watch => "WATCH",
        Breach => "BREACH",
    }
}

/// One figure against one limit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitUse {
    /// Stable key, e.g. `nop.total`, `nop.ccy.EUR`, `stop.daily`, `var`, `cpty.pse.CITIBANK N.A.`.
    pub key: String,
    pub label: String,
    /// The figure as an absolute amount: a loss as a positive number, a position as its size.
    pub value: f64,
    pub limit: f64,
    pub pct: f64,
    pub status: LimitStatus,
}

/// The bank's net position in one currency, from every FX deal traded up to the day.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CcyPosition {
    pub ccy: String,
    /// In the currency.
    pub amount: f64,
    /// In BHD at the day's mid; `None` when the currency has no rate.
    pub bhd: Option<f64>,
    pub pegged: bool,
    pub spot: Option<f64>,
    pub limit: f64,
    pub pct: f64,
    pub status: LimitStatus,
}

/// Net amounts still to settle, per currency, by how far away the value date is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FwdLadderRow {
    pub ccy: String,
    /// In the currency, one per bucket.
    pub amounts: Vec<f64>,
    /// In BHD, one per bucket.
    pub bhd: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForwardLadder {
    pub buckets: Vec<String>,
    pub rows: Vec<FwdLadderRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PnlSummary {
    /// Change in the book's value since the previous close, including the day's new deals.
    pub today: f64,
    /// The day's new deals valued at the day's mid: the spread the desk earned on them.
    pub new_deals: f64,
    /// The rest of the day's change: positions held overnight, moved by the market and carry.
    pub market_move: f64,
    pub mtd: f64,
    pub ytd: f64,
    /// Everything traded, valued at the day's market.
    pub total: f64,
    /// Of the total, deals not yet settled (mark-to-market of open contracts).
    pub open_mtm: f64,
    /// Of the total, settled deals: currency balances revalued at the day's spot.
    pub settled: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PnlBy {
    pub key: String,
    pub today: f64,
    pub mtd: f64,
    pub ytd: f64,
    pub deals: u32,
    /// The aggregate open position this key carries (shorthand method, BHD).
    pub nop: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CounterpartyExposure {
    pub name: String,
    pub deals: u32,
    /// Net mark-to-market of unsettled deals with this counterparty (positive: they owe the bank).
    pub mtm: f64,
    /// Potential future exposure: notional times the add-on factor for each deal's remaining life.
    pub add_on: f64,
    pub pre_settlement: f64,
    pub pre_settlement_line: f64,
    pub pre_settlement_pct: f64,
    /// The largest amount the bank pays the counterparty on any one value date ahead.
    pub settlement_peak: f64,
    pub settlement_peak_date: Option<NaiveDate>,
    pub settlement_line: f64,
    pub settlement_pct: f64,
    pub status: LimitStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CcyVar {
    pub ccy: String,
    pub position: f64,
    pub var99: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct VarResult {
    /// One-day 99% historical value at risk of today's open positions, BHD, as a positive loss.
    pub var99: f64,
    /// The average of the losses beyond the VaR.
    pub es99: f64,
    /// The worst one-day 99% VaR over any 250-day window in the history held.
    pub stressed_var99: f64,
    pub stressed_window_end: Option<NaiveDate>,
    /// Daily returns used for today's VaR.
    pub observations: u32,
    pub by_ccy: Vec<CcyVar>,
}

/// One FX deal valued at the day's market.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FxDealValue {
    pub deal_id: Uuid,
    pub deal_ref: String,
    pub desk_code: String,
    pub trader: String,
    pub counterparty: String,
    pub pair: String,
    pub side: String,
    pub notional: f64,
    pub rate: f64,
    pub trade_date: NaiveDate,
    pub value_date: NaiveDate,
    /// The market rate for the deal's value date on the valuation day.
    pub market_rate: Option<f64>,
    /// BHD; `None` when a currency of the deal has no rate.
    pub mtm: Option<f64>,
    pub settled: bool,
    /// The market mid for the deal's value date on its trade date, and the deal's distance from it.
    pub trade_mid: Option<f64>,
    pub off_market_bp: Option<f64>,
    pub off_market: bool,
    /// Value on the trade date at that day's mid: the spread captured, BHD.
    pub day_one: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FxRiskResult {
    pub as_of: NaiveDate,
    pub base_ccy: String,
    pub limits_version: Option<i32>,
    pub positions: Vec<CcyPosition>,
    pub nop_long: f64,
    pub nop_short: f64,
    /// Shorthand aggregate: the larger of total longs and total shorts.
    pub nop_total: f64,
    pub nop_floating: f64,
    pub nop_pegged: f64,
    /// The largest aggregate position reached during the day, taking the day's deals in booking order.
    pub nop_intraday_peak: f64,
    pub ladder: ForwardLadder,
    pub pnl: PnlSummary,
    pub by_trader: Vec<PnlBy>,
    pub by_pair: Vec<PnlBy>,
    pub counterparties: Vec<CounterpartyExposure>,
    pub var: VarResult,
    pub limits: Vec<LimitUse>,
    /// Deals priced away from the market on their trade date, worst first.
    pub off_market: Vec<FxDealValue>,
    /// Currencies dealt that have no market rate, so their deals are not valued.
    pub unpriced: Vec<String>,
    /// FX deals cancelled in the last 30 days.
    pub cancelled_30d: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FxHistoryPoint {
    pub date: NaiveDate,
    pub pnl_day: f64,
    pub pnl_total: f64,
    pub nop_total: f64,
    pub nop_floating: f64,
    pub var99: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FxHistory {
    pub points: Vec<FxHistoryPoint>,
}

/// A day a figure stood outside its limit, and who has signed it off.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Breach {
    pub date: NaiveDate,
    pub key: String,
    pub label: String,
    pub value: f64,
    pub limit: f64,
    pub signed_by: Option<String>,
    pub signed_at: Option<DateTime<Utc>>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignOffRequest {
    pub date: NaiveDate,
    pub key: String,
    pub note: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DaysQuery {
    pub days: Option<i64>,
    pub as_of: Option<NaiveDate>,
}

// ---------------------------------------------------------------------------------------------
// The ALCO pack
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlcoNote {
    pub body: String,
    pub updated_by: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlcoSignoff {
    pub signed_by: Option<String>,
    pub signed_at: DateTime<Utc>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlcoPack {
    /// The first of the month the pack is for.
    pub period: NaiveDate,
    pub notes: BTreeMap<String, AlcoNote>,
    pub signoff: Option<AlcoSignoff>,
    /// Whether the caller may sign it off (the treasury head).
    pub can_sign: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PeriodQuery {
    pub period: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackNoteRequest {
    pub period: NaiveDate,
    pub section: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackSignRequest {
    pub period: NaiveDate,
    pub note: Option<String>,
}
