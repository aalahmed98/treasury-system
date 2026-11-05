//! The bond portfolio of the PMU desk: what it holds (books, bonds, purchase lots, the swaps that
//! hedge them, the repos that fund them) and what the `pmu` crate makes of it. Amounts in results
//! are in BHD unless a field says local; rates are percent; DV01 is the P&L of a one basis point
//! rise, in BHD (negative for a long bond).

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

text_enum! {
    Accounting {
        Afs => "AFS",
        Htm => "HTM",
        Fvtpl => "FVTPL",
    }
}

text_enum! {
    /// Bond and swap-leg day counts.
    BondDayCount {
        ActAct => "ACT/ACT",
        Thirty360 => "30/360",
        Act360 => "ACT/360",
        Act365 => "ACT/365",
    }
}

text_enum! {
    BondRateType {
        Fixed => "FIXED",
        Float => "FLOAT",
    }
}

// ---------------------------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PmuBook {
    pub code: String,
    pub name: String,
    pub accounting: Accounting,
    pub ccy: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bond {
    pub isin: String,
    pub name: String,
    pub issuer: String,
    pub issuer_type: String,
    pub ccy: String,
    /// Percent a year; for a floater, the spread over the index is `float_spread`.
    pub coupon: f64,
    pub coupon_freq: u32,
    pub day_count: BondDayCount,
    pub issue_date: NaiveDate,
    pub maturity: NaiveDate,
    pub rate_type: BondRateType,
    pub float_index: Option<String>,
    pub float_spread: f64,
    pub hqla_level: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lot {
    pub id: Uuid,
    pub book: String,
    pub isin: String,
    pub trade_date: NaiveDate,
    pub settle_date: NaiveDate,
    /// In the bond's currency.
    pub face: f64,
    /// Per 100 of face.
    pub clean_price: f64,
    pub accrued_paid: f64,
    pub kondor_id: Option<String>,
    /// Effective interest rate fixed at purchase, percent a year (annual compounding, ACT/365).
    pub eir: f64,
    /// The FTP rate locked on the trade date; `None` when no curve was published for the currency.
    pub ftp_rate: Option<f64>,
    pub ftp_publication_id: Option<i32>,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwapUnwind {
    pub date: NaiveDate,
    pub notional: f64,
    /// Cash that settled the unwind, in the swap's currency; positive received.
    pub settlement: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Swap {
    pub id: Uuid,
    pub book: String,
    pub kondor_id: Option<String>,
    pub ccy: String,
    pub notional: f64,
    pub start: NaiveDate,
    pub maturity: NaiveDate,
    /// Pays fixed and receives floating: the hedge of a fixed-rate bond.
    pub pay_fixed: bool,
    pub fixed_rate: f64,
    pub fixed_freq: u32,
    pub fixed_day_count: BondDayCount,
    pub float_index: String,
    pub float_tenor_months: u32,
    pub float_spread: f64,
    pub status: String,
    /// (lot, notional assigned to it).
    pub hedges: Vec<(Uuid, f64)>,
    pub unwinds: Vec<SwapUnwind>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Repo {
    pub id: Uuid,
    pub counterparty: String,
    pub kondor_id: Option<String>,
    pub ccy: String,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub cash: f64,
    pub rate: f64,
    pub haircut_pct: f64,
    /// (lot, face pledged).
    pub collateral: Vec<(Uuid, f64)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoQuote {
    pub as_of: NaiveDate,
    pub counterparty: String,
    pub tenor: String,
    pub tenor_days: i32,
    pub spread_bp: f64,
    pub quoted_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------------------------

/// Income over a period, broken into its parts, in BHD.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Income {
    pub coupon: f64,
    pub amortisation: f64,
    /// Coupon plus amortisation: income at the effective interest rate.
    pub eir_income: f64,
    /// The FTP charge for funding the bond, as a positive cost.
    pub ftp: f64,
    /// Coupon + amortisation − FTP.
    pub carry: f64,
    /// Receive leg less pay leg of the swaps hedging the bond.
    pub irs: f64,
    /// Bond carry + IRS carry: the management net interest income.
    pub net_nii: f64,
    /// Interest paid on repos funding the bond, as a positive cost.
    pub repo: f64,
    /// Coupon + amortisation + IRS − repo interest.
    pub accounting_nii: f64,
}

impl Income {
    pub fn finish(mut self) -> Income {
        self.eir_income = self.coupon + self.amortisation;
        self.carry = self.eir_income - self.ftp;
        self.net_nii = self.carry + self.irs;
        self.accounting_nii = self.eir_income + self.irs - self.repo;
        self
    }
    pub fn add(&mut self, o: &Income) {
        self.coupon += o.coupon;
        self.amortisation += o.amortisation;
        self.ftp += o.ftp;
        self.irs += o.irs;
        self.repo += o.repo;
        *self = self.finish();
    }
}

text_enum! {
    RowLevel {
        Book => "BOOK",
        Bond => "BOND",
        Lot => "LOT",
    }
}

/// One row of the positions tree: a book, a bond within it, or a purchase lot of that bond.
/// Figures on book and bond rows add up their children; prices, yields and duration are weighted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PositionRow {
    pub level: RowLevel,
    /// `book`, `book|isin` or the lot id.
    pub key: String,
    pub parent: Option<String>,
    pub book: String,
    pub isin: Option<String>,
    pub label: String,
    pub issuer: Option<String>,
    pub ccy: Option<String>,
    pub kondor_id: Option<String>,
    pub trade_date: Option<NaiveDate>,
    pub maturity: Option<NaiveDate>,
    pub lots: u32,
    /// Local currency (`None` on book rows that mix currencies).
    pub face_local: Option<f64>,
    pub face_bhd: f64,
    /// Purchase price, weighted by face.
    pub price: f64,
    pub market_price: Option<f64>,
    pub market_value_bhd: f64,
    pub book_value_bhd: f64,
    pub mtm_bhd: f64,
    /// Effective interest rate at purchase, weighted by book value.
    pub eir: f64,
    pub ytm: Option<f64>,
    pub ftp_rate: Option<f64>,
    pub dv01_bhd: f64,
    pub swap_dv01_bhd: f64,
    pub mod_duration: f64,
    pub hedged_face_bhd: f64,
    pub pledged_face_bhd: f64,
    /// Share of the whole portfolio's face.
    pub share: f64,
    pub income: Income,
    /// Today's accrual after FTP.
    pub daily_carry_bhd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwapView {
    pub id: Uuid,
    pub book: String,
    pub kondor_id: Option<String>,
    pub ccy: String,
    pub notional: f64,
    pub closed_notional: f64,
    pub start: NaiveDate,
    pub maturity: NaiveDate,
    pub pay_fixed: bool,
    pub fixed_rate: f64,
    pub float_index: String,
    pub float_rate: f64,
    pub mtm_bhd: f64,
    pub dv01_bhd: f64,
    pub isins: Vec<String>,
}

/// A bond and the swaps hedging it, across books.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HedgePair {
    pub isin: String,
    pub name: String,
    pub ccy: String,
    pub face_bhd: f64,
    pub hedged_face_bhd: f64,
    pub bond_dv01: f64,
    pub irs_dv01: f64,
    pub net_dv01: f64,
    pub bond_mtm: f64,
    pub irs_mtm: f64,
    pub net_mtm: f64,
    /// Swap notional unwound before maturity, in the bond's currency.
    pub closed_notional: f64,
    pub swaps: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookFigure {
    pub book: String,
    pub name: String,
    pub accounting: Accounting,
    pub face_bhd: f64,
    pub market_value_bhd: f64,
    pub book_value_bhd: f64,
    pub mtm_bhd: f64,
    pub dv01_bhd: f64,
    pub net_dv01_bhd: f64,
    pub mod_duration: f64,
    pub yield_pct: f64,
    pub daily_carry_bhd: f64,
}

/// Face in a maturity bucket, split by whether a swap hedges it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HedgeBucket {
    pub label: String,
    pub hedged_bhd: f64,
    pub unhedged_bhd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoSummary {
    pub amount_bhd: f64,
    pub available_face_bhd: f64,
    pub pledged_face_bhd: f64,
    pub due_7d_bhd: f64,
    pub active: u32,
    pub avg_rate: f64,
    pub avg_haircut: f64,
    pub wtd_maturity_days: f64,
    pub receipts_12m_bhd: f64,
}

/// The six headline figures, today and on the previous business day.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Headline {
    pub portfolio_face_bhd: f64,
    pub market_value_bhd: f64,
    pub book_value_bhd: f64,
    pub net_mtm_bhd: f64,
    pub mtd_carry_bhd: f64,
    pub net_dv01_bhd: f64,
    pub mtd_nii_bhd: f64,
    pub portfolio_yield: f64,
    pub mod_duration: f64,
    pub lots: u32,
    /// Share of lots carrying a Kondor deal id, percent.
    pub kondor_pct: f64,
    pub daily_carry_bhd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonthIncome {
    /// First day of the month.
    pub month: NaiveDate,
    /// True for months up to the business date (the current month is part actual).
    pub actual: bool,
    pub income: Income,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Warning {
    pub key: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PmuOverview {
    pub as_of: NaiveDate,
    pub previous: Option<NaiveDate>,
    pub headline: Headline,
    pub headline_prev: Option<Headline>,
    pub by_book: Vec<BookFigure>,
    pub monthly: Vec<MonthIncome>,
    pub maturity_now: Vec<HedgeBucket>,
    pub maturity_year_end: Vec<HedgeBucket>,
    /// Bond rows, largest face first.
    pub concentration: Vec<PositionRow>,
    pub repo: RepoSummary,
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PmuPositions {
    pub as_of: NaiveDate,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub rows: Vec<PositionRow>,
    pub swaps: Vec<SwapView>,
    pub headline: Headline,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DayIncome {
    pub date: NaiveDate,
    pub income: Income,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamedIncome {
    pub key: String,
    pub label: String,
    pub income: Income,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PmuIncome {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub total: Income,
    pub daily: Vec<DayIncome>,
    pub by_bond: Vec<NamedIncome>,
    pub by_book: Vec<NamedIncome>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeriodIncome {
    pub label: String,
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// Covers less than the whole calendar year.
    pub partial: bool,
    pub income: Income,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookYear {
    pub book: String,
    pub actual: Income,
    pub forecast: Income,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PmuForecast {
    pub as_of: NaiveDate,
    pub reinvest: bool,
    pub full_year: Income,
    pub year_to_date: Income,
    pub rest_of_year: Income,
    pub monthly: Vec<MonthIncome>,
    /// The rest of this year, then each future calendar year.
    pub periods: Vec<PeriodIncome>,
    pub by_book: Vec<BookYear>,
    pub year_end_face_bhd: f64,
    pub year_end_hedged_face_bhd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dv01Bucket {
    pub label: String,
    pub bond: f64,
    pub irs: f64,
    pub net: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShockRow {
    pub shift_bp: f64,
    /// P&L per book, in the order of `PmuRisk::books`.
    pub by_book: Vec<f64>,
    pub total: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YieldRow {
    pub book: Option<String>,
    pub year: i32,
    pub face_bhd: f64,
    pub yield_all: f64,
    pub yield_unhedged: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CcyHedge {
    pub ccy: String,
    pub hedged_bhd: f64,
    pub unhedged_bhd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PmuRisk {
    pub as_of: NaiveDate,
    pub hedge_pairs: Vec<HedgePair>,
    /// Bond rows with nothing hedged.
    pub unhedged: Vec<PositionRow>,
    pub dv01_buckets: Vec<Dv01Bucket>,
    pub books: Vec<String>,
    pub shocks: Vec<ShockRow>,
    pub by_book: Vec<BookFigure>,
    /// Across books (book `None`) for the chart, then per book for the table.
    pub yield_by_year: Vec<YieldRow>,
    pub hedged_by_ccy: Vec<CcyHedge>,
    pub maturity_now: Vec<HedgeBucket>,
    pub maturity_year_end: Vec<HedgeBucket>,
    pub headline: Headline,
    pub year_end_face_bhd: f64,
    pub year_end_hedged_face_bhd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoView {
    pub id: Uuid,
    pub counterparty: String,
    pub kondor_id: Option<String>,
    pub ccy: String,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub cash_bhd: f64,
    pub rate: f64,
    pub haircut_pct: f64,
    pub days_left: i64,
    pub collateral_face_bhd: f64,
    pub isins: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonthFlow {
    pub month: NaiveDate,
    pub a: f64,
    pub b: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuoteView {
    pub counterparty: String,
    pub tenor: String,
    pub tenor_days: i32,
    pub spread_bp: f64,
    pub as_of: NaiveDate,
    pub age_days: i64,
    pub sofr: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollateralBook {
    pub book: String,
    pub pledged_bhd: f64,
    pub free_bhd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PmuFunding {
    pub as_of: NaiveDate,
    pub summary: RepoSummary,
    pub repos: Vec<RepoView>,
    pub by_remaining: Vec<NamedAmount>,
    /// a = funding raised (repos starting), b = repaid (repos ending), by month.
    pub funding_flows: Vec<MonthFlow>,
    pub collateral_by_book: Vec<CollateralBook>,
    pub quotes: Vec<QuoteView>,
    /// a = coupons, b = principal, by month, next twelve months.
    pub receipts: Vec<MonthFlow>,
    /// a = net swap settlements by month, next twelve months; b = unwind cash received in the
    /// past twelve months (kept apart from income).
    pub hedge_cash: Vec<MonthFlow>,
    pub by_book_year: Vec<BookYear>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamedAmount {
    pub label: String,
    pub amount: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CashFlowRow {
    pub date: NaiveDate,
    pub kind: String,
    pub amount_local: f64,
    pub amount_bhd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PmuBondDetail {
    pub bond: Bond,
    pub lots: Vec<PositionRow>,
    pub swaps: Vec<SwapView>,
    pub repos: Vec<RepoView>,
    pub cash_flows: Vec<CashFlowRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeriodQueryPmu {
    #[serde(default)]
    pub from: Option<NaiveDate>,
    #[serde(default)]
    pub to: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForecastQuery {
    #[serde(default)]
    pub reinvest: Option<bool>,
}
