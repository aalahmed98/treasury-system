//! Asset and liability management: the banking book as cohorts, the bank's parameter set, and the
//! inputs and outputs of the ALM calculators. Pure data shapes; the maths lives in the `alm` crate.
//! Analytics carry `f64` (they are estimates, not accounting); every ratio whose denominator can
//! be zero is an `Option`, because JSON has no NaN.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

text_enum! {
    /// Which side of the balance sheet a cohort sits on.
    BsSide {
        Asset => "ASSET",
        Liability => "LIABILITY",
        OffBalance => "OFF_BALANCE",
    }
}

text_enum! {
    /// How a cohort's rate behaves. ADMIN is a non-maturity deposit whose rate the bank sets and
    /// moves by a pass-through beta; NONE is not rate sensitive at all (equity, cash, other).
    RateType {
        Fixed => "FIXED",
        Float => "FLOAT",
        Admin => "ADMIN",
        NotSensitive => "NONE",
    }
}

text_enum! {
    Amort {
        Bullet => "BULLET",
        Linear => "LINEAR",
        Annuity => "ANNUITY",
    }
}

text_enum! {
    HqlaLevel {
        L1 => "L1",
        L2A => "L2A",
        L2B => "L2B",
    }
}

text_enum! {
    DayCount {
        Act360 => "ACT/360",
        Act365 => "ACT/365",
    }
}

impl DayCount {
    pub fn basis(self) -> f64 {
        match self {
            DayCount::Act360 => 360.0,
            DayCount::Act365 => 365.0,
        }
    }
}

/// One cohort of the banking book on one snapshot date: a segment, product, currency, rate type
/// and maturity bucket aggregated into a single row with stable `position_id` across snapshots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BsPosition {
    pub as_of: NaiveDate,
    pub position_id: String,
    pub side: BsSide,
    pub book: String,
    pub segment: String,
    pub product: String,
    pub ccy: String,
    pub balance: f64,
    /// Annual rate in percent, like `deals.rate`.
    pub rate: f64,
    pub rate_type: RateType,
    pub index_code: Option<String>,
    /// Spread over the index in percent.
    pub spread: f64,
    pub reset_months: Option<i16>,
    /// Payment frequency in months; `None` pays at maturity.
    pub pay_months: Option<i16>,
    pub amort: Amort,
    pub next_repricing: Option<NaiveDate>,
    pub maturity: Option<NaiveDate>,
    pub origination: Option<NaiveDate>,
    pub day_count: DayCount,
    pub hqla_level: Option<HqlaLevel>,
    pub encumbered: bool,
    /// LCR outflow class for liabilities and commitments, inflow class for assets.
    pub lcr_class: Option<String>,
    /// NSFR type; the residual-maturity bucket is derived from `maturity`.
    pub nsfr_class: Option<String>,
    /// Deposit rate pass-through for ADMIN cohorts.
    pub beta: f64,
    pub behavioural_months: Option<i16>,
    /// Undrawn amount of a commitment (OFF_BALANCE rows).
    pub undrawn: f64,
}

// ---------------------------------------------------------------------------------------------
// Parameters: the bank's limits and assumptions. Basel III defaults; every section has defaults so
// an older stored version still parses after keys are added.
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AlmParams {
    pub reporting_ccy: String,
    pub tier1_capital: f64,
    pub lcr: LcrParams,
    pub nsfr: NsfrParams,
    pub gap: GapParams,
    pub stress: StressParams,
    pub nii: NiiParams,
    pub funding: FundingParams,
    pub liquidity_gap: LiquidityGapParams,
    pub ewi: EwiParams,
}

impl Default for AlmParams {
    fn default() -> Self {
        Self {
            reporting_ccy: "BHD".into(),
            tier1_capital: 580_000_000.0,
            lcr: LcrParams::default(),
            nsfr: NsfrParams::default(),
            gap: GapParams::default(),
            stress: StressParams::default(),
            nii: NiiParams::default(),
            funding: FundingParams::default(),
            liquidity_gap: LiquidityGapParams::default(),
            ewi: EwiParams::default(),
        }
    }
}

fn map<const N: usize>(pairs: [(&str, f64); N]) -> BTreeMap<String, f64> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LcrParams {
    /// Regulatory minimum, percent.
    pub minimum: f64,
    /// The bank's own floor, percent.
    pub internal_minimum: f64,
    /// Early-warning level, percent.
    pub early_warning: f64,
    pub significant_ccys: Vec<String>,
    /// Percent haircut per HQLA level.
    pub haircuts: BTreeMap<String, f64>,
    /// Level 2 assets may be at most this percent of the stock.
    pub cap_l2_total_pct: f64,
    /// Level 2B assets may be at most this percent of the stock.
    pub cap_l2b_pct: f64,
    /// Inflows may offset at most this percent of outflows.
    pub inflow_cap_pct: f64,
    /// Run-off rates per outflow class, percent.
    pub outflow_rates: BTreeMap<String, f64>,
    /// Inflow rates per inflow class, percent.
    pub inflow_rates: BTreeMap<String, f64>,
}

impl Default for LcrParams {
    fn default() -> Self {
        Self {
            minimum: 100.0,
            internal_minimum: 110.0,
            early_warning: 120.0,
            significant_ccys: vec!["BHD".into(), "USD".into()],
            haircuts: map([("L1", 0.0), ("L2A", 15.0), ("L2B", 50.0)]),
            cap_l2_total_pct: 40.0,
            cap_l2b_pct: 15.0,
            inflow_cap_pct: 75.0,
            outflow_rates: map([
                ("RET_STABLE", 5.0),
                ("RET_LESS_STABLE", 10.0),
                ("SME_STABLE", 5.0),
                ("SME_LESS_STABLE", 10.0),
                ("OPERATIONAL", 25.0),
                ("NONFIN_WHOLESALE", 40.0),
                ("FI_WHOLESALE", 100.0),
                ("SECURED_L1", 0.0),
                ("SECURED_L2A", 15.0),
                ("SECURED_L2B", 50.0),
                ("SECURED_OTHER", 100.0),
                ("COMMIT_CREDIT_RETAIL", 5.0),
                ("COMMIT_CREDIT_CORP", 10.0),
                ("COMMIT_LIQ_CORP", 30.0),
                ("COMMIT_FI", 40.0),
                ("OTHER_CONTINGENT", 5.0),
            ]),
            inflow_rates: map([
                ("INFLOW_RETAIL_SME", 50.0),
                ("INFLOW_NONFIN", 50.0),
                ("INFLOW_FI", 100.0),
                ("INFLOW_SECURITIES", 100.0),
                ("INFLOW_OTHER", 0.0),
            ]),
        }
    }
}

/// Factors by residual maturity: under six months, six months to a year, a year or more.
pub type Factors3 = [f64; 3];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NsfrParams {
    pub minimum: f64,
    pub internal_minimum: f64,
    /// Available stable funding factors per liability class and bucket, percent.
    pub asf: BTreeMap<String, Factors3>,
    /// Required stable funding factors per asset class and bucket, percent.
    pub rsf: BTreeMap<String, Factors3>,
    /// Required stable funding per HQLA level, percent.
    pub hqla_rsf: BTreeMap<String, f64>,
    pub off_balance_rsf_pct: f64,
    pub encumbered_rsf_pct: f64,
}

impl Default for NsfrParams {
    fn default() -> Self {
        let f3 = |pairs: &[(&str, Factors3)]| pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect::<BTreeMap<_, _>>();
        Self {
            minimum: 100.0,
            internal_minimum: 105.0,
            asf: f3(&[
                ("CAPITAL", [100.0, 100.0, 100.0]),
                ("RETAIL_SME_STABLE", [95.0, 95.0, 95.0]),
                ("RETAIL_SME_LESS_STABLE", [90.0, 90.0, 90.0]),
                ("NONFIN_WHOLESALE", [50.0, 50.0, 100.0]),
                ("OPERATIONAL", [50.0, 50.0, 100.0]),
                ("FI", [0.0, 50.0, 100.0]),
                ("OTHER", [0.0, 50.0, 100.0]),
            ]),
            rsf: f3(&[
                ("CASH_CB", [0.0, 0.0, 0.0]),
                ("LOANS_FI", [15.0, 50.0, 100.0]),
                ("LOANS_RW35", [50.0, 50.0, 65.0]),
                ("LOANS_OTHER", [50.0, 50.0, 85.0]),
                ("OTHER", [100.0, 100.0, 100.0]),
            ]),
            hqla_rsf: map([("L1", 5.0), ("L2A", 15.0), ("L2B", 50.0)]),
            off_balance_rsf_pct: 5.0,
            encumbered_rsf_pct: 100.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GapParams {
    /// Behavioural life in months per non-maturity product.
    pub nmd_behavioural_months: BTreeMap<String, f64>,
    /// Cumulative gap limits per bucket as a percent of total assets.
    pub cumulative_gap_limit_pct_assets: BTreeMap<String, f64>,
    pub eve_limit_pct_tier1: f64,
    pub nii_sensitivity_limit_pct: f64,
}

impl Default for GapParams {
    fn default() -> Self {
        Self {
            nmd_behavioural_months: map([("CURRENT_ACCOUNT", 36.0), ("SAVINGS", 48.0)]),
            cumulative_gap_limit_pct_assets: map([("1M", 15.0), ("3M", 20.0), ("6M", 25.0), ("1Y", 30.0)]),
            eve_limit_pct_tier1: 15.0,
            nii_sensitivity_limit_pct: 10.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RateShock {
    pub parallel: f64,
    pub short: f64,
    pub long: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LiquidityStress {
    pub outflow_multiplier: f64,
    pub extra_hqla_haircut_pct: f64,
    pub inflow_multiplier: f64,
}

impl Default for LiquidityStress {
    fn default() -> Self {
        Self { outflow_multiplier: 1.5, extra_hqla_haircut_pct: 5.0, inflow_multiplier: 0.5 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StressParams {
    /// Basel standardised shocks in basis points per currency.
    pub shocks: BTreeMap<String, RateShock>,
    /// Post-shock floor at the overnight point, basis points (negative).
    pub floor_bp: f64,
    /// The floor rises by this many basis points per year of tenor until it reaches zero.
    pub floor_slope_bp_per_year: f64,
    pub liquidity: LiquidityStress,
    /// FX shock per currency, percent.
    pub fx_shock_pct: BTreeMap<String, f64>,
}

impl Default for StressParams {
    fn default() -> Self {
        let s = |p, sh, l| RateShock { parallel: p, short: sh, long: l };
        Self {
            shocks: [
                ("USD", s(200.0, 300.0, 150.0)),
                ("EUR", s(200.0, 250.0, 100.0)),
                ("GBP", s(250.0, 300.0, 150.0)),
                ("SAR", s(200.0, 300.0, 150.0)),
                ("AED", s(200.0, 300.0, 150.0)),
                ("BHD", s(200.0, 300.0, 150.0)),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
            floor_bp: -100.0,
            floor_slope_bp_per_year: 5.0,
            liquidity: LiquidityStress::default(),
            fx_shock_pct: map([("EUR", 15.0), ("GBP", 15.0)]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NiiParams {
    pub horizon_months: u32,
    pub constant_balance_sheet: bool,
    /// Pass-through for administered products not listed in `betas`, as a fraction.
    pub default_beta: f64,
    /// How much of a short-rate move each administered product passes on, as a fraction. Keyed by
    /// product (`SAVINGS`), or by product and segment (`CURRENT_ACCOUNT.RETAIL`), which wins.
    /// An ALM assumption: it replaces the beta the source data carries.
    pub betas: BTreeMap<String, f64>,
}

impl Default for NiiParams {
    fn default() -> Self {
        Self {
            horizon_months: 12,
            constant_balance_sheet: true,
            default_beta: 0.5,
            betas: map([
                ("CURRENT_ACCOUNT", 0.2),
                ("CURRENT_ACCOUNT.RETAIL", 0.0),
                ("CURRENT_ACCOUNT.SME", 0.0),
                ("SAVINGS", 0.4),
                ("OPERATIONAL_DEPOSIT", 0.3),
                ("OVERDRAFT", 0.8),
            ]),
        }
    }
}

impl NiiParams {
    /// The beta an administered cohort passes on: product and segment, else product, else the default.
    pub fn beta_for(&self, product: &str, segment: &str) -> f64 {
        self.betas.get(&format!("{product}.{segment}")).or_else(|| self.betas.get(product)).copied().unwrap_or(self.default_beta)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FundingParams {
    pub top1_limit_pct: f64,
    pub top10_limit_pct: f64,
    pub wholesale_share_limit_pct: f64,
}

impl Default for FundingParams {
    fn default() -> Self {
        Self { top1_limit_pct: 10.0, top10_limit_pct: 25.0, wholesale_share_limit_pct: 35.0 }
    }
}

/// The liquidity maturity ladder's behavioural assumptions and the bank's cumulative-gap limits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LiquidityGapParams {
    /// Share of non-maturity deposits that stays (spread over the behavioural life); the rest is
    /// treated as leaving overnight. Per segment, percent.
    pub stable_share_pct: BTreeMap<String, f64>,
    /// Share of undrawn commitments assumed drawn within a month, percent.
    pub commitment_draw_pct: f64,
    /// The cumulative gap may not fall below minus this share of total assets, per bucket, percent.
    pub cumulative_floor_pct_assets: BTreeMap<String, f64>,
}

impl Default for LiquidityGapParams {
    fn default() -> Self {
        Self {
            stable_share_pct: map([("RETAIL", 85.0), ("SME", 70.0), ("CORPORATE", 60.0), ("FI", 0.0)]),
            commitment_draw_pct: 5.0,
            cumulative_floor_pct_assets: map([("ON", 10.0), ("1W", 12.0), ("1M", 15.0), ("3M", 20.0)]),
        }
    }
}

/// An early-warning indicator's thresholds. `rising` means higher values are worse.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EwiThreshold {
    pub amber: f64,
    pub red: f64,
}

/// Thresholds for the early-warning indicators and the contingency funding plan's actions per stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EwiParams {
    pub thresholds: BTreeMap<String, EwiThreshold>,
    pub actions: BTreeMap<String, Vec<String>>,
}

impl Default for EwiParams {
    fn default() -> Self {
        let t = |a: f64, r: f64| EwiThreshold { amber: a, red: r };
        Self {
            thresholds: [
                ("deposits_5d", t(-2.0, -5.0)),
                ("retail_5d", t(-2.0, -4.0)),
                ("wholesale_30d", t(8.0, 12.0)),
                ("top10", t(22.5, 25.0)),
                ("lcr_5d", t(-15.0, -30.0)),
                ("bhibor_5d", t(25.0, 50.0)),
                ("peg_spread_5d", t(25.0, 50.0)),
                ("survival", t(60.0, 30.0)),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
            actions: [
                ("NORMAL", vec!["Daily monitoring; indicators reported to ALCO monthly."]),
                (
                    "ALERT",
                    vec![
                        "Treasurer and CRO informed the same day; indicators reviewed daily by ALCO members.",
                        "Raise term funding and lengthen interbank placements; avoid new long-dated lending in the affected currency.",
                        "Confirm the repo capacity of the liquid securities with the counterparties and the CBB.",
                    ],
                ),
                (
                    "CRISIS",
                    vec![
                        "Emergency ALCO convened; the contingency funding plan's crisis team takes over.",
                        "Draw on the repo capacity and the CBB facilities; sell level 1 securities if needed.",
                        "Communicate with large depositors and the CBB as the plan sets out; stop discretionary lending.",
                    ],
                ),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.into_iter().map(String::from).collect()))
            .collect(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// API shapes
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlmParamsVersion {
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<String>,
    pub note: Option<String>,
    pub is_active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlmParamsResponse {
    pub active: AlmParamsVersion,
    pub params: AlmParams,
    /// Most recent versions first, the active one included.
    pub history: Vec<AlmParamsVersion>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveParamsRequest {
    pub params: AlmParams,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlmSnapshot {
    pub as_of: NaiveDate,
    pub business_date: NaiveDate,
    pub generated_at: DateTime<Utc>,
    pub positions: i64,
}

/// What every calculator takes: a snapshot (the newest complete one when omitted) and either the
/// saved bank parameters or an unsaved what-if set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AlmRequest {
    pub as_of: Option<NaiveDate>,
    pub params: Option<AlmParams>,
    /// Rate scenario for the NII and EVE tools.
    pub scenario: Option<RateScenario>,
    /// Basis points for the custom parallel scenario.
    pub custom_shock_bp: Option<f64>,
    /// Attribution: against the prior month or the budget.
    pub compare_to: Option<CompareTo>,
    /// Attribution: the later period (first day of the month); defaults to the latest.
    pub period: Option<NaiveDate>,
}

/// What each lever applies to in the snapshot, in the reporting currency, keyed by the
/// parameter's path (`lcr.outflow_rates.FI_WHOLESALE`, `nsfr.asf.FI.1`, `nii.betas.SAVINGS`). Zero
/// means the lever has nothing to act on in this snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlmCoverage {
    pub as_of: NaiveDate,
    pub reporting_ccy: String,
    pub amounts: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LcrLine {
    pub class: String,
    pub amount: f64,
    /// Percent.
    pub rate: f64,
    pub weighted: f64,
}

/// The LCR of one currency, or of the whole book in the reporting currency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LcrFigures {
    pub ccy: String,
    /// Level 1 after haircut (none by default).
    pub hqla_l1: f64,
    /// Level 2A after haircut.
    pub hqla_l2a: f64,
    /// Level 2B after haircut.
    pub hqla_l2b: f64,
    pub adjustment_l2b_cap: f64,
    pub adjustment_l2_cap: f64,
    pub stock: f64,
    pub outflows: Vec<LcrLine>,
    pub total_outflows: f64,
    pub inflows: Vec<LcrLine>,
    pub total_inflows: f64,
    /// Inflows after the cap.
    pub capped_inflows: f64,
    pub net_outflows: f64,
    /// Percent; `None` when there are no net outflows.
    pub ratio: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LcrResult {
    pub as_of: NaiveDate,
    /// The saved parameter version used, or `None` for an unsaved what-if.
    pub params_version: Option<i32>,
    pub reporting_ccy: String,
    pub minimum: f64,
    pub internal_minimum: f64,
    pub early_warning: f64,
    pub total: LcrFigures,
    pub by_ccy: Vec<LcrFigures>,
}

// ---------------------------------------------------------------------------------------------
// Market data and scenarios
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub tenor: String,
    pub tenor_days: i32,
    /// Percent.
    pub rate: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateCurve {
    pub ccy: String,
    pub index_code: String,
    pub as_of: NaiveDate,
    /// Sorted by tenor.
    pub points: Vec<CurvePoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FxSpot {
    pub pair: String,
    pub as_of: NaiveDate,
    pub mid: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketResponse {
    pub curves: Vec<RateCurve>,
    pub spots: Vec<FxSpot>,
}

text_enum! {
    /// The Basel standardised interest rate scenarios, plus a custom parallel shock.
    RateScenario {
        Base => "BASE",
        ParallelUp => "PARALLEL_UP",
        ParallelDown => "PARALLEL_DOWN",
        Steepener => "STEEPENER",
        Flattener => "FLATTENER",
        ShortUp => "SHORT_UP",
        ShortDown => "SHORT_DOWN",
        Custom => "CUSTOM",
    }
}

impl Default for RateScenario {
    fn default() -> Self {
        RateScenario::Base
    }
}

impl RateScenario {
    pub const BASEL: [RateScenario; 6] = [
        RateScenario::ParallelUp,
        RateScenario::ParallelDown,
        RateScenario::Steepener,
        RateScenario::Flattener,
        RateScenario::ShortUp,
        RateScenario::ShortDown,
    ];

    pub fn label(self) -> &'static str {
        match self {
            RateScenario::Base => "Base",
            RateScenario::ParallelUp => "Parallel up",
            RateScenario::ParallelDown => "Parallel down",
            RateScenario::Steepener => "Steepener",
            RateScenario::Flattener => "Flattener",
            RateScenario::ShortUp => "Short rates up",
            RateScenario::ShortDown => "Short rates down",
            RateScenario::Custom => "Custom parallel",
        }
    }
}

text_enum! {
    CompareTo {
        Prior => "PRIOR",
        Budget => "BUDGET",
    }
}

text_enum! {
    PeriodKind {
        Actual => "ACTUAL",
        Budget => "BUDGET",
    }
}

/// One month of net interest income for a book, side and currency: the signed contribution to
/// NII (assets positive, liabilities negative) and the average balance, in the currency's units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeriodLine {
    pub period: NaiveDate,
    pub book: String,
    pub side: BsSide,
    pub ccy: String,
    pub kind: PeriodKind,
    pub nii: f64,
    pub avg_balance: f64,
}

// ---------------------------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NsfrLine {
    pub class: String,
    pub bucket: String,
    pub amount: f64,
    pub factor: f64,
    pub weighted: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NsfrResult {
    pub as_of: NaiveDate,
    pub params_version: Option<i32>,
    pub reporting_ccy: String,
    pub minimum: f64,
    pub internal_minimum: f64,
    pub asf_lines: Vec<NsfrLine>,
    pub rsf_lines: Vec<NsfrLine>,
    pub asf: f64,
    pub rsf: f64,
    /// Percent.
    pub ratio: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GapBucket {
    pub label: String,
    pub assets: f64,
    pub liabilities: f64,
    pub gap: f64,
    pub cumulative: f64,
    pub cumulative_pct_assets: f64,
    pub limit_pct: Option<f64>,
    pub breach: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GapCurrency {
    pub ccy: String,
    pub buckets: Vec<GapBucket>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GapResult {
    pub as_of: NaiveDate,
    pub params_version: Option<i32>,
    pub reporting_ccy: String,
    pub total_assets: f64,
    pub buckets: Vec<GapBucket>,
    pub by_ccy: Vec<GapCurrency>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NiiMonth {
    pub period: NaiveDate,
    pub income: f64,
    pub expense: f64,
    pub nii: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NiiBreakdown {
    pub key: String,
    pub nii: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NiiResult {
    pub as_of: NaiveDate,
    pub params_version: Option<i32>,
    pub reporting_ccy: String,
    pub scenario: RateScenario,
    pub horizon_months: u32,
    pub months: Vec<NiiMonth>,
    pub income: f64,
    pub expense: f64,
    pub nii: f64,
    pub earning_assets: f64,
    /// Annualised NII over earning assets, percent.
    pub nim: Option<f64>,
    pub by_book: Vec<NiiBreakdown>,
    pub by_ccy: Vec<NiiBreakdown>,
    /// The base scenario's NII when this is a shocked run.
    pub base_nii: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EveCurrency {
    pub ccy: String,
    pub base: f64,
    pub shocked: f64,
    pub delta: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EveResult {
    pub as_of: NaiveDate,
    pub params_version: Option<i32>,
    pub reporting_ccy: String,
    pub scenario: RateScenario,
    pub base: f64,
    pub shocked: f64,
    /// Shocked minus base, summed over currencies (negative is a loss).
    pub delta: f64,
    /// Basel aggregation: losses less half the gains, floored at zero.
    pub basel_loss: f64,
    pub by_ccy: Vec<EveCurrency>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StressScenarioRow {
    pub scenario: RateScenario,
    pub label: String,
    pub nii: f64,
    pub nii_delta: f64,
    pub nii_delta_pct: Option<f64>,
    pub eve_delta: f64,
    pub eve_loss: f64,
    pub eve_loss_pct_tier1: Option<f64>,
    pub nii_breach: bool,
    pub eve_breach: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurvivalPoint {
    pub day: u32,
    pub cumulative_net_outflow: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiquidityStressResult {
    pub lcr_base: Option<f64>,
    pub lcr_stressed: Option<f64>,
    pub stock_stressed: f64,
    pub net_outflows_stressed: f64,
    /// First day the cumulative stressed net outflow exceeds the stock; `None` means the stock
    /// lasts the whole 90-day horizon.
    pub survival_days: Option<u32>,
    pub horizon: Vec<SurvivalPoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FxStressRow {
    pub ccy: String,
    pub shock_pct: f64,
    /// Net assets in the currency, in the reporting currency.
    pub exposure: f64,
    /// Revaluation if the currency weakens by the shock.
    pub pnl: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StressResult {
    pub as_of: NaiveDate,
    pub params_version: Option<i32>,
    pub reporting_ccy: String,
    pub base_nii: f64,
    pub base_eve: f64,
    pub tier1_capital: f64,
    pub nii_limit_pct: f64,
    pub eve_limit_pct_tier1: f64,
    pub rate_scenarios: Vec<StressScenarioRow>,
    pub worst_eve: Option<RateScenario>,
    pub liquidity: LiquidityStressResult,
    pub fx: Vec<FxStressRow>,
    /// The test the annual report discloses: every curve up and down 200 bp.
    #[serde(default)]
    pub uniform_200: Vec<StressScenarioRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttributionLine {
    pub book: String,
    pub side: BsSide,
    pub ccy: String,
    pub nii_a: f64,
    pub nii_b: f64,
    pub volume_a: f64,
    pub volume_b: f64,
    /// Percent.
    pub rate_a: f64,
    pub rate_b: f64,
    pub days_effect: f64,
    pub volume_effect: f64,
    pub rate_effect: f64,
    pub residual: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttributionStep {
    pub label: String,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttributionResult {
    pub period_a: NaiveDate,
    pub period_b: NaiveDate,
    pub compare_to: CompareTo,
    pub reporting_ccy: String,
    pub nii_a: f64,
    pub nii_b: f64,
    pub delta: f64,
    pub days_effect: f64,
    pub pure_volume_effect: f64,
    pub mix_effect: f64,
    pub rate_effect: f64,
    pub residual: f64,
    /// Waterfall: start, the effects, end.
    pub steps: Vec<AttributionStep>,
    pub lines: Vec<AttributionLine>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FundingGroup {
    pub name: String,
    pub segment: String,
    pub balance: f64,
    pub share_pct: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FundingResult {
    pub as_of: NaiveDate,
    pub params_version: Option<i32>,
    pub reporting_ccy: String,
    pub total_deposits: f64,
    pub total_funding: f64,
    pub wholesale_funding: f64,
    pub top1_pct: f64,
    pub top10_pct: f64,
    pub wholesale_pct: f64,
    pub top1_limit_pct: f64,
    pub top10_limit_pct: f64,
    pub wholesale_limit_pct: f64,
    pub breaches: Vec<String>,
    pub groups: Vec<FundingGroup>,
    /// Deposits by segment.
    pub by_segment: Vec<NiiBreakdown>,
}

/// Interest rate differential: a forward from two money-market rates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrdQuote {
    pub tenor: String,
    pub days: i64,
    pub base_rate: f64,
    pub quote_rate: f64,
    pub forward: f64,
    /// In pips of the quote currency.
    pub points: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrendPoint {
    pub as_of: NaiveDate,
    pub value: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrendResponse {
    pub metric: String,
    pub points: Vec<TrendPoint>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrendQuery {
    pub metric: Option<String>,
    pub days: Option<i64>,
}

/// A metric against the bank's limit. `floor` means the value must stay above the limit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitRow {
    pub metric: String,
    pub value: Option<f64>,
    pub unit: String,
    pub limit: f64,
    pub trigger: Option<f64>,
    pub floor: bool,
    pub breach: bool,
    pub warn: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NiiPeriodRow {
    pub period: NaiveDate,
    pub actual: f64,
    pub budget: f64,
}

/// The ALM block of the overview: headline ratios, income against budget, limits and trends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlmOverview {
    pub as_of: NaiveDate,
    pub params_version: i32,
    pub reporting_ccy: String,
    pub lcr: Option<f64>,
    pub nsfr: Option<f64>,
    pub nii_12m: f64,
    pub nim: Option<f64>,
    pub nii_mtd: f64,
    pub budget_mtd: f64,
    pub nii_ytd: f64,
    pub budget_ytd: f64,
    pub gap_1y_pct: f64,
    pub worst_eve_scenario: Option<RateScenario>,
    pub worst_eve_loss_pct_tier1: Option<f64>,
    pub top10_pct: f64,
    pub wholesale_pct: f64,
    pub limits: Vec<LimitRow>,
    pub lcr_trend: Vec<TrendPoint>,
    pub nii_months: Vec<NiiPeriodRow>,
    pub gap_buckets: Vec<GapBucket>,
    /// The same headline figures on the previous snapshot, for "since yesterday".
    #[serde(default)]
    pub previous: Option<OverviewPrevious>,
    #[serde(default)]
    pub ratios: Option<Ratios>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverviewPrevious {
    pub as_of: NaiveDate,
    pub lcr: Option<f64>,
    pub nsfr: Option<f64>,
    pub worst_eve_loss_pct_tier1: Option<f64>,
    pub top10_pct: f64,
    pub nii_12m: f64,
}

/// Balance-sheet health ratios, percent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ratios {
    pub loan_to_deposit: Option<f64>,
    pub liquid_assets_to_assets: Option<f64>,
    /// Balance-weighted rate on interest-bearing liabilities.
    pub cost_of_funds: Option<f64>,
    /// Balance-weighted rate on earning assets.
    pub asset_yield: Option<f64>,
    pub net_spread: Option<f64>,
    pub wholesale_funding: f64,
    pub equity_to_assets: Option<f64>,
}

// ---------------------------------------------------------------------------------------------
// Liquidity maturity ladder
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiquidityBucket {
    pub label: String,
    pub inflows: f64,
    pub outflows: f64,
    pub gap: f64,
    pub cumulative: f64,
    pub cumulative_pct_assets: f64,
    /// The floor the cumulative gap may not fall below, percent of assets (negative), if set.
    pub floor_pct: Option<f64>,
    pub breach: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiquidityCcy {
    pub ccy: String,
    /// Gap per bucket and its running total, in the reporting currency.
    pub gaps: Vec<f64>,
    pub cumulative: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiquidityLadder {
    pub buckets: Vec<LiquidityBucket>,
    pub by_ccy: Vec<LiquidityCcy>,
    /// The first bucket in which the cumulative gap turns negative, if any.
    pub first_negative: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiquidityGapResult {
    pub as_of: NaiveDate,
    pub params_version: Option<i32>,
    pub reporting_ccy: String,
    pub total_assets: f64,
    pub contractual: LiquidityLadder,
    pub behavioural: LiquidityLadder,
    pub breaches: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// Early-warning indicators and the contingency funding plan
// ---------------------------------------------------------------------------------------------

text_enum! {
    /// Where an indicator stands, or the contingency plan's stage.
    EwiStatus {
        Green => "GREEN",
        Amber => "AMBER",
        Red => "RED",
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EwiPoint {
    pub as_of: NaiveDate,
    pub value: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EwiIndicator {
    pub key: String,
    pub label: String,
    pub what: String,
    pub unit: String,
    pub value: Option<f64>,
    pub amber: f64,
    pub red: f64,
    /// Whether a higher value is worse.
    pub rising_is_worse: bool,
    pub status: EwiStatus,
    pub series: Vec<EwiPoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EwiResult {
    pub as_of: NaiveDate,
    pub params_version: Option<i32>,
    /// NORMAL, ALERT or CRISIS.
    pub stage: String,
    pub indicators: Vec<EwiIndicator>,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PositionsSummaryRow {
    pub book: String,
    pub side: BsSide,
    pub ccy: String,
    pub rate_type: RateType,
    pub balance: f64,
    /// Balance-weighted, percent.
    pub avg_rate: f64,
    pub cohorts: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AsOfQuery {
    pub as_of: Option<NaiveDate>,
}
