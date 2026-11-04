//! State of the ALM workbench: the saved bank parameters, the unsaved what-if overrides, the
//! snapshot in use, the inputs of each tool and one slot per tool holding its last result
//! pre-formatted for drawing. A generation counter marks which inputs a result was computed for,
//! so the open tool recomputes only when something it depends on changed.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use domain::{
    AlmOverview, AlmParams, AlmParamsResponse, AlmParamsVersion, AlmRequest, AlmSnapshot, AttributionResult, CompareTo, FundingResult, GapResult, LcrFigures,
    LcrResult, MarketResponse, NiiResult, NsfrResult, RateScenario, StressResult,
};

use crate::state::Fetched;
use crate::ui::fmt::{fmt_money_short, fmt_pct};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlmTool {
    #[default]
    Dashboard,
    Lcr,
    Nsfr,
    Nii,
    Stress,
    Attribution,
    Ird,
    Gap,
    Funding,
    Setup,
}

impl AlmTool {
    pub const ALL: [AlmTool; 10] = [
        AlmTool::Dashboard,
        AlmTool::Lcr,
        AlmTool::Nsfr,
        AlmTool::Nii,
        AlmTool::Stress,
        AlmTool::Attribution,
        AlmTool::Ird,
        AlmTool::Gap,
        AlmTool::Funding,
        AlmTool::Setup,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AlmTool::Dashboard => "Dashboard",
            AlmTool::Lcr => "LCR",
            AlmTool::Nsfr => "NSFR",
            AlmTool::Nii => "NII / NIM",
            AlmTool::Stress => "Stress",
            AlmTool::Attribution => "Attribution",
            AlmTool::Ird => "IRD",
            AlmTool::Gap => "Gap",
            AlmTool::Funding => "Funding",
            AlmTool::Setup => "Setup",
        }
    }

    pub fn from_name(name: &str) -> Option<AlmTool> {
        AlmTool::ALL.iter().copied().find(|t| t.label().to_lowercase().starts_with(&name.to_lowercase()) || format!("{t:?}").to_lowercase() == name.to_lowercase())
    }

    /// Tools whose result comes from the server.
    pub fn computed(self) -> bool {
        matches!(self, AlmTool::Lcr | AlmTool::Nsfr | AlmTool::Nii | AlmTool::Stress | AlmTool::Attribution | AlmTool::Gap | AlmTool::Funding)
    }
}

/// A server result, one variant per tool.
#[derive(Debug)]
pub enum AlmPayload {
    Lcr(LcrResult),
    Nsfr(NsfrResult),
    Gap(GapResult),
    Nii(NiiResult),
    Stress(StressResult),
    Attribution(AttributionResult),
    Funding(FundingResult),
}

/// How a headline compares with the bank's thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    Comfortable,
    Watch,
    Breach,
    Unknown,
}

pub fn standing(ratio: Option<f64>, minimum: f64, internal: f64, warning: f64) -> Standing {
    match ratio {
        None => Standing::Unknown,
        Some(r) if r < minimum.max(internal) => Standing::Breach,
        Some(r) if r < warning => Standing::Watch,
        Some(_) => Standing::Comfortable,
    }
}

fn pct_or_dash(v: Option<f64>) -> String {
    v.map(|r| fmt_pct(r, 1)).unwrap_or_else(|| "—".into())
}

fn minus(v: f64) -> String {
    if v > 0.5 { format!("-{}", fmt_money_short(v)) } else { "0".into() }
}

fn signed(v: f64) -> String {
    if v >= 0.0 { format!("+{}", fmt_money_short(v)) } else { fmt_money_short(v) }
}

fn basis_label(version: Option<i32>) -> String {
    match version {
        Some(v) => format!("bank parameters v{v}"),
        None => "unsaved what-if".into(),
    }
}

fn nice(class: &str) -> String {
    class.replace('_', " ").to_lowercase()
}

// ---------------------------------------------------------------------------------------------
// Views: everything the draw code needs, formatted once.
// ---------------------------------------------------------------------------------------------

pub struct LcrView {
    pub raw: LcrResult,
    pub headline: String,
    pub standing: Standing,
    pub basis: String,
    pub hqla_rows: Vec<(String, String)>,
    pub outflow_rows: Vec<[String; 4]>,
    pub inflow_rows: Vec<[String; 4]>,
    pub ccy_rows: Vec<[String; 4]>,
    pub totals: Vec<(String, String)>,
}

impl LcrView {
    pub fn new(raw: LcrResult) -> Self {
        let t: &LcrFigures = &raw.total;
        let lines = |rows: &[domain::LcrLine]| -> Vec<[String; 4]> {
            rows.iter().map(|l| [nice(&l.class), fmt_money_short(l.amount), fmt_pct(l.rate, 0), fmt_money_short(l.weighted)]).collect()
        };
        Self {
            headline: pct_or_dash(t.ratio),
            standing: standing(t.ratio, raw.minimum, raw.internal_minimum, raw.early_warning),
            basis: basis_label(raw.params_version),
            hqla_rows: vec![
                ("Level 1".into(), fmt_money_short(t.hqla_l1)),
                ("Level 2A after haircut".into(), fmt_money_short(t.hqla_l2a)),
                ("Level 2B after haircut".into(), fmt_money_short(t.hqla_l2b)),
                ("Level 2B cap adjustment".into(), minus(t.adjustment_l2b_cap)),
                ("Level 2 cap adjustment".into(), minus(t.adjustment_l2_cap)),
                ("Stock of liquid assets".into(), fmt_money_short(t.stock)),
            ],
            totals: vec![
                ("Total outflows".into(), fmt_money_short(t.total_outflows)),
                ("Total inflows".into(), fmt_money_short(t.total_inflows)),
                ("Inflows after cap".into(), fmt_money_short(t.capped_inflows)),
                ("Net cash outflows".into(), fmt_money_short(t.net_outflows)),
            ],
            ccy_rows: raw.by_ccy.iter().map(|c| [c.ccy.clone(), fmt_money_short(c.stock), fmt_money_short(c.net_outflows), pct_or_dash(c.ratio)]).collect(),
            outflow_rows: lines(&t.outflows),
            inflow_rows: lines(&t.inflows),
            raw,
        }
    }
}

pub struct NsfrView {
    pub raw: NsfrResult,
    pub headline: String,
    pub standing: Standing,
    pub basis: String,
    pub asf_rows: Vec<[String; 4]>,
    pub rsf_rows: Vec<[String; 4]>,
    pub totals: Vec<(String, String)>,
}

impl NsfrView {
    pub fn new(raw: NsfrResult) -> Self {
        let rows = |l: &[domain::NsfrLine]| -> Vec<[String; 4]> {
            l.iter().map(|x| [format!("{} · {}", nice(&x.class), x.bucket), fmt_money_short(x.amount), fmt_pct(x.factor, 0), fmt_money_short(x.weighted)]).collect()
        };
        Self {
            headline: pct_or_dash(raw.ratio),
            standing: standing(raw.ratio, raw.minimum, raw.internal_minimum, raw.internal_minimum),
            basis: basis_label(raw.params_version),
            asf_rows: rows(&raw.asf_lines),
            rsf_rows: rows(&raw.rsf_lines),
            totals: vec![("Available stable funding".into(), fmt_money_short(raw.asf)), ("Required stable funding".into(), fmt_money_short(raw.rsf))],
            raw,
        }
    }
}

pub struct GapView {
    pub raw: GapResult,
    pub basis: String,
    /// label, assets, liabilities, gap, cumulative, cumulative % (limit)
    pub rows: Vec<[String; 6]>,
    pub gap_bars: Vec<(String, f64)>,
    pub cumulative_bars: Vec<(String, f64)>,
    pub breaches: Vec<String>,
    pub ccy_rows: Vec<[String; 3]>,
}

impl GapView {
    pub fn new(raw: GapResult) -> Self {
        let rows = raw
            .buckets
            .iter()
            .map(|b| {
                [
                    b.label.clone(),
                    fmt_money_short(b.assets),
                    fmt_money_short(b.liabilities),
                    signed(b.gap),
                    signed(b.cumulative),
                    match b.limit_pct {
                        Some(l) => format!("{:.1}% of {:.0}%", b.cumulative_pct_assets, l),
                        None => format!("{:.1}%", b.cumulative_pct_assets),
                    },
                ]
            })
            .collect();
        let breaches = raw.buckets.iter().filter(|b| b.breach).map(|b| format!("{}: {:.1}% against {:.0}%", b.label, b.cumulative_pct_assets, b.limit_pct.unwrap_or(0.0))).collect();
        let ccy_rows = raw
            .by_ccy
            .iter()
            .map(|c| {
                let y1 = c.buckets.iter().find(|b| b.label == "1Y");
                [c.ccy.clone(), y1.map(|b| signed(b.cumulative)).unwrap_or_default(), y1.map(|b| format!("{:.1}%", b.cumulative_pct_assets)).unwrap_or_default()]
            })
            .collect();
        Self {
            basis: basis_label(raw.params_version),
            rows,
            gap_bars: raw.buckets.iter().map(|b| (b.label.clone(), b.gap)).collect(),
            cumulative_bars: raw.buckets.iter().map(|b| (b.label.clone(), b.cumulative)).collect(),
            breaches,
            ccy_rows,
            raw,
        }
    }
}

pub struct NiiView {
    pub raw: NiiResult,
    pub headline: String,
    pub nim: String,
    pub basis: String,
    pub delta: Option<String>,
    pub month_bars: Vec<(String, f64)>,
    pub rows: Vec<[String; 4]>,
    pub by_book: Vec<[String; 2]>,
    pub by_ccy: Vec<[String; 2]>,
}

impl NiiView {
    pub fn new(raw: NiiResult) -> Self {
        Self {
            headline: fmt_money_short(raw.nii),
            nim: raw.nim.map(|n| format!("{n:.2}%")).unwrap_or_else(|| "—".into()),
            basis: format!("{} · {}", raw.scenario.label(), basis_label(raw.params_version)),
            delta: raw.base_nii.map(|b| format!("{} against the base ({})", signed(raw.nii - b), fmt_money_short(b))),
            month_bars: raw.months.iter().map(|m| (m.period.format("%b %y").to_string(), m.nii)).collect(),
            rows: raw.months.iter().map(|m| [m.period.format("%b %Y").to_string(), fmt_money_short(m.income), fmt_money_short(m.expense), fmt_money_short(m.nii)]).collect(),
            by_book: raw.by_book.iter().map(|b| [nice(&b.key), signed(b.nii)]).collect(),
            by_ccy: raw.by_ccy.iter().map(|b| [b.key.clone(), signed(b.nii)]).collect(),
            raw,
        }
    }
}

pub struct StressView {
    pub raw: StressResult,
    pub basis: String,
    /// scenario, ΔNII, ΔNII %, ΔEVE, EVE loss, % Tier 1, flags
    pub rows: Vec<[String; 7]>,
    pub nii_bars: Vec<(String, f64)>,
    pub eve_bars: Vec<(String, f64)>,
    pub liquidity_rows: Vec<(String, String)>,
    pub survival: Vec<[f64; 2]>,
    pub stock: f64,
    pub fx_rows: Vec<[String; 4]>,
    pub breaches: Vec<String>,
}

impl StressView {
    pub fn new(raw: StressResult) -> Self {
        let rows = raw
            .rate_scenarios
            .iter()
            .map(|s| {
                [
                    s.label.clone(),
                    signed(s.nii_delta),
                    s.nii_delta_pct.map(|p| format!("{p:+.1}%")).unwrap_or_else(|| "—".into()),
                    signed(s.eve_delta),
                    fmt_money_short(s.eve_loss),
                    s.eve_loss_pct_tier1.map(|p| format!("{p:.1}%")).unwrap_or_else(|| "—".into()),
                    match (s.nii_breach, s.eve_breach) {
                        (true, true) => "NII and EVE limits".into(),
                        (true, false) => "NII limit".into(),
                        (false, true) => "EVE limit".into(),
                        _ => String::new(),
                    },
                ]
            })
            .collect();
        let l = &raw.liquidity;
        let liquidity_rows = vec![
            ("LCR today".into(), pct_or_dash(l.lcr_base)),
            ("LCR under stress".into(), pct_or_dash(l.lcr_stressed)),
            ("Stock after extra haircut".into(), fmt_money_short(l.stock_stressed)),
            ("Stressed net outflows".into(), fmt_money_short(l.net_outflows_stressed)),
            ("Survival".into(), l.survival_days.map(|d| format!("{d} days")).unwrap_or_else(|| "beyond 90 days".into())),
        ];
        let mut breaches: Vec<String> = raw.rate_scenarios.iter().filter(|s| s.nii_breach || s.eve_breach).map(|s| s.label.clone()).collect();
        if let Some(d) = l.survival_days {
            breaches.push(format!("liquid assets exhausted on day {d} of the liquidity stress"));
        }
        Self {
            basis: basis_label(raw.params_version),
            rows,
            nii_bars: raw.rate_scenarios.iter().map(|s| (short_label(s.scenario), s.nii_delta)).collect(),
            eve_bars: raw.rate_scenarios.iter().map(|s| (short_label(s.scenario), s.eve_delta)).collect(),
            liquidity_rows,
            survival: l.horizon.iter().map(|p| [p.day as f64, p.cumulative_net_outflow]).collect(),
            stock: l.stock_stressed,
            fx_rows: raw.fx.iter().map(|f| [f.ccy.clone(), format!("-{:.0}%", f.shock_pct), signed(f.exposure), signed(f.pnl)]).collect(),
            breaches,
            raw,
        }
    }
}

fn short_label(s: RateScenario) -> String {
    match s {
        RateScenario::Base => "Base",
        RateScenario::ParallelUp => "Par +",
        RateScenario::ParallelDown => "Par −",
        RateScenario::Steepener => "Steep",
        RateScenario::Flattener => "Flat",
        RateScenario::ShortUp => "Short +",
        RateScenario::ShortDown => "Short −",
        RateScenario::Custom => "Custom",
    }
    .into()
}

pub struct AttributionView {
    pub raw: AttributionResult,
    pub headline: String,
    pub subtitle: String,
    /// label, value, is a total
    pub steps: Vec<(String, f64, bool)>,
    pub effect_rows: Vec<(String, String)>,
    /// book, side, ccy, NII A, NII B, rate A, rate B, change
    pub rows: Vec<[String; 8]>,
}

impl AttributionView {
    pub fn new(raw: AttributionResult) -> Self {
        let n = raw.steps.len();
        let steps = raw.steps.iter().enumerate().map(|(i, s)| (s.label.clone(), s.value, i == 0 || i + 1 == n)).collect();
        let effect_rows = vec![
            ("Days".into(), signed(raw.days_effect)),
            ("Pure volume".into(), signed(raw.pure_volume_effect)),
            ("Mix".into(), signed(raw.mix_effect)),
            ("Rate".into(), signed(raw.rate_effect)),
            ("Residual".into(), signed(raw.residual)),
            ("Change".into(), signed(raw.delta)),
        ];
        let rows = raw
            .lines
            .iter()
            .map(|l| {
                [
                    nice(&l.book),
                    l.side.as_str().to_lowercase(),
                    l.ccy.clone(),
                    signed(l.nii_a),
                    signed(l.nii_b),
                    format!("{:.2}%", l.rate_a),
                    format!("{:.2}%", l.rate_b),
                    signed(l.nii_b - l.nii_a),
                ]
            })
            .collect();
        Self {
            headline: signed(raw.delta),
            subtitle: format!(
                "{} against {} {} · in {}",
                raw.period_b.format("%B %Y"),
                match raw.compare_to {
                    CompareTo::Prior => "the prior month,",
                    CompareTo::Budget => "the budget for",
                },
                raw.period_a.format("%B %Y"),
                raw.reporting_ccy
            ),
            steps,
            effect_rows,
            rows,
            raw,
        }
    }
}

pub struct FundingView {
    pub raw: FundingResult,
    pub basis: String,
    pub headline_rows: Vec<(String, String)>,
    /// name, segment, balance, share
    pub group_rows: Vec<[String; 4]>,
    pub group_bars: Vec<(String, f64)>,
    pub segment_rows: Vec<[String; 2]>,
}

impl FundingView {
    pub fn new(raw: FundingResult) -> Self {
        Self {
            basis: basis_label(raw.params_version),
            headline_rows: vec![
                ("Largest depositor".into(), format!("{:.1}% (limit {:.0}%)", raw.top1_pct, raw.top1_limit_pct)),
                ("Ten largest depositors".into(), format!("{:.1}% (limit {:.0}%)", raw.top10_pct, raw.top10_limit_pct)),
                ("Wholesale funding".into(), format!("{:.1}% (limit {:.0}%)", raw.wholesale_pct, raw.wholesale_limit_pct)),
                ("Deposits".into(), fmt_money_short(raw.total_deposits)),
                ("Total funding".into(), fmt_money_short(raw.total_funding)),
            ],
            group_rows: raw.groups.iter().map(|g| [g.name.clone(), nice(&g.segment), fmt_money_short(g.balance), format!("{:.2}%", g.share_pct)]).collect(),
            group_bars: raw.groups.iter().take(10).map(|g| (g.name.replace("Government entity", "Govt").replace("Large corporate", "Corp").replace("Financial institution", "FI").replace("High net worth group", "HNW"), g.share_pct)).collect(),
            segment_rows: raw.by_segment.iter().map(|s| [nice(&s.key), fmt_money_short(s.nii)]).collect(),
            raw,
        }
    }
}

/// The overview's ALM block, formatted.
pub struct OverviewAlmView {
    pub raw: AlmOverview,
    pub tiles: Vec<(String, String, String, Standing)>,
    /// metric, value, limit, gauge value, gauge limit, gauge trigger, gauge max, breach, warn
    pub limit_rows: Vec<LimitView>,
    pub lcr_trend: Vec<[f64; 2]>,
    pub lcr_trend_labels: Vec<String>,
    pub nii_months: Vec<(String, f64, f64)>,
    pub gap_bars: Vec<(String, f64)>,
}

pub struct LimitView {
    pub metric: String,
    pub value: String,
    pub limit: String,
    pub gauge_value: f64,
    pub gauge_limit: f64,
    pub gauge_trigger: Option<f64>,
    pub gauge_max: f64,
    pub floor: bool,
    pub breach: bool,
    pub warn: bool,
}

impl OverviewAlmView {
    pub fn new(raw: AlmOverview) -> Self {
        let st = |b: bool, w: bool| if b { Standing::Breach } else if w { Standing::Watch } else { Standing::Comfortable };
        let lim = |m: &str| raw.limits.iter().find(|l| l.metric.starts_with(m));
        let lcr_l = lim("LCR");
        let nsfr_l = lim("NSFR");
        let budget = |a: f64, b: f64| if b.abs() > 1e-9 { format!("{:+.1}% vs budget {}", (a - b) / b.abs() * 100.0, fmt_money_short(b)) } else { "no budget".into() };
        let tiles = vec![
            ("LCR".into(), pct_or_dash(raw.lcr), format!("internal minimum {}", lcr_l.map(|l| format!("{:.0}%", l.limit)).unwrap_or_default()), st(lcr_l.is_some_and(|l| l.breach), lcr_l.is_some_and(|l| l.warn))),
            ("NSFR".into(), pct_or_dash(raw.nsfr), format!("internal minimum {}", nsfr_l.map(|l| format!("{:.0}%", l.limit)).unwrap_or_default()), st(nsfr_l.is_some_and(|l| l.breach), nsfr_l.is_some_and(|l| l.warn))),
            ("NII next 12 months".into(), fmt_money_short(raw.nii_12m), format!("NIM {}", raw.nim.map(|n| format!("{n:.2}%")).unwrap_or_else(|| "—".into())), Standing::Comfortable),
            ("NII month to date".into(), fmt_money_short(raw.nii_mtd), budget(raw.nii_mtd, raw.budget_mtd), if raw.nii_mtd < raw.budget_mtd * 0.95 { Standing::Watch } else { Standing::Comfortable }),
            ("NII year to date".into(), fmt_money_short(raw.nii_ytd), budget(raw.nii_ytd, raw.budget_ytd), if raw.nii_ytd < raw.budget_ytd * 0.95 { Standing::Watch } else { Standing::Comfortable }),
            ("Cumulative gap 1Y".into(), format!("{:+.1}%", raw.gap_1y_pct), "of total assets".into(), st(lim("Cumulative").is_some_and(|l| l.breach), lim("Cumulative").is_some_and(|l| l.warn))),
            (
                "Worst EVE scenario".into(),
                raw.worst_eve_loss_pct_tier1.map(|p| format!("{p:.1}%")).unwrap_or_else(|| "—".into()),
                raw.worst_eve_scenario.map(|s| format!("of Tier 1 · {}", s.label())).unwrap_or_else(|| "no loss in any scenario".into()),
                st(lim("EVE").is_some_and(|l| l.breach), lim("EVE").is_some_and(|l| l.warn)),
            ),
            ("Ten largest depositors".into(), format!("{:.1}%", raw.top10_pct), format!("wholesale funding {:.1}%", raw.wholesale_pct), st(lim("Ten").is_some_and(|l| l.breach), lim("Ten").is_some_and(|l| l.warn))),
        ];
        let limit_rows = raw
            .limits
            .iter()
            .map(|l| {
                let v = l.value.unwrap_or(0.0);
                let max = v.max(l.limit).max(l.trigger.unwrap_or(0.0)) * 1.2;
                LimitView {
                    metric: l.metric.clone(),
                    value: l.value.map(|x| format!("{x:.1}{}", l.unit)).unwrap_or_else(|| "—".into()),
                    limit: format!("{}{:.0}{}", if l.floor { "≥ " } else { "≤ " }, l.limit, l.unit),
                    gauge_value: v,
                    gauge_limit: l.limit,
                    gauge_trigger: l.trigger,
                    gauge_max: max.max(1.0),
                    floor: l.floor,
                    breach: l.breach,
                    warn: l.warn,
                }
            })
            .collect();
        let lcr_trend = raw.lcr_trend.iter().enumerate().filter_map(|(i, p)| p.value.map(|v| [i as f64, v])).collect();
        let lcr_trend_labels = raw.lcr_trend.iter().map(|p| p.as_of.format("%d %b").to_string()).collect();
        Self {
            tiles,
            limit_rows,
            lcr_trend,
            lcr_trend_labels,
            nii_months: raw.nii_months.iter().map(|m| (m.period.format("%b").to_string(), m.actual, m.budget)).collect(),
            gap_bars: raw.gap_buckets.iter().map(|b| (b.label.clone(), b.gap)).collect(),
            raw,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The interest rate differential calculator runs here, on the latest curves.
// ---------------------------------------------------------------------------------------------

pub struct IrdState {
    pub base_ccy: String,
    pub quote_ccy: String,
    pub spot: f64,
    pub days: i64,
    pub base_rate: f64,
    pub quote_rate: f64,
    pub notional: f64,
    pub forward_input: f64,
    pub follow_market: bool,
    pub rates: Fetched<MarketResponse>,
    pub rates_as_of: String,
}

impl Default for IrdState {
    fn default() -> Self {
        Self {
            base_ccy: "USD".into(),
            quote_ccy: "BHD".into(),
            spot: 0.376,
            days: 90,
            base_rate: 4.3,
            quote_rate: 5.9,
            notional: 10_000_000.0,
            forward_input: 0.0,
            follow_market: true,
            rates: Fetched::default(),
            rates_as_of: String::new(),
        }
    }
}

impl IrdState {
    /// Spot of base/quote from the BHD-quoted spots (a cross is the ratio of the two).
    pub fn market_spot(&self, m: &MarketResponse) -> Option<f64> {
        let bhd_per = |ccy: &str| -> Option<f64> { if ccy == "BHD" { Some(1.0) } else { m.spots.iter().find(|s| s.pair == format!("{ccy}/BHD")).map(|s| s.mid) } };
        Some(bhd_per(&self.base_ccy)? / bhd_per(&self.quote_ccy)?)
    }

    pub fn market_rate(&self, m: &MarketResponse, ccy: &str, days: i64) -> Option<f64> {
        let c = m.curves.iter().find(|c| c.ccy == ccy)?;
        Some(alm::curve::Curve::from_rate_curve(c).rate(days as f64 / 365.0))
    }

    /// Pull spot and rates for the current tenor from the market when following it.
    pub fn refresh_from_market(&mut self) {
        if !self.follow_market {
            return;
        }
        let Some(m) = &self.rates.view else { return };
        if let Some(s) = self.market_spot(m) {
            self.spot = s;
        }
        if let Some(r) = self.market_rate(m, &self.base_ccy, self.days) {
            self.base_rate = r;
        }
        if let Some(r) = self.market_rate(m, &self.quote_ccy, self.days) {
            self.quote_rate = r;
        }
    }
}

pub struct SetupState {
    pub draft: Option<AlmParams>,
    pub note: String,
    pub saving: Fetched<AlmParamsVersion>,
}

impl Default for SetupState {
    fn default() -> Self {
        Self { draft: None, note: String::new(), saving: Fetched::default() }
    }
}

const DEBOUNCE: Duration = Duration::from_millis(300);

pub struct ToolSlot<V> {
    pub result: Fetched<V>,
    /// The inputs generation the result was computed for.
    pub generation: u64,
}

impl<V> Default for ToolSlot<V> {
    fn default() -> Self {
        Self { result: Fetched::default(), generation: 0 }
    }
}

#[derive(Default)]
pub struct AlmState {
    pub tool: AlmTool,
    pub snapshots: Fetched<Vec<AlmSnapshot>>,
    pub as_of: Option<NaiveDate>,
    pub params: Fetched<AlmParamsResponse>,
    /// The saved parameters, shared with the result views.
    pub bank: Option<Arc<AlmParams>>,
    pub bank_version: i32,
    /// Unsaved what-if; `None` means the bank parameters.
    pub overrides: Option<AlmParams>,
    /// A newer saved version arrived while overrides were active.
    pub bank_changed: bool,
    // tool inputs
    pub scenario: RateScenario,
    pub custom_bp: f64,
    pub compare_to: Option<CompareTo>,
    pub period: Option<NaiveDate>,
    // results
    pub lcr: ToolSlot<LcrView>,
    pub nsfr: ToolSlot<NsfrView>,
    pub gap: ToolSlot<GapView>,
    pub nii: ToolSlot<NiiView>,
    pub stress: ToolSlot<StressView>,
    pub attribution: ToolSlot<AttributionView>,
    pub funding: ToolSlot<FundingView>,
    pub overview: ToolSlot<OverviewAlmView>,
    pub ird: IrdState,
    pub setup: SetupState,
    /// Bumped whenever an input changed; results carry the generation they were computed for.
    pub generation: u64,
    changed_at: Option<Instant>,
}

impl AlmState {
    /// A copy of the parameters a tool runs with: the what-if when one exists, else the bank's.
    /// Callers store it back as the what-if only when something changed.
    pub fn working(&self) -> Option<AlmParams> {
        match &self.overrides {
            Some(o) => Some(o.clone()),
            None => self.bank.as_ref().map(|b| (**b).clone()),
        }
    }

    pub fn reset_overrides(&mut self) {
        self.overrides = None;
        self.bank_changed = false;
        self.touch();
    }

    /// Something a result depends on changed.
    pub fn touch(&mut self) {
        self.generation += 1;
        self.changed_at.get_or_insert_with(Instant::now);
    }

    /// Whether the open tool should be recomputed now: its result is for an older generation and
    /// edits have settled.
    pub fn due(&mut self) -> bool {
        let computed_for = match self.tool {
            AlmTool::Lcr => self.lcr.generation,
            AlmTool::Nsfr => self.nsfr.generation,
            AlmTool::Gap => self.gap.generation,
            AlmTool::Nii => self.nii.generation,
            AlmTool::Stress => self.stress.generation,
            AlmTool::Attribution => self.attribution.generation,
            AlmTool::Funding => self.funding.generation,
            _ => return false,
        };
        if computed_for == self.generation {
            return false;
        }
        match self.changed_at {
            Some(at) if at.elapsed() < DEBOUNCE => false,
            _ => {
                self.changed_at = None;
                true
            }
        }
    }

    pub fn is_pending(&self) -> bool {
        self.changed_at.is_some()
    }

    pub fn request(&self) -> AlmRequest {
        AlmRequest {
            as_of: self.as_of,
            params: self.overrides.clone(),
            scenario: Some(self.scenario),
            custom_shock_bp: if self.tool == AlmTool::Stress || self.scenario == RateScenario::Custom { Some(self.custom_bp) } else { None },
            compare_to: self.compare_to,
            period: self.period,
        }
    }

    /// Mark the open tool's slot as in flight for the current generation.
    pub fn begin(&mut self, tool: AlmTool, req: u64) {
        let g = self.generation;
        match tool {
            AlmTool::Lcr => {
                self.lcr.result.begin(req);
                self.lcr.generation = g;
            }
            AlmTool::Nsfr => {
                self.nsfr.result.begin(req);
                self.nsfr.generation = g;
            }
            AlmTool::Gap => {
                self.gap.result.begin(req);
                self.gap.generation = g;
            }
            AlmTool::Nii => {
                self.nii.result.begin(req);
                self.nii.generation = g;
            }
            AlmTool::Stress => {
                self.stress.result.begin(req);
                self.stress.generation = g;
            }
            AlmTool::Attribution => {
                self.attribution.result.begin(req);
                self.attribution.generation = g;
            }
            AlmTool::Funding => {
                self.funding.result.begin(req);
                self.funding.generation = g;
            }
            _ => {}
        }
    }

    pub fn accept(&mut self, tool: AlmTool, req: u64, result: Result<AlmPayload, String>) {
        match (tool, result) {
            (AlmTool::Lcr, Ok(AlmPayload::Lcr(r))) => {
                self.lcr.result.accept(req, Ok(LcrView::new(r)));
            }
            (AlmTool::Nsfr, Ok(AlmPayload::Nsfr(r))) => {
                self.nsfr.result.accept(req, Ok(NsfrView::new(r)));
            }
            (AlmTool::Gap, Ok(AlmPayload::Gap(r))) => {
                self.gap.result.accept(req, Ok(GapView::new(r)));
            }
            (AlmTool::Nii, Ok(AlmPayload::Nii(r))) => {
                self.nii.result.accept(req, Ok(NiiView::new(r)));
            }
            (AlmTool::Stress, Ok(AlmPayload::Stress(r))) => {
                self.stress.result.accept(req, Ok(StressView::new(r)));
            }
            (AlmTool::Attribution, Ok(AlmPayload::Attribution(r))) => {
                self.attribution.result.accept(req, Ok(AttributionView::new(r)));
            }
            (AlmTool::Funding, Ok(AlmPayload::Funding(r))) => {
                self.funding.result.accept(req, Ok(FundingView::new(r)));
            }
            (AlmTool::Lcr, Err(e)) => {
                self.lcr.result.accept(req, Err(e));
            }
            (AlmTool::Nsfr, Err(e)) => {
                self.nsfr.result.accept(req, Err(e));
            }
            (AlmTool::Gap, Err(e)) => {
                self.gap.result.accept(req, Err(e));
            }
            (AlmTool::Nii, Err(e)) => {
                self.nii.result.accept(req, Err(e));
            }
            (AlmTool::Stress, Err(e)) => {
                self.stress.result.accept(req, Err(e));
            }
            (AlmTool::Attribution, Err(e)) => {
                self.attribution.result.accept(req, Err(e));
            }
            (AlmTool::Funding, Err(e)) => {
                self.funding.result.accept(req, Err(e));
            }
            _ => {}
        }
    }
}
