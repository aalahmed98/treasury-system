//! A synthetic banking book for the ALM workbench: an NBB-scale balance sheet of loans, deposits,
//! securities and funding as cohorts, snapshotted daily for the recent past and at month-ends
//! before that, with rate curves, spots and depositor groups. Deterministic by seed; the daily
//! random walk is simulated once from the earliest snapshot to the business date so daily and
//! month-end snapshots agree. Real extracts from the core banking system replace this later; the
//! tables stay the same.

use std::collections::BTreeMap;

use chrono::{Datelike, Duration, NaiveDate, Utc};
use domain::calendar::Calendar;
use domain::{Amort, BsPosition, BsSide, DayCount, HqlaLevel, RateType};

use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, QueryBuilder};

use alm::schedule::add_months;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AlmCfg {
    pub enabled: bool,
    /// Total assets in the reporting currency (BHD).
    pub total_assets: f64,
    /// Daily snapshots before the business date.
    pub daily_history: u32,
    /// Month-end snapshots before that.
    pub month_ends: u32,
    /// Days of daily curve and spot history.
    pub rate_history_days: u32,
}

impl Default for AlmCfg {
    fn default() -> Self {
        Self { enabled: true, total_assets: 5.6e9, daily_history: 35, month_ends: 13, rate_history_days: 400 }
    }
}

pub const CCYS: [&str; 6] = ["BHD", "USD", "EUR", "GBP", "SAR", "AED"];
pub const TENORS: [(&str, i32); 11] =
    [("ON", 1), ("1W", 7), ("1M", 30), ("3M", 91), ("6M", 182), ("1Y", 365), ("2Y", 730), ("3Y", 1095), ("5Y", 1825), ("7Y", 2555), ("10Y", 3650)];

pub fn index_for(ccy: &str) -> &'static str {
    match ccy {
        "BHD" => "BHIBOR",
        "USD" => "SOFR",
        "EUR" => "EURIBOR",
        "GBP" => "SONIA",
        "SAR" => "SAIBOR",
        "AED" => "EIBOR",
        _ => "OTHER",
    }
}

/// One-year level in percent around which each curve wanders.
fn base_level(ccy: &str) -> f64 {
    match ccy {
        "BHD" => 5.9,
        "USD" => 4.3,
        "EUR" => 2.3,
        "GBP" => 4.1,
        "SAR" => 5.2,
        "AED" => 4.4,
        _ => 4.0,
    }
}

fn day_count(ccy: &str) -> DayCount {
    if ccy == "GBP" { DayCount::Act365 } else { DayCount::Act360 }
}

/// BHD per one unit of the currency on the anchor day; the pegged currencies stay there.
pub fn base_spot(ccy: &str) -> f64 {
    match ccy {
        "BHD" => 1.0,
        "USD" => 0.376,
        "EUR" => 0.408,
        "GBP" => 0.47752,
        "SAR" => 0.1003,
        "AED" => 0.1024,
        _ => 1.0,
    }
}

/// The currencies held to the US dollar (and so to the dinar): their spot never moves.
pub fn pegged(ccy: &str) -> bool {
    matches!(ccy, "BHD" | "USD" | "SAR" | "AED" | "QAR" | "OMR")
}

/// The day the synthetic spot path is anchored to its base level. A fixed date, so the market
/// generator and the deal generator see the same mid on any day without sharing state.
const SPOT_ANCHOR: (i32, u32, u32) = (2026, 1, 1);
/// Daily volatility of a floating currency against the dinar.
const SPOT_VOL: f64 = 0.003;

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn unit(h: u64) -> f64 {
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// The day's standard normal shock for a currency: a pure function of seed, currency and date.
fn spot_shock(seed: u64, ccy: &str, day: NaiveDate) -> f64 {
    let h = splitmix(seed ^ hash(ccy) ^ (day.num_days_from_ce() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let u1 = unit(h).max(1e-12);
    let u2 = unit(splitmix(h));
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

/// BHD per one unit of `ccy` at the close of `day`: the base level on the anchor day, walked by
/// one shock a day forwards or backwards from it. Pegged currencies stay at their base.
pub fn fx_mid(seed: u64, ccy: &str, day: NaiveDate) -> f64 {
    let base = base_spot(ccy);
    if pegged(ccy) {
        return base;
    }
    let anchor = NaiveDate::from_ymd_opt(SPOT_ANCHOR.0, SPOT_ANCHOR.1, SPOT_ANCHOR.2).expect("anchor date");
    let mut level = base;
    if day >= anchor {
        let mut d = anchor + Duration::days(1);
        while d <= day {
            level *= 1.0 + SPOT_VOL * spot_shock(seed, ccy, d);
            d += Duration::days(1);
        }
    } else {
        let mut d = anchor;
        while d > day {
            level /= 1.0 + SPOT_VOL * spot_shock(seed, ccy, d);
            d -= Duration::days(1);
        }
    }
    (level * 1e6).round() / 1e6
}

/// Units of `quote` per one unit of `base` on `day`, crossed through the dinar.
pub fn fx_cross(seed: u64, base: &str, quote: &str, day: NaiveDate) -> f64 {
    fx_mid(seed, base, day) / fx_mid(seed, quote, day)
}

/// The rate level a currency's money market sits around, percent; for synthetic forward points.
pub fn money_rate(ccy: &str) -> f64 {
    base_level(ccy)
}

pub(crate) fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

pub(crate) fn normal(rng: &mut StdRng) -> f64 {
    let u1: f64 = rng.random_range(1e-12..1.0);
    let u2: f64 = rng.random_range(0.0..1.0);
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

// ---------------------------------------------------------------------------------------------
// Market data
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct RatePoint {
    pub as_of: NaiveDate,
    pub ccy: String,
    pub index_code: String,
    pub tenor: String,
    pub tenor_days: i32,
    pub rate: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpotPoint {
    pub as_of: NaiveDate,
    pub pair: String,
    pub mid: f64,
}

#[derive(Debug, Clone, Default)]
pub struct Market {
    pub rates: Vec<RatePoint>,
    pub spots: Vec<SpotPoint>,
    /// (ccy, as_of) -> curve by tenor days, for lookups while building the book.
    curves: BTreeMap<(String, NaiveDate), Vec<(i32, f64)>>,
    spot_by: BTreeMap<(String, NaiveDate), f64>,
    first_day: Option<NaiveDate>,
    last_day: Option<NaiveDate>,
}

impl Market {
    /// Daily closes for `days` days ending on `end`.
    pub fn generate(seed: u64, end: NaiveDate, days: u32) -> Market {
        let mut m = Market::default();
        let start = end - Duration::days(days.max(1) as i64 - 1);
        m.first_day = Some(start);
        m.last_day = Some(end);
        for ccy in CCYS {
            let mut rng = StdRng::seed_from_u64(seed ^ hash(ccy) ^ 0xa1a1_0000);
            let base = base_level(ccy);
            let mut level = base;
            let mut slope: f64 = 0.8;
            let mut spot = fx_mid(seed, ccy, start);
            let mut d = start;
            while d <= end {
                level += 0.02 * (base - level) + 0.025 * normal(&mut rng);
                slope += 0.05 * (0.8 - slope) + 0.01 * normal(&mut rng);
                let mut curve = Vec::with_capacity(TENORS.len());
                for (tenor, td) in TENORS {
                    let t = td as f64 / 3650.0;
                    let rate = (level - 0.25 + slope * t.sqrt()).max(0.05);
                    let rate = (rate * 10_000.0).round() / 10_000.0;
                    curve.push((td, rate));
                    m.rates.push(RatePoint { as_of: d, ccy: ccy.into(), index_code: index_for(ccy).into(), tenor: tenor.into(), tenor_days: td, rate });
                }
                m.curves.insert((ccy.to_string(), d), curve);
                if d > start && !pegged(ccy) {
                    spot *= 1.0 + SPOT_VOL * spot_shock(seed, ccy, d);
                }
                if ccy != "BHD" {
                    let mid = (spot * 1e6).round() / 1e6;
                    m.spots.push(SpotPoint { as_of: d, pair: format!("{ccy}/BHD"), mid });
                    m.spot_by.insert((ccy.to_string(), d), mid);
                }
                d += Duration::days(1);
            }
        }
        m
    }

    /// The curve rate for a tenor on a day, interpolated between tenors and clamped to the
    /// history on either side (an origination twenty years ago reads the first day).
    pub fn rate(&self, ccy: &str, on: NaiveDate, tenor_days: i32) -> f64 {
        let (Some(first), Some(last)) = (self.first_day, self.last_day) else { return base_level(ccy) };
        let on = on.clamp(first, last);
        let Some(curve) = self.curves.get(&(ccy.to_string(), on)) else { return base_level(ccy) };
        let td = tenor_days.max(1);
        if td <= curve[0].0 {
            return curve[0].1;
        }
        for w in curve.windows(2) {
            let ((a, ra), (b, rb)) = (w[0], w[1]);
            if td <= b {
                return ra + (rb - ra) * (td - a) as f64 / (b - a) as f64;
            }
        }
        curve.last().map(|c| c.1).unwrap_or(base_level(ccy))
    }

    /// The first and last day of the history.
    pub fn span(&self) -> Option<(NaiveDate, NaiveDate)> {
        self.first_day.zip(self.last_day)
    }

    pub fn spot(&self, ccy: &str, on: NaiveDate) -> f64 {
        if ccy == "BHD" {
            return 1.0;
        }
        let (Some(first), Some(last)) = (self.first_day, self.last_day) else { return base_spot(ccy) };
        self.spot_by.get(&(ccy.to_string(), on.clamp(first, last))).copied().unwrap_or_else(|| base_spot(ccy))
    }
}

// ---------------------------------------------------------------------------------------------
// The book: cohort templates expanded by currency, walked day by day.
// ---------------------------------------------------------------------------------------------

type Mix = &'static [(&'static str, f64)];
const DOMESTIC: Mix = &[("BHD", 1.0)];
const MIXED: Mix = &[("BHD", 0.6), ("USD", 0.3), ("SAR", 0.05), ("AED", 0.05)];
const INTL: Mix = &[("USD", 0.6), ("EUR", 0.2), ("GBP", 0.1), ("SAR", 0.05), ("AED", 0.05)];
const USD: Mix = &[("USD", 1.0)];
const GCC: Mix = &[("USD", 0.6), ("SAR", 0.4)];

struct Tpl {
    pid: &'static str,
    side: BsSide,
    book: &'static str,
    segment: &'static str,
    product: &'static str,
    /// Share of total assets (assets) or of total funding (liabilities), before normalisation.
    share: f64,
    mix: Mix,
    rate_type: RateType,
    /// Spread over the index (FLOAT), over the curve at origination (FIXED), or the administered
    /// rate itself (ADMIN), in percent.
    spread: f64,
    reset_months: Option<i16>,
    pay_months: Option<i16>,
    amort: Amort,
    term_months: Option<i16>,
    hqla: Option<HqlaLevel>,
    lcr_class: Option<&'static str>,
    nsfr_class: Option<&'static str>,
    beta: f64,
    behavioural_months: Option<i16>,
}

const fn t(pid: &'static str, side: BsSide, book: &'static str, segment: &'static str, product: &'static str, share: f64, mix: Mix) -> Tpl {
    Tpl {
        pid,
        side,
        book,
        segment,
        product,
        share,
        mix,
        rate_type: RateType::NotSensitive,
        spread: 0.0,
        reset_months: None,
        pay_months: None,
        amort: Amort::Bullet,
        term_months: None,
        hqla: None,
        lcr_class: None,
        nsfr_class: None,
        beta: 0.0,
        behavioural_months: None,
    }
}

impl Tpl {
    const fn fixed(mut self, premium: f64, term: i16, pay: Option<i16>, amort: Amort) -> Self {
        self.rate_type = RateType::Fixed;
        self.spread = premium;
        self.term_months = Some(term);
        self.pay_months = pay;
        self.amort = amort;
        self
    }
    const fn float(mut self, spread: f64, reset: i16, term: i16, pay: Option<i16>, amort: Amort) -> Self {
        self.rate_type = RateType::Float;
        self.spread = spread;
        self.reset_months = Some(reset);
        self.term_months = Some(term);
        self.pay_months = pay;
        self.amort = amort;
        self
    }
    const fn admin(mut self, rate: f64, beta: f64, behavioural: i16) -> Self {
        self.rate_type = RateType::Admin;
        self.spread = rate;
        self.beta = beta;
        self.behavioural_months = Some(behavioural);
        self
    }
    const fn hqla(mut self, level: HqlaLevel) -> Self {
        self.hqla = Some(level);
        self
    }
    const fn lcr(mut self, class: &'static str) -> Self {
        self.lcr_class = Some(class);
        self
    }
    const fn nsfr(mut self, class: &'static str) -> Self {
        self.nsfr_class = Some(class);
        self
    }
}

use Amort::{Annuity, Bullet, Linear};
use BsSide::{Asset, Liability, OffBalance};

const TEMPLATES: &[Tpl] = &[
    // assets
    t("CASH", Asset, "CASH", "CENTRAL_BANK", "CASH_AND_RESERVES", 0.080, DOMESTIC).hqla(HqlaLevel::L1).nsfr("CASH_CB"),
    t("PLACE-1M", Asset, "INTERBANK", "FI", "PLACEMENT", 0.035, INTL).fixed(0.10, 1, None, Bullet).lcr("INFLOW_FI").nsfr("LOANS_FI"),
    t("PLACE-3M", Asset, "INTERBANK", "FI", "PLACEMENT", 0.030, INTL).fixed(0.15, 3, None, Bullet).lcr("INFLOW_FI").nsfr("LOANS_FI"),
    t("GOVT-2Y", Asset, "SECURITIES", "SOVEREIGN", "GOVT_BOND", 0.090, DOMESTIC).fixed(0.20, 24, Some(6), Bullet).hqla(HqlaLevel::L1).lcr("INFLOW_SECURITIES"),
    t("GOVT-5Y", Asset, "SECURITIES", "SOVEREIGN", "GOVT_BOND", 0.070, DOMESTIC).fixed(0.35, 60, Some(6), Bullet).hqla(HqlaLevel::L1).lcr("INFLOW_SECURITIES"),
    t("UST-2Y", Asset, "SECURITIES", "SOVEREIGN", "US_TREASURY", 0.060, USD).fixed(-0.10, 24, Some(6), Bullet).hqla(HqlaLevel::L1).lcr("INFLOW_SECURITIES"),
    t("SUKUK-3Y", Asset, "SECURITIES", "SOVEREIGN", "SUKUK", 0.035, DOMESTIC).fixed(0.25, 36, Some(6), Bullet).hqla(HqlaLevel::L1).lcr("INFLOW_SECURITIES"),
    t("GCC-SOV-3Y", Asset, "SECURITIES", "SOVEREIGN", "GOVT_BOND", 0.055, GCC).fixed(0.45, 36, Some(6), Bullet).hqla(HqlaLevel::L2A).lcr("INFLOW_SECURITIES"),
    t("CORP-BOND-4Y", Asset, "SECURITIES", "CORPORATE", "CORP_BOND", 0.027, USD).fixed(1.40, 48, Some(6), Bullet).hqla(HqlaLevel::L2B).lcr("INFLOW_SECURITIES"),
    t("MORTGAGE-FIX", Asset, "LOANS", "RETAIL", "MORTGAGE", 0.060, DOMESTIC).fixed(2.50, 240, Some(1), Annuity).lcr("INFLOW_RETAIL_SME").nsfr("LOANS_RW35"),
    t("MORTGAGE-FLT", Asset, "LOANS", "RETAIL", "MORTGAGE", 0.047, DOMESTIC).float(2.00, 6, 240, Some(1), Annuity).lcr("INFLOW_RETAIL_SME").nsfr("LOANS_RW35"),
    t("PERSONAL", Asset, "LOANS", "RETAIL", "PERSONAL", 0.090, DOMESTIC).fixed(4.00, 48, Some(1), Annuity).lcr("INFLOW_RETAIL_SME").nsfr("LOANS_OTHER"),
    t("SME-LOAN", Asset, "LOANS", "SME", "TERM_LOAN", 0.062, DOMESTIC).float(3.00, 3, 36, Some(3), Linear).lcr("INFLOW_RETAIL_SME").nsfr("LOANS_OTHER"),
    t("CORP-1Y", Asset, "LOANS", "CORPORATE", "TERM_LOAN", 0.080, MIXED).float(1.50, 3, 12, Some(3), Bullet).lcr("INFLOW_NONFIN").nsfr("LOANS_OTHER"),
    t("CORP-3Y", Asset, "LOANS", "CORPORATE", "TERM_LOAN", 0.090, MIXED).float(1.80, 3, 36, Some(6), Linear).lcr("INFLOW_NONFIN").nsfr("LOANS_OTHER"),
    t("CORP-5Y", Asset, "LOANS", "CORPORATE", "TERM_LOAN", 0.070, MIXED).float(2.00, 6, 60, Some(6), Linear).lcr("INFLOW_NONFIN").nsfr("LOANS_OTHER"),
    t("CORP-FIX-2Y", Asset, "LOANS", "CORPORATE", "TERM_LOAN", 0.040, MIXED).fixed(2.20, 24, Some(6), Bullet).lcr("INFLOW_NONFIN").nsfr("LOANS_OTHER"),
    t("OVERDRAFT", Asset, "LOANS", "CORPORATE", "OVERDRAFT", 0.030, DOMESTIC).admin(8.5, 0.8, 12).lcr("INFLOW_NONFIN").nsfr("LOANS_OTHER"),
    t("OTHER-ASSETS", Asset, "OTHER", "OTHER", "OTHER_ASSETS", 0.070, DOMESTIC).nsfr("OTHER"),
    // liabilities and equity
    t("CA-RETAIL", Liability, "DEPOSITS", "RETAIL", "CURRENT_ACCOUNT", 0.110, DOMESTIC).admin(0.0, 0.0, 36).lcr("RET_STABLE").nsfr("RETAIL_SME_STABLE"),
    t("SAV-RETAIL", Liability, "DEPOSITS", "RETAIL", "SAVINGS", 0.090, DOMESTIC).admin(1.5, 0.4, 48).lcr("RET_LESS_STABLE").nsfr("RETAIL_SME_LESS_STABLE"),
    t("TD-RETAIL-3M", Liability, "DEPOSITS", "RETAIL", "TERM_DEPOSIT", 0.060, DOMESTIC).fixed(-0.60, 3, None, Bullet).lcr("RET_STABLE").nsfr("RETAIL_SME_STABLE"),
    t("TD-RETAIL-1Y", Liability, "DEPOSITS", "RETAIL", "TERM_DEPOSIT", 0.100, DOMESTIC).fixed(-0.40, 12, None, Bullet).lcr("RET_STABLE").nsfr("RETAIL_SME_STABLE"),
    t("CA-SME", Liability, "DEPOSITS", "SME", "CURRENT_ACCOUNT", 0.035, DOMESTIC).admin(0.0, 0.0, 36).lcr("SME_STABLE").nsfr("RETAIL_SME_STABLE"),
    t("TD-SME-6M", Liability, "DEPOSITS", "SME", "TERM_DEPOSIT", 0.020, DOMESTIC).fixed(-0.50, 6, None, Bullet).lcr("SME_LESS_STABLE").nsfr("RETAIL_SME_LESS_STABLE"),
    t("CORP-OPER", Liability, "DEPOSITS", "CORPORATE", "OPERATIONAL_DEPOSIT", 0.090, MIXED).admin(0.5, 0.3, 24).lcr("OPERATIONAL").nsfr("OPERATIONAL"),
    t("CA-CORP", Liability, "DEPOSITS", "CORPORATE", "CURRENT_ACCOUNT", 0.040, MIXED).admin(0.0, 0.2, 12).lcr("NONFIN_WHOLESALE").nsfr("NONFIN_WHOLESALE"),
    t("TD-CORP-1M", Liability, "DEPOSITS", "CORPORATE", "TERM_DEPOSIT", 0.060, MIXED).fixed(-0.30, 1, None, Bullet).lcr("NONFIN_WHOLESALE").nsfr("NONFIN_WHOLESALE"),
    t("TD-CORP-6M", Liability, "DEPOSITS", "CORPORATE", "TERM_DEPOSIT", 0.060, MIXED).fixed(-0.20, 6, None, Bullet).lcr("NONFIN_WHOLESALE").nsfr("NONFIN_WHOLESALE"),
    t("FI-DEP-1M", Liability, "DEPOSITS", "FI", "TERM_DEPOSIT", 0.030, INTL).fixed(-0.10, 1, None, Bullet).lcr("FI_WHOLESALE").nsfr("FI"),
    t("FI-DEP-3M", Liability, "DEPOSITS", "FI", "TERM_DEPOSIT", 0.025, INTL).fixed(-0.05, 3, None, Bullet).lcr("FI_WHOLESALE").nsfr("FI"),
    t("INTERBANK-1M", Liability, "INTERBANK", "FI", "BORROWING", 0.055, INTL).fixed(0.05, 1, None, Bullet).lcr("FI_WHOLESALE").nsfr("FI"),
    t("TERM-3Y", Liability, "TERM_FUNDING", "FI", "SUKUK_ISSUED", 0.050, USD).fixed(0.90, 36, Some(6), Bullet).lcr("FI_WHOLESALE").nsfr("OTHER"),
    t("TERM-5Y", Liability, "TERM_FUNDING", "FI", "BOND_ISSUED", 0.030, USD).fixed(1.10, 60, Some(6), Bullet).lcr("FI_WHOLESALE").nsfr("OTHER"),
    t("OTHER-LIAB", Liability, "OTHER", "OTHER", "OTHER_LIABILITIES", 0.050, DOMESTIC).nsfr("OTHER"),
    t("EQUITY", Liability, "EQUITY", "SHAREHOLDERS", "EQUITY", 0.115, DOMESTIC).nsfr("CAPITAL"),
    // commitments (undrawn)
    t("COMMIT-CORP", OffBalance, "COMMITMENTS", "CORPORATE", "COMMITTED_CREDIT", 0.080, MIXED).lcr("COMMIT_CREDIT_CORP"),
    t("COMMIT-RETAIL", OffBalance, "COMMITMENTS", "RETAIL", "CARD_LINES", 0.020, DOMESTIC).lcr("COMMIT_CREDIT_RETAIL"),
    t("COMMIT-LIQ", OffBalance, "COMMITMENTS", "CORPORATE", "LIQUIDITY_FACILITY", 0.020, MIXED).lcr("COMMIT_LIQ_CORP"),
];

/// Snapshot dates: month-ends for `month_ends` months, then every day for `daily_history` days
/// up to and including the business date.
pub fn snapshot_dates(business_date: NaiveDate, cfg: &AlmCfg) -> Vec<NaiveDate> {
    let mut dates = Vec::new();
    let mut d = business_date - Duration::days(cfg.daily_history as i64);
    while d <= business_date {
        dates.push(d);
        d += Duration::days(1);
    }
    let first_daily = dates[0];
    let mut me = month_end(add_months(first_daily, -1));
    for _ in 0..cfg.month_ends {
        if me < first_daily {
            dates.push(me);
        }
        me = month_end(add_months(me, -1));
    }
    dates.sort();
    dates.dedup();
    dates
}

fn month_end(d: NaiveDate) -> NaiveDate {
    let next = add_months(NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap_or(d), 1);
    next - Duration::days(1)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Depositor {
    pub as_of: NaiveDate,
    pub group_name: String,
    pub segment: String,
    pub ccy: String,
    pub balance: f64,
}

#[derive(Debug, Clone, Default)]
pub struct Book {
    pub dates: Vec<NaiveDate>,
    pub positions: Vec<BsPosition>,
    pub depositors: Vec<Depositor>,
}

/// Build every snapshot. Pure: the same seed, business date and configuration give the same book.
pub fn generate(cfg: &AlmCfg, market: &Market, business_date: NaiveDate, seed: u64) -> Book {
    let dates = snapshot_dates(business_date, cfg);
    let earliest = dates[0];
    let asset_total: f64 = TEMPLATES.iter().filter(|t| t.side == Asset).map(|t| t.share).sum();
    let funding_total: f64 = TEMPLATES.iter().filter(|t| t.side == Liability).map(|t| t.share).sum();
    let mut positions = Vec::new();

    for tpl in TEMPLATES {
        let norm = match tpl.side {
            Asset => asset_total,
            Liability => funding_total,
            OffBalance => 1.0,
        };
        for (ccy, weight) in tpl.mix {
            let pid = format!("{}-{ccy}", tpl.pid);
            let mut rng = StdRng::seed_from_u64(seed ^ hash(&pid) ^ 0xb00c);
            let base_bhd = cfg.total_assets * tpl.share / norm * weight;
            // Where this cohort is in its maturity cycle: maturities repeat every term.
            let phase: f64 = rng.random_range(0.0..1.0);
            let first_maturity = tpl.term_months.map(|term| add_months(earliest, (term as f64 * phase).ceil().max(1.0) as i64));
            let mut factor = 1.0 + 0.05 * normal(&mut rng);
            let mut d = earliest;
            let mut next = 0usize;
            while d <= business_date && next < dates.len() {
                factor += 0.03 * (1.0 - factor) + 0.004 * normal(&mut rng);
                if d == dates[next] {
                    next += 1;
                    let spot = market.spot(ccy, d);
                    let balance = (base_bhd * factor / spot * 100.0).round() / 100.0;
                    // Roll the maturity forward by whole terms until it is after as_of.
                    let (origination, maturity) = match (tpl.term_months, first_maturity) {
                        (Some(term), Some(m0)) => {
                            let mut m = m0;
                            while m <= d {
                                m = add_months(m, term as i64);
                            }
                            (Some(add_months(m, -(term as i64))), Some(m))
                        }
                        _ => (None, None),
                    };
                    let next_repricing = match (tpl.rate_type, tpl.reset_months, origination) {
                        (RateType::Float, Some(reset), Some(o)) => {
                            let mut r = add_months(o, reset as i64);
                            while r <= d {
                                r = add_months(r, reset as i64);
                            }
                            Some(r.min(maturity.unwrap_or(r)))
                        }
                        _ => None,
                    };
                    let rate = match tpl.rate_type {
                        RateType::Fixed => {
                            let term_days = tpl.term_months.unwrap_or(12) as i32 * 30;
                            (market.rate(ccy, origination.unwrap_or(d), term_days) + tpl.spread).max(0.0)
                        }
                        RateType::Float => {
                            let reset = tpl.reset_months.unwrap_or(3);
                            let last_reset = next_repricing.map(|n| add_months(n, -(reset as i64))).unwrap_or(d);
                            (market.rate(ccy, last_reset, reset as i32 * 30) + tpl.spread).max(0.0)
                        }
                        RateType::Admin => tpl.spread,
                        RateType::NotSensitive => 0.0,
                    };
                    let (balance, undrawn) = if tpl.side == OffBalance { (0.0, balance) } else { (balance, 0.0) };
                    positions.push(BsPosition {
                        as_of: d,
                        position_id: pid.clone(),
                        side: tpl.side,
                        book: tpl.book.into(),
                        segment: tpl.segment.into(),
                        product: tpl.product.into(),
                        ccy: (*ccy).into(),
                        balance,
                        rate: (rate * 1e4).round() / 1e4,
                        rate_type: tpl.rate_type,
                        index_code: (tpl.rate_type == RateType::Float).then(|| index_for(ccy).to_string()),
                        spread: if tpl.rate_type == RateType::Float { tpl.spread } else { 0.0 },
                        reset_months: tpl.reset_months,
                        pay_months: tpl.pay_months,
                        amort: tpl.amort,
                        next_repricing,
                        maturity,
                        origination,
                        day_count: day_count(ccy),
                        hqla_level: tpl.hqla,
                        encumbered: false,
                        lcr_class: tpl.lcr_class.map(String::from),
                        nsfr_class: tpl.nsfr_class.map(String::from),
                        beta: tpl.beta,
                        behavioural_months: tpl.behavioural_months,
                        undrawn,
                    });
                }
                d += Duration::days(1);
            }
        }
    }

    // Depositor groups: shares of the deposit book that drift a little.
    let groups: [(&str, &str, &str, f64); 20] = [
        ("Government entity A", "SOVEREIGN", "BHD", 4.73),
        ("Government entity B", "SOVEREIGN", "BHD", 3.44),
        ("Government entity C", "SOVEREIGN", "USD", 2.15),
        ("Large corporate 1", "CORPORATE", "BHD", 2.58),
        ("Large corporate 2", "CORPORATE", "BHD", 2.24),
        ("Large corporate 3", "CORPORATE", "USD", 1.89),
        ("Large corporate 4", "CORPORATE", "BHD", 1.63),
        ("Large corporate 5", "CORPORATE", "BHD", 1.46),
        ("Large corporate 6", "CORPORATE", "USD", 1.29),
        ("Large corporate 7", "CORPORATE", "BHD", 1.12),
        ("Large corporate 8", "CORPORATE", "BHD", 0.95),
        ("Large corporate 9", "CORPORATE", "SAR", 0.77),
        ("Financial institution 1", "FI", "USD", 1.72),
        ("Financial institution 2", "FI", "USD", 1.20),
        ("Financial institution 3", "FI", "EUR", 0.69),
        ("Financial institution 4", "FI", "GBP", 0.52),
        ("High net worth group 1", "RETAIL", "BHD", 0.77),
        ("High net worth group 2", "RETAIL", "BHD", 0.60),
        ("High net worth group 3", "RETAIL", "USD", 0.52),
        ("High net worth group 4", "RETAIL", "BHD", 0.43),
    ];
    let mut depositors = Vec::new();
    for (name, segment, ccy, share_pct) in groups {
        let mut rng = StdRng::seed_from_u64(seed ^ hash(name) ^ 0xdeb0);
        let mut factor = 1.0;
        for d in &dates {
            factor += 0.1 * (1.0 - factor) + 0.02 * normal(&mut rng);
            let deposits_bhd: f64 = positions.iter().filter(|p| p.as_of == *d && p.side == Liability && p.book == "DEPOSITS").map(|p| p.balance * market.spot(&p.ccy, *d)).sum();
            let balance = deposits_bhd * share_pct / 100.0 * factor / market.spot(ccy, *d);
            depositors.push(Depositor { as_of: *d, group_name: name.into(), segment: segment.into(), ccy: ccy.into(), balance: (balance * 100.0).round() / 100.0 });
        }
    }

    Book { dates, positions, depositors }
}

/// A stable fingerprint of a book, for the determinism test and the log line.
pub fn digest(book: &Book) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut mix = |v: u64| h = (h ^ v).wrapping_mul(0x0100_0000_01b3);
    for p in &book.positions {
        mix(hash(&p.position_id));
        mix(p.as_of.num_days_from_ce() as u64);
        mix((p.balance * 100.0).round() as i64 as u64);
        mix((p.rate * 1e4).round() as i64 as u64);
        mix(p.maturity.map(|m| m.num_days_from_ce() as u64).unwrap_or(0));
    }
    for d in &book.depositors {
        mix(hash(&d.group_name));
        mix((d.balance * 100.0).round() as i64 as u64);
    }
    h
}

/// One month of net interest income for a book, side and currency: the actual from the last
/// snapshot of the month, and a budget that differs a little in balance and rate.
#[derive(Debug, Clone, PartialEq)]
pub struct PeriodRow {
    pub period: NaiveDate,
    pub book: String,
    pub side: BsSide,
    pub ccy: String,
    pub kind: &'static str,
    pub nii: f64,
    pub avg_balance: f64,
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Interest at each cohort's rate over the month's days, signed (assets positive, liabilities
/// negative), aggregated by book, side and currency from the last snapshot of every month.
pub fn periods(book: &Book, seed: u64) -> Vec<PeriodRow> {
    let mut last_in_month: BTreeMap<(i32, u32), NaiveDate> = BTreeMap::new();
    for d in &book.dates {
        let e = last_in_month.entry((d.year(), d.month())).or_insert(*d);
        if d > e {
            *e = *d;
        }
    }
    let mut rows = Vec::new();
    for ((y, m), d) in last_in_month {
        let Some(period) = NaiveDate::from_ymd_opt(y, m, 1) else { continue };
        let days = (add_months(period, 1) - period).num_days() as f64;
        let mut agg: BTreeMap<(String, BsSide, String), (f64, f64)> = BTreeMap::new();
        for p in book.positions.iter().filter(|p| p.as_of == d && p.side != BsSide::OffBalance && p.rate_type != RateType::NotSensitive) {
            let sign = if p.side == BsSide::Asset { 1.0 } else { -1.0 };
            let e = agg.entry((p.book.clone(), p.side, p.ccy.clone())).or_insert((0.0, 0.0));
            e.0 += sign * p.balance * p.rate / 100.0 * days / p.day_count.basis();
            e.1 += p.balance;
        }
        for ((b, side, ccy), (nii, balance)) in agg {
            let mut rng = StdRng::seed_from_u64(seed ^ hash(&format!("{b}|{side}|{ccy}|{y}-{m}")) ^ 0xb0d6);
            let balance_factor = 0.95 + 0.08 * rng.random_range(0.0..1.0);
            let rate_factor = 0.97 + 0.06 * rng.random_range(0.0..1.0);
            rows.push(PeriodRow { period, book: b.clone(), side, ccy: ccy.clone(), kind: "ACTUAL", nii: round2(nii), avg_balance: round2(balance) });
            rows.push(PeriodRow { period, book: b, side, ccy, kind: "BUDGET", nii: round2(nii * balance_factor * rate_factor), avg_balance: round2(balance * balance_factor) });
        }
    }
    rows
}

// ---------------------------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AlmSummary {
    pub snapshots: usize,
    pub skipped: usize,
    pub positions: usize,
    pub depositors: usize,
    pub rates: usize,
    pub spots: usize,
    pub periods: usize,
    /// Synthetic FTP publications added.
    pub ftp: usize,
}

/// Generate and persist. Snapshots already complete are left alone, so re-runs are cheap and a
/// snapshot interrupted half way is rebuilt. Every table is written in bulk.
pub async fn run(pool: &PgPool, _cal: &Calendar, cfg: &AlmCfg, business_date: NaiveDate, seed: u64) -> anyhow::Result<AlmSummary> {
    let market = Market::generate(seed, business_date, cfg.rate_history_days);
    let book = generate(cfg, &market, business_date, seed);
    tracing::info!(snapshots = book.dates.len(), positions = book.positions.len(), digest = format!("{:016x}", digest(&book)), "banking book generated");
    let mut summary = write(pool, &market, &book, business_date, seed).await?;
    summary.ftp = crate::pmu_synth::write_ftp(pool, _cal, &market).await?;
    Ok(summary)
}

pub async fn write(pool: &PgPool, market: &Market, book: &Book, business_date: NaiveDate, seed: u64) -> anyhow::Result<AlmSummary> {
    let mut summary = AlmSummary::default();

    for chunk in market.rates.chunks(2000) {
        let mut qb = QueryBuilder::new("INSERT INTO market_rates (as_of, ccy, index_code, tenor, tenor_days, rate) ");
        qb.push_values(chunk, |mut b, r| {
            b.push_bind(r.as_of).push_bind(&r.ccy).push_bind(&r.index_code).push_bind(&r.tenor).push_bind(r.tenor_days).push_bind(r.rate);
        });
        qb.push(" ON CONFLICT DO NOTHING");
        summary.rates += qb.build().execute(pool).await?.rows_affected() as usize;
    }
    for chunk in market.spots.chunks(2000) {
        let mut qb = QueryBuilder::new("INSERT INTO fx_spots (as_of, pair, mid) ");
        qb.push_values(chunk, |mut b, s| {
            b.push_bind(s.as_of).push_bind(&s.pair).push_bind(s.mid);
        });
        qb.push(" ON CONFLICT DO NOTHING");
        summary.spots += qb.build().execute(pool).await?.rows_affected() as usize;
    }
    // The live curve starts as the last close; realtime ticks move it later.
    let last = market.rates.iter().map(|r| r.as_of).max();
    if let Some(last) = last {
        let live: Vec<&RatePoint> = market.rates.iter().filter(|r| r.as_of == last).collect();
        let mut qb = QueryBuilder::new("INSERT INTO market_rates_live (ccy, index_code, tenor, tenor_days, rate, as_of) ");
        let at = last.and_hms_opt(16, 0, 0).map(|t| t.and_utc()).unwrap_or_else(Utc::now);
        qb.push_values(live, |mut b, r| {
            b.push_bind(&r.ccy).push_bind(&r.index_code).push_bind(&r.tenor).push_bind(r.tenor_days).push_bind(r.rate).push_bind(at);
        });
        qb.push(" ON CONFLICT (ccy, index_code, tenor) DO UPDATE SET rate = excluded.rate, as_of = excluded.as_of WHERE market_rates_live.as_of < excluded.as_of");
        qb.build().execute(pool).await?;
    }

    for as_of in &book.dates {
        let state: Option<bool> = sqlx::query_scalar("SELECT complete FROM bs_snapshots WHERE as_of = $1").bind(as_of).fetch_optional(pool).await?;
        match state {
            Some(true) => {
                summary.skipped += 1;
                continue;
            }
            Some(false) => {
                sqlx::query("DELETE FROM bs_positions WHERE as_of = $1").bind(as_of).execute(pool).await?;
                sqlx::query("DELETE FROM bs_depositors WHERE as_of = $1").bind(as_of).execute(pool).await?;
            }
            None => {
                sqlx::query("INSERT INTO bs_snapshots (as_of, business_date, generated_at, seed, complete) VALUES ($1, $2, $3, $4, false)")
                    .bind(as_of)
                    .bind(business_date)
                    .bind(Utc::now())
                    .bind(seed as i64)
                    .execute(pool)
                    .await?;
            }
        }
        let rows: Vec<&BsPosition> = book.positions.iter().filter(|p| p.as_of == *as_of).collect();
        for chunk in rows.chunks(400) {
            let mut qb = QueryBuilder::new(
                "INSERT INTO bs_positions (as_of, position_id, side, book, segment, product, ccy, balance, rate, rate_type, index_code, spread, \
                 reset_months, pay_months, amort, next_repricing, maturity, origination, day_count, hqla_level, encumbered, lcr_class, nsfr_class, \
                 beta, behavioural_months, undrawn) ",
            );
            qb.push_values(chunk, |mut b, p| {
                b.push_bind(p.as_of)
                    .push_bind(&p.position_id)
                    .push_bind(p.side.as_str())
                    .push_bind(&p.book)
                    .push_bind(&p.segment)
                    .push_bind(&p.product)
                    .push_bind(&p.ccy)
                    .push_bind(p.balance)
                    .push_bind(p.rate)
                    .push_bind(p.rate_type.as_str())
                    .push_bind(p.index_code.as_deref())
                    .push_bind(p.spread)
                    .push_bind(p.reset_months)
                    .push_bind(p.pay_months)
                    .push_bind(p.amort.as_str())
                    .push_bind(p.next_repricing)
                    .push_bind(p.maturity)
                    .push_bind(p.origination)
                    .push_bind(p.day_count.as_str())
                    .push_bind(p.hqla_level.map(|h| h.as_str()))
                    .push_bind(p.encumbered)
                    .push_bind(p.lcr_class.as_deref())
                    .push_bind(p.nsfr_class.as_deref())
                    .push_bind(p.beta)
                    .push_bind(p.behavioural_months)
                    .push_bind(p.undrawn);
            });
            qb.push(" ON CONFLICT DO NOTHING");
            summary.positions += qb.build().execute(pool).await?.rows_affected() as usize;
        }
        let deps: Vec<&Depositor> = book.depositors.iter().filter(|d| d.as_of == *as_of).collect();
        if !deps.is_empty() {
            let mut qb = QueryBuilder::new("INSERT INTO bs_depositors (as_of, group_name, segment, ccy, balance) ");
            qb.push_values(deps, |mut b, d| {
                b.push_bind(d.as_of).push_bind(&d.group_name).push_bind(&d.segment).push_bind(&d.ccy).push_bind(d.balance);
            });
            qb.push(" ON CONFLICT DO NOTHING");
            summary.depositors += qb.build().execute(pool).await?.rows_affected() as usize;
        }
        sqlx::query("UPDATE bs_snapshots SET complete = true WHERE as_of = $1").bind(as_of).execute(pool).await?;
        sqlx::query("SELECT pg_notify('alm_events', json_build_object('snapshot', $1::date)::text)").bind(as_of).execute(pool).await?;
        summary.snapshots += 1;
    }
    let rows = periods(book, seed);
    for chunk in rows.chunks(1000) {
        let mut qb = QueryBuilder::new("INSERT INTO alm_period (period, book, side, ccy, kind, nii, avg_balance) ");
        qb.push_values(chunk, |mut b, r| {
            b.push_bind(r.period).push_bind(&r.book).push_bind(r.side.as_str()).push_bind(&r.ccy).push_bind(r.kind).push_bind(r.nii).push_bind(r.avg_balance);
        });
        qb.push(" ON CONFLICT (period, book, side, ccy, kind) DO UPDATE SET nii = excluded.nii, avg_balance = excluded.avg_balance");
        summary.periods += qb.build().execute(pool).await?.rows_affected() as usize;
    }
    Ok(summary)
}
