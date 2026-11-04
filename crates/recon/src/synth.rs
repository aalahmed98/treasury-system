//! A synthetic world of deals and the observations each source would emit for them, with the
//! failure modes real feeds have: lag, drops, duplicates, out-of-order delivery, rounding,
//! counterparty name variants, reversed quotes and missing cross references. Deterministic for a
//! given seed and configuration.

use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, Utc, Weekday};
use domain::{EventKind, Product, Side, Source};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::calendar::Calendar;

// ---------------------------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthConfig {
    pub world: WorldCfg,
    pub mix: MixCfg,
    #[serde(default)]
    pub source: BTreeMap<String, SourceCfg>,
    /// The synthetic banking book generated after the deals.
    #[serde(default)]
    pub alm: crate::alm_synth::AlmCfg,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldCfg {
    pub seed: u64,
    pub start: NaiveDate,
    pub days: u32,
    pub deals_per_day: u32,
    /// Share of deals the desk also books internally.
    pub internal_share: f64,
}

/// Scenario weights; normalised when drawing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MixCfg {
    pub clean: f64,
    pub late: f64,
    pub missing: f64,
    pub mismatch: f64,
    pub orphan: f64,
    pub ambiguous: f64,
    pub duplicate: f64,
    pub cancel: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceCfg {
    pub lag_median: String,
    pub lag_p95: String,
    #[serde(default)]
    pub drop_p: f64,
    #[serde(default)]
    pub dup_p: f64,
    #[serde(default)]
    pub reorder_p: f64,
    #[serde(default = "default_rate_dp")]
    pub rate_dp: u32,
    #[serde(default)]
    pub cpty_variant_p: f64,
    #[serde(default)]
    pub reversed_pair_p: f64,
    #[serde(default = "default_cross_ref_p")]
    pub cross_ref_p: f64,
}

fn default_rate_dp() -> u32 {
    5
}
fn default_cross_ref_p() -> f64 {
    1.0
}

impl Default for SynthConfig {
    fn default() -> Self {
        toml::from_str(DEFAULT_CONFIG).expect("default config parses")
    }
}

pub const DEFAULT_CONFIG: &str = r#"
[world]
seed = 42
start = "2026-10-05"
days = 30
deals_per_day = 40
internal_share = 0.3

[mix]
clean = 0.80
late = 0.05
missing = 0.04
mismatch = 0.04
orphan = 0.02
ambiguous = 0.02
duplicate = 0.02
cancel = 0.01

[source.KONDOR]
lag_median = "2m"
lag_p95 = "6m"
drop_p = 0.0
dup_p = 0.02
reorder_p = 0.03
rate_dp = 5
cpty_variant_p = 0.3
cross_ref_p = 0.7

[source.BLOOMBERG]
lag_median = "5s"
lag_p95 = "30s"
drop_p = 0.0
dup_p = 0.01
reorder_p = 0.0
rate_dp = 5
cpty_variant_p = 0.5
reversed_pair_p = 0.4

[source.QUANTUM]
lag_median = "12m"
lag_p95 = "25m"
drop_p = 0.0
dup_p = 0.01
reorder_p = 0.02
rate_dp = 4
cpty_variant_p = 0.2
cross_ref_p = 0.9
"#;

pub fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    let (num, unit) = s.split_at(s.find(|c: char| c.is_ascii_alphabetic())?);
    let n: f64 = num.trim().parse().ok()?;
    let secs = match unit {
        "s" => n,
        "m" => n * 60.0,
        "h" => n * 3600.0,
        "d" => n * 86400.0,
        _ => return None,
    };
    Some(Duration::milliseconds((secs * 1000.0) as i64))
}

// ---------------------------------------------------------------------------------------------
// The world
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scenario {
    Clean,
    /// One source delivers well beyond its SLA.
    Late(Source),
    /// One source never delivers.
    Missing(Source),
    /// Quantum's image differs in one field.
    Mismatch(MismatchField),
    /// A back-office record with no front-office counterpart.
    Orphan,
    /// Two front-office deals with identical economics and a confirmation that names neither.
    Ambiguous,
    /// Kondor books the same deal twice.
    Duplicate,
    /// Kondor cancels after the back office picked it up.
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MismatchField {
    Rate,
    Notional,
    ValueDate,
    Counterparty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrueDeal {
    pub n: u32,
    pub product: Product,
    pub side: Side,
    pub ccy1: String,
    pub ccy2: Option<String>,
    pub notional: Decimal,
    pub rate: Decimal,
    pub executed_at: DateTime<Utc>,
    pub trade_date: NaiveDate,
    pub value_date: NaiveDate,
    pub maturity_date: Option<NaiveDate>,
    pub counterparty: String,
    pub desk: &'static str,
    pub trader: &'static str,
    pub internal: bool,
    pub scenario: Scenario,
    /// For Ambiguous: the twin deal shares these economics.
    pub twin_of: Option<u32>,
}

/// One thing the feed does at a point in simulated time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Emission {
    Event {
        at: DateTime<Utc>,
        source: Source,
        source_ref: String,
        kind: EventKind,
        seq: Option<i64>,
        source_ts: DateTime<Utc>,
        payload: Value,
    },
    /// The desk books the deal in our own system (through the booking role).
    Internal { at: DateTime<Utc>, deal: u32 },
    Heartbeat { at: DateTime<Utc>, source: Source },
}

impl Emission {
    pub fn at(&self) -> DateTime<Utc> {
        match self {
            Emission::Event { at, .. } | Emission::Internal { at, .. } | Emission::Heartbeat { at, .. } => *at,
        }
    }
}

const COUNTERPARTIES: &[(&str, &[&str])] = &[
    ("Gulf International Bank", &["GIB", "Gulf Intl Bank", "GULF INTERNATIONAL BANK B.S.C."]),
    ("Arab Banking Corporation", &["ABC", "Bank ABC", "ARAB BANKING CORPORATION BSC"]),
    ("BBK", &["Bank of Bahrain and Kuwait", "BBK B.S.C."]),
    ("Ahli United Bank", &["AUB", "Ahli United", "AHLI UNITED BANK BSC"]),
    ("Al Salam Bank", &["Al Salam", "ALSALAM BANK", "Al Salam Bank B.S.C."]),
    ("HSBC Bahrain", &["HSBC", "HSBC Bank Middle East"]),
    ("Standard Chartered", &["SCB", "StanChart", "Standard Chartered Bank"]),
    ("Citibank N.A.", &["Citi", "CITIBANK NA"]),
    ("JPMorgan Chase", &["JPM", "JP Morgan", "JPMORGAN"]),
    ("Emirates NBD", &["ENBD", "Emirates NBD Bank"]),
    ("Al Rajhi Bank", &["AlRajhi", "Al Rajhi"]),
    ("Qatar National Bank", &["QNB"]),
];

const FX_PAIRS: &[(&str, &str, f64)] = &[
    ("USD", "BHD", 0.37600),
    ("EUR", "USD", 1.08500),
    ("GBP", "USD", 1.27000),
    ("USD", "SAR", 3.75000),
    ("EUR", "BHD", 0.40800),
    ("USD", "AED", 3.67250),
];
const NOTIONALS: &[i64] = &[100_000, 250_000, 500_000, 1_000_000, 2_000_000, 5_000_000, 10_000_000, 25_000_000];
const FWD_TENORS: &[i64] = &[7, 14, 30, 60, 90, 180];
const MM_CCYS: &[&str] = &["BHD", "USD", "SAR"];
const MM_TENORS: &[i64] = &[1, 7, 14, 30, 90, 180];

pub struct World {
    pub deals: Vec<TrueDeal>,
    pub cfg: SynthConfig,
    /// The simulated present: nothing after this is emitted (later settlements have not happened yet).
    pub window_end: DateTime<Utc>,
}

impl World {
    pub fn generate(cfg: SynthConfig, cal: &Calendar) -> World {
        let mut rng = StdRng::seed_from_u64(cfg.world.seed);
        let mut deals = Vec::new();
        let mut n = 0u32;
        let mut day = cfg.world.start;
        let mut days_done = 0;
        let mut last_day = day;
        while days_done < cfg.world.days {
            if !cal.is_business_day(day) {
                day += Duration::days(1);
                continue;
            }
            let count = cfg.world.deals_per_day;
            for _ in 0..count {
                n += 1;
                let scenario = draw_scenario(&mut rng, &cfg.mix);
                let mut d = gen_deal(&mut rng, n, day, cal, &cfg);
                d.scenario = scenario;
                if scenario == Scenario::Ambiguous {
                    let mut twin = d.clone();
                    n += 1;
                    twin.n = n;
                    twin.scenario = Scenario::Clean;
                    twin.twin_of = Some(d.n);
                    twin.executed_at = d.executed_at + Duration::seconds(rng.random_range(30..600));
                    twin.internal = false;
                    // 0.8 pip (2 bp) apart: distinct deals, each within tolerance of a confirmation in the middle.
                    let half = if d.product.is_fx() { Decimal::new(4, 5) } else { Decimal::new(8, 3) };
                    let mid = if d.product.is_fx() { d.rate.round_dp(4) } else { d.rate.round_dp(2) };
                    d.rate = mid - half;
                    twin.rate = mid + half;
                    deals.push(d);
                    deals.push(twin);
                } else {
                    deals.push(d);
                }
            }
            last_day = day;
            day += Duration::days(1);
            days_done += 1;
        }
        let window_end = (last_day + Duration::days(1)).and_time(NaiveTime::from_hms_opt(23, 59, 59).unwrap()).and_utc();
        World { deals, cfg, window_end }
    }

    /// Everything the feeds do, in time order.
    pub fn emit(&self, cal: &Calendar) -> Vec<Emission> {
        let mut rng = StdRng::seed_from_u64(self.cfg.world.seed ^ 0x5eed);
        let mut out = Vec::new();
        let kondor = self.source_cfg("KONDOR");
        let bloomberg = self.source_cfg("BLOOMBERG");
        let quantum = self.source_cfg("QUANTUM");

        for d in &self.deals {
            emit_deal(d, &mut rng, &mut out, &kondor, &bloomberg, &quantum, cal);
        }

        // Only what has happened by the simulated present.
        out.retain(|e| e.at() <= self.window_end);

        // Heartbeats every two simulated minutes across the trading window, per feed.
        let first = out.iter().map(Emission::at).min().unwrap_or_else(Utc::now);
        let mut t = first;
        while t <= self.window_end {
            for s in [Source::Kondor, Source::Bloomberg, Source::Quantum] {
                out.push(Emission::Heartbeat { at: t, source: s });
            }
            t += Duration::seconds(120);
        }

        out.sort_by(|a, b| a.at().cmp(&b.at()).then_with(|| emission_key(a).cmp(&emission_key(b))));
        out
    }

    fn source_cfg(&self, code: &str) -> SourceCfg {
        self.cfg.source.get(code).cloned().unwrap_or(SourceCfg {
            lag_median: "1m".into(),
            lag_p95: "3m".into(),
            drop_p: 0.0,
            dup_p: 0.0,
            reorder_p: 0.0,
            rate_dp: 5,
            cpty_variant_p: 0.0,
            reversed_pair_p: 0.0,
            cross_ref_p: 1.0,
        })
    }
}

fn emission_key(e: &Emission) -> String {
    match e {
        Emission::Event { source, source_ref, kind, .. } => format!("1:{}:{}:{}", source.as_str(), source_ref, kind.as_str()),
        Emission::Internal { deal, .. } => format!("0:{deal}"),
        Emission::Heartbeat { source, .. } => format!("2:{}", source.as_str()),
    }
}

fn draw_scenario(rng: &mut StdRng, m: &MixCfg) -> Scenario {
    let weights = [
        (Scenario::Clean, m.clean),
        (Scenario::Late(Source::Quantum), m.late * 0.6),
        (Scenario::Late(Source::Kondor), m.late * 0.4),
        (Scenario::Missing(Source::Quantum), m.missing * 0.7),
        (Scenario::Missing(Source::Kondor), m.missing * 0.3),
        (Scenario::Mismatch(MismatchField::Rate), m.mismatch * 0.4),
        (Scenario::Mismatch(MismatchField::Notional), m.mismatch * 0.2),
        (Scenario::Mismatch(MismatchField::ValueDate), m.mismatch * 0.2),
        (Scenario::Mismatch(MismatchField::Counterparty), m.mismatch * 0.2),
        (Scenario::Orphan, m.orphan),
        (Scenario::Ambiguous, m.ambiguous),
        (Scenario::Duplicate, m.duplicate),
        (Scenario::Cancel, m.cancel),
    ];
    let total: f64 = weights.iter().map(|(_, w)| w).sum();
    let mut x = rng.random_range(0.0..total.max(f64::EPSILON));
    for (s, w) in weights {
        if x < w {
            return s;
        }
        x -= w;
    }
    Scenario::Clean
}

fn pick<'a, T>(rng: &mut StdRng, xs: &'a [T]) -> &'a T {
    &xs[rng.random_range(0..xs.len())]
}

fn dec(v: f64, scale: u32) -> Decimal {
    Decimal::from_f64(v).unwrap_or_default().round_dp(scale)
}

fn gen_deal(rng: &mut StdRng, n: u32, day: NaiveDate, cal: &Calendar, cfg: &SynthConfig) -> TrueDeal {
    let hour = rng.random_range(7..15);
    let minute = rng.random_range(0..60);
    let second = rng.random_range(0..60);
    let executed_at = day.and_time(NaiveTime::from_hms_opt(hour, minute, second).unwrap()).and_utc();
    let is_fx = rng.random_range(0..100) < 60;
    let (cpty, _) = pick(rng, COUNTERPARTIES);
    let internal = rng.random_range(0.0..1.0) < cfg.world.internal_share;
    if is_fx {
        let (c1, c2, _) = *pick(rng, FX_PAIRS);
        // Priced off the same synthetic market the ALM generator writes, so revaluation and the
        // off-market check see deals done near the mid of their day.
        let mid = crate::alm_synth::fx_cross(cfg.world.seed, c1, c2, day);
        let spot = cal.add_business_days(day, 2);
        let is_fwd = rng.random_range(0..100) < 35;
        let (product, value_date) = if is_fwd {
            (Product::FxFwd, cal.add_business_days(spot, *pick(rng, FWD_TENORS)))
        } else {
            (Product::FxSpot, spot)
        };
        let jitter = 1.0 + rng.random_range(-0.003..0.003);
        // Covered interest parity on the two money-market levels: the quote currency's rate over the base's.
        let t = (value_date - spot).num_days() as f64 / 360.0;
        let fwd_points = if is_fwd { (1.0 + crate::alm_synth::money_rate(c2) / 100.0 * t) / (1.0 + crate::alm_synth::money_rate(c1) / 100.0 * t) } else { 1.0 };
        TrueDeal {
            n,
            product,
            side: if rng.random_range(0..2) == 0 { Side::Buy } else { Side::Sell },
            ccy1: c1.into(),
            ccy2: Some(c2.into()),
            notional: Decimal::from(*pick(rng, NOTIONALS) * rng.random_range(1..4)),
            rate: dec(mid * jitter * fwd_points, 5),
            executed_at,
            trade_date: day,
            value_date,
            maturity_date: None,
            counterparty: cpty.to_string(),
            desk: "FX",
            trader: if rng.random_range(0..2) == 0 { "fx1" } else { "fx2" },
            internal,
            scenario: Scenario::Clean,
            twin_of: None,
        }
    } else {
        let tenor = *pick(rng, MM_TENORS);
        let value_date = if tenor == 1 { day } else { cal.add_business_days(day, 2) };
        let is_depo = rng.random_range(0..2) == 0;
        TrueDeal {
            n,
            product: if is_depo { Product::MmDepo } else { Product::MmLoan },
            side: if is_depo { Side::Lend } else { Side::Borrow },
            ccy1: pick(rng, MM_CCYS).to_string(),
            ccy2: None,
            notional: Decimal::from(*pick(rng, NOTIONALS) * rng.random_range(1..4)),
            rate: dec(rng.random_range(4.0..6.5), 4),
            executed_at,
            trade_date: day,
            value_date,
            maturity_date: Some(cal.add_business_days(value_date, tenor)),
            counterparty: cpty.to_string(),
            desk: "MM",
            trader: "mm1",
            internal,
            scenario: Scenario::Clean,
            twin_of: None,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Per-source emission
// ---------------------------------------------------------------------------------------------

struct Lag {
    median: Duration,
    p95: Duration,
}

impl Lag {
    fn from(cfg: &SourceCfg) -> Self {
        Self {
            median: parse_duration(&cfg.lag_median).unwrap_or(Duration::minutes(1)),
            p95: parse_duration(&cfg.lag_p95).unwrap_or(Duration::minutes(3)),
        }
    }

    /// Log-normal-ish draw: median most of the time, with a tail reaching p95.
    fn draw(&self, rng: &mut StdRng) -> Duration {
        let m = self.median.num_milliseconds().max(1) as f64;
        let p = self.p95.num_milliseconds().max(2) as f64;
        let sigma = ((p / m).ln() / 1.645).max(0.05);
        let z: f64 = normal(rng);
        Duration::milliseconds((m * (sigma * z).exp()) as i64)
    }
}

fn normal(rng: &mut StdRng) -> f64 {
    let u1: f64 = rng.random_range(f64::EPSILON..1.0);
    let u2: f64 = rng.random_range(0.0..1.0);
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

fn cpty_name(rng: &mut StdRng, canonical: &str, variant_p: f64) -> String {
    if rng.random_range(0.0..1.0) < variant_p {
        if let Some((_, variants)) = COUNTERPARTIES.iter().find(|(c, _)| *c == canonical) {
            return pick(rng, variants).to_string();
        }
    }
    canonical.to_string()
}

fn ymd(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn ymd_compact(d: NaiveDate) -> String {
    d.format("%Y%m%d").to_string()
}

fn kondor_payload(d: &TrueDeal, status: &str, cpty: &str, refs: &BTreeMap<&str, String>, rate_dp: u32, trader_tag: &str) -> Value {
    let instrument = match d.product {
        Product::FxSpot => "FXSPOT",
        Product::FxFwd => "FXFWD",
        Product::MmDepo => "DEPOSIT",
        Product::MmLoan => "LOAN",
    };
    json!({
        "deal_id": format!("K{:06}", d.n),
        "portfolio": if d.desk == "FX" { "FX-BOOK" } else { "MM-BOOK" },
        "instrument": instrument,
        "direction": d.side.as_str(),
        "ccy1": d.ccy1, "ccy2": d.ccy2,
        "amount": d.notional.normalize(),
        "rate": d.rate.round_dp(rate_dp).normalize(),
        "trade_date": ymd(d.trade_date), "value_date": ymd(d.value_date),
        "maturity_date": d.maturity_date.map(ymd),
        "counterparty": cpty, "trader": trader_tag, "status": status,
        "refs": refs,
    })
}

fn bloomberg_payload(d: &TrueDeal, state: &str, cpty: &str, reversed: bool, rate_dp: u32) -> Value {
    let ticket = format!("VCON{:07}", d.n * 7 + 1000);
    let product = match d.product {
        Product::FxSpot => "SPOT",
        Product::FxFwd => "FWD",
        Product::MmDepo => "DEPO",
        Product::MmLoan => "LOAN",
    };
    let (pair, qty, price, side) = match (&d.ccy2, reversed) {
        (Some(c2), true) => {
            let qty = (d.notional * d.rate).round_dp(2);
            let price = (Decimal::ONE / d.rate).round_dp(rate_dp);
            (format!("{}/{}", c2, d.ccy1), qty, price, d.side.flipped())
        }
        (Some(c2), false) => (format!("{}/{}", d.ccy1, c2), d.notional, d.rate.round_dp(rate_dp), d.side),
        (None, _) => (String::new(), d.notional, d.rate.round_dp(rate_dp), d.side),
    };
    let side = match side {
        Side::Buy => "B",
        Side::Sell => "S",
        Side::Lend => "L",
        Side::Borrow => "BO",
    };
    json!({
        "ticket": ticket, "account": if d.desk == "FX" { "NBB-FX" } else { "NBB-MM" }, "product": product,
        "side": side, "pair": if pair.is_empty() { Value::Null } else { Value::String(pair) }, "ccy": if d.ccy2.is_none() { Value::String(d.ccy1.clone()) } else { Value::Null },
        "qty": qty.normalize(), "price": price.normalize(),
        "trade_dt": ymd_compact(d.trade_date), "settle_dt": ymd_compact(d.value_date),
        "maturity_dt": d.maturity_date.map(ymd_compact), "cpty": cpty, "state": state,
    })
}

fn quantum_payload(d: &TrueDeal, status: &str, cpty: &str, front_ref: Option<&str>, rate_dp: u32, mismatch: Option<MismatchField>, cal: &Calendar) -> Value {
    let mut amount = d.notional;
    // For the ambiguous scenario the confirmation sits exactly between the two front-office rates.
    let mut rate = if d.scenario == Scenario::Ambiguous {
        if d.product.is_fx() { (d.rate + Decimal::new(4, 5)).round_dp(4) } else { (d.rate + Decimal::new(8, 3)).round_dp(2) }
    } else {
        d.rate.round_dp(rate_dp)
    };
    let mut value_date = d.value_date;
    let mut cpty = cpty.to_string();
    match mismatch {
        Some(MismatchField::Rate) => rate += if d.product.is_fx() { Decimal::new(5, 4) } else { Decimal::new(5, 2) },
        Some(MismatchField::Notional) => amount *= Decimal::TEN,
        Some(MismatchField::ValueDate) => value_date = cal.add_business_days(value_date, 1),
        Some(MismatchField::Counterparty) => cpty = "UNKNOWN COUNTERPARTY LTD".into(),
        None => {}
    }
    json!({
        "trade_no": format!("Q{:07}", d.n * 3 + 500), "book": if d.desk == "FX" { "TRS-FX" } else { "TRS-MM" },
        "type": d.product.as_str(), "direction": d.side.as_str(), "ccy1": d.ccy1, "ccy2": d.ccy2,
        "amount": amount.normalize(), "rate": rate.normalize(),
        "trade_date": ymd(d.trade_date), "value_date": ymd(value_date), "maturity_date": d.maturity_date.map(ymd),
        "counterparty": cpty, "status": status, "front_ref": front_ref,
    })
}

#[allow(clippy::too_many_arguments)]
fn emit_deal(d: &TrueDeal, rng: &mut StdRng, out: &mut Vec<Emission>, kc: &SourceCfg, bc: &SourceCfg, qc: &SourceCfg, cal: &Calendar) {
    let k_lag = Lag::from(kc);
    let b_lag = Lag::from(bc);
    let q_lag = Lag::from(qc);
    let kondor_ref = format!("K{:06}", d.n);
    let bb_ticket = format!("VCON{:07}", d.n * 7 + 1000);
    let q_no = format!("Q{:07}", d.n * 3 + 500);
    let push = |out: &mut Vec<Emission>, at: DateTime<Utc>, source: Source, source_ref: &str, kind: EventKind, seq: Option<i64>, payload: Value| {
        out.push(Emission::Event { at, source, source_ref: source_ref.to_string(), kind, seq, source_ts: at, payload });
    };

    // Orphan: only the back office ever sees it.
    if d.scenario == Scenario::Orphan {
        let at = d.executed_at + q_lag.draw(rng) + Duration::minutes(10);
        let cpty = cpty_name(rng, &d.counterparty, qc.cpty_variant_p);
        push(out, at, Source::Quantum, &q_no, EventKind::New, Some(1), quantum_payload(d, "PENDING", &cpty, Some("K-UNKNOWN"), qc.rate_dp, None, cal));
        return;
    }

    // Bloomberg execution (FX only), immediately after execution.
    let fx = d.product.is_fx();
    let bloomberg_seen = fx && d.scenario != Scenario::Missing(Source::Bloomberg);
    if bloomberg_seen {
        let at = d.executed_at + b_lag.draw(rng);
        let cpty = cpty_name(rng, &d.counterparty, bc.cpty_variant_p);
        let reversed = rng.random_range(0.0..1.0) < bc.reversed_pair_p;
        let payload = bloomberg_payload(d, "DONE", &cpty, reversed, bc.rate_dp);
        push(out, at, Source::Bloomberg, &bb_ticket, EventKind::New, None, payload.clone());
        if rng.random_range(0.0..1.0) < bc.dup_p {
            push(out, at + Duration::seconds(rng.random_range(5..120)), Source::Bloomberg, &bb_ticket, EventKind::New, None, payload);
        }
    }

    // Internal booking by the desk.
    if d.internal {
        out.push(Emission::Internal { at: d.executed_at + Duration::seconds(rng.random_range(20..180)), deal: d.n });
    }

    // Kondor front-office booking.
    let kondor_seen = d.scenario != Scenario::Missing(Source::Kondor);
    let mut kondor_at = d.executed_at + k_lag.draw(rng);
    if d.scenario == Scenario::Late(Source::Kondor) {
        kondor_at = d.executed_at + Duration::minutes(rng.random_range(12..40));
    }
    if kondor_seen {
        let cpty = cpty_name(rng, &d.counterparty, kc.cpty_variant_p);
        let mut refs = BTreeMap::new();
        if bloomberg_seen && rng.random_range(0.0..1.0) < kc.cross_ref_p {
            refs.insert("BLOOMBERG", bb_ticket.clone());
        }
        let new_payload = kondor_payload(d, "NEW", &cpty, &refs, kc.rate_dp, d.trader);
        let amend_payload = kondor_payload(d, "VERIFIED", &cpty, &refs, kc.rate_dp, d.trader);
        let reorder = rng.random_range(0.0..1.0) < kc.reorder_p;
        let verify_at = kondor_at + Duration::minutes(rng.random_range(3..20));
        if reorder {
            // The verification amend overtakes the new.
            push(out, kondor_at, Source::Kondor, &kondor_ref, EventKind::Amend, Some(2), amend_payload);
            push(out, kondor_at + Duration::seconds(rng.random_range(5..60)), Source::Kondor, &kondor_ref, EventKind::New, Some(1), new_payload);
        } else {
            push(out, kondor_at, Source::Kondor, &kondor_ref, EventKind::New, Some(1), new_payload.clone());
            push(out, verify_at, Source::Kondor, &kondor_ref, EventKind::Amend, Some(2), amend_payload);
            if rng.random_range(0.0..1.0) < kc.dup_p {
                push(out, kondor_at + Duration::seconds(rng.random_range(5..90)), Source::Kondor, &kondor_ref, EventKind::New, Some(1), new_payload);
            }
        }
        if d.scenario == Scenario::Duplicate {
            let dup_ref = format!("K{:06}", d.n + 900_000);
            let mut p = kondor_payload(d, "NEW", &cpty, &BTreeMap::new(), kc.rate_dp, d.trader);
            p["deal_id"] = Value::String(dup_ref.clone());
            push(out, kondor_at + Duration::minutes(rng.random_range(1..15)), Source::Kondor, &dup_ref, EventKind::New, Some(1), p);
        }
        if d.scenario == Scenario::Cancel {
            let cancel_at = kondor_at + Duration::minutes(rng.random_range(40..120));
            push(out, cancel_at, Source::Kondor, &kondor_ref, EventKind::Cancel, Some(3), kondor_payload(d, "CANCELLED", &cpty, &refs, kc.rate_dp, d.trader));
        }
    }

    // Quantum back office, downstream of Kondor (or of execution when Kondor never books it).
    let quantum_seen = d.scenario != Scenario::Missing(Source::Quantum);
    if quantum_seen {
        let base = if kondor_seen { kondor_at } else { d.executed_at };
        let mut q_at = base + q_lag.draw(rng);
        if d.scenario == Scenario::Late(Source::Quantum) {
            q_at = base + Duration::minutes(rng.random_range(45..150));
        }
        let cpty = cpty_name(rng, &d.counterparty, qc.cpty_variant_p);
        let names_front = kondor_seen && d.twin_of.is_none() && d.scenario != Scenario::Ambiguous && rng.random_range(0.0..1.0) < qc.cross_ref_p;
        let front_ref = if names_front { Some(kondor_ref.as_str()) } else { None };
        let mismatch = match d.scenario {
            Scenario::Mismatch(f) => Some(f),
            _ => None,
        };
        let pending = quantum_payload(d, "PENDING", &cpty, front_ref, qc.rate_dp, mismatch, cal);
        push(out, q_at, Source::Quantum, &q_no, EventKind::New, Some(1), pending.clone());
        if rng.random_range(0.0..1.0) < qc.dup_p {
            push(out, q_at + Duration::seconds(rng.random_range(10..300)), Source::Quantum, &q_no, EventKind::New, Some(1), pending);
        }
        let confirm_at = q_at + Duration::minutes(rng.random_range(5..90));
        // Mismatches get corrected by the back office later in most cases.
        let corrected = mismatch.is_some() && rng.random_range(0.0..1.0) < 0.6;
        let confirmed = quantum_payload(d, "CONFIRMED", &cpty, front_ref, qc.rate_dp, if corrected { None } else { mismatch }, cal);
        push(out, confirm_at, Source::Quantum, &q_no, EventKind::Status, Some(2), confirmed.clone());
        let settle_at = d.value_date.and_time(NaiveTime::from_hms_opt(16, 0, 0).unwrap()).and_utc() + Duration::minutes(rng.random_range(0..120));
        if settle_at > confirm_at && d.scenario != Scenario::Cancel {
            push(out, settle_at, Source::Quantum, &q_no, EventKind::Status, Some(3), quantum_payload(d, "SETTLED", &cpty, front_ref, qc.rate_dp, if corrected { None } else { mismatch }, cal));
        }
        if d.scenario == Scenario::Cancel && rng.random_range(0.0..1.0) < 0.7 {
            push(out, confirm_at + Duration::minutes(rng.random_range(60..240)), Source::Quantum, &q_no, EventKind::Cancel, Some(4), quantum_payload(d, "CANCELLED", &cpty, front_ref, qc.rate_dp, None, cal));
        }
    }
}

/// Trader identity used for internal bookings, identical to the server's seed.
pub fn user_id(username: &str) -> uuid::Uuid {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, format!("treasury-user:{username}").as_bytes())
}

pub fn is_business_day(d: NaiveDate) -> bool {
    !matches!(d.weekday(), Weekday::Fri | Weekday::Sat)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_parses_and_durations_work() {
        let c = SynthConfig::default();
        assert_eq!(c.world.deals_per_day, 40);
        assert_eq!(parse_duration("2m"), Some(Duration::minutes(2)));
        assert_eq!(parse_duration("30s"), Some(Duration::seconds(30)));
        assert_eq!(parse_duration("1.5h"), Some(Duration::minutes(90)));
        assert_eq!(parse_duration("x"), None);
    }

    #[test]
    fn world_is_deterministic_and_covers_every_scenario() {
        let cal = Calendar::default();
        let mut cfg = SynthConfig::default();
        cfg.world.days = 10;
        let a = World::generate(cfg.clone(), &cal);
        let b = World::generate(cfg, &cal);
        assert_eq!(serde_json::to_string(&a.deals).unwrap(), serde_json::to_string(&b.deals).unwrap());
        let ea = a.emit(&cal);
        let eb = b.emit(&cal);
        assert_eq!(serde_json::to_string(&ea).unwrap(), serde_json::to_string(&eb).unwrap(), "same seed, same feed");

        let has = |p: fn(&Scenario) -> bool| a.deals.iter().any(|d| p(&d.scenario));
        assert!(has(|s| matches!(s, Scenario::Late(_))));
        assert!(has(|s| matches!(s, Scenario::Missing(_))));
        assert!(has(|s| matches!(s, Scenario::Mismatch(_))));
        assert!(has(|s| *s == Scenario::Orphan));
        assert!(has(|s| *s == Scenario::Ambiguous));
        assert!(has(|s| *s == Scenario::Duplicate));
        assert!(has(|s| *s == Scenario::Cancel));
        assert!(ea.windows(2).all(|w| w[0].at() <= w[1].at()), "emissions are time ordered");
        assert!(ea.iter().any(|e| matches!(e, Emission::Internal { .. })));
        assert!(ea.iter().any(|e| matches!(e, Emission::Heartbeat { .. })));
    }
}
