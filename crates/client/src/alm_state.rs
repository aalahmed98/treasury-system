//! State of the ALM workbench: the saved bank parameters, the unsaved what-if overrides, the
//! snapshot in use and one slot per tool holding its last result pre-formatted for drawing.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use domain::{AlmParams, AlmParamsResponse, AlmSnapshot, LcrFigures, LcrResult};

use crate::state::Fetched;
use crate::ui::fmt::{fmt_money_short, fmt_pct};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlmTool {
    Dashboard,
    #[default]
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
}

/// How a headline compares with the bank's thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    Comfortable,
    Watch,
    Breach,
    Unknown,
}

pub struct LcrView {
    pub raw: LcrResult,
    pub headline: String,
    pub standing: Standing,
    pub basis: String,
    /// label, amount
    pub hqla_rows: Vec<(String, String)>,
    /// class, amount, rate, weighted
    pub outflow_rows: Vec<[String; 4]>,
    pub inflow_rows: Vec<[String; 4]>,
    /// ccy, stock, net outflows, ratio
    pub ccy_rows: Vec<[String; 4]>,
    pub totals: Vec<(String, String)>,
}

fn standing(ratio: Option<f64>, minimum: f64, internal: f64, warning: f64) -> Standing {
    match ratio {
        None => Standing::Unknown,
        Some(r) if r < minimum.max(internal) => Standing::Breach,
        Some(r) if r < warning => Standing::Watch,
        Some(_) => Standing::Comfortable,
    }
}

fn minus(v: f64) -> String {
    if v > 0.5 { format!("-{}", fmt_money_short(v)) } else { "0".into() }
}

fn pct_or_dash(v: Option<f64>) -> String {
    v.map(|r| fmt_pct(r, 1)).unwrap_or_else(|| "—".into())
}

fn lines(rows: &[domain::LcrLine]) -> Vec<[String; 4]> {
    rows.iter().map(|l| [l.class.replace('_', " ").to_lowercase(), fmt_money_short(l.amount), fmt_pct(l.rate, 0), fmt_money_short(l.weighted)]).collect()
}

impl LcrView {
    pub fn new(raw: LcrResult) -> Self {
        let t: &LcrFigures = &raw.total;
        let headline = pct_or_dash(t.ratio);
        let st = standing(t.ratio, raw.minimum, raw.internal_minimum, raw.early_warning);
        let basis = match raw.params_version {
            Some(v) => format!("bank parameters v{v}"),
            None => "unsaved what-if".into(),
        };
        let hqla_rows = vec![
            ("Level 1".into(), fmt_money_short(t.hqla_l1)),
            ("Level 2A after haircut".into(), fmt_money_short(t.hqla_l2a)),
            ("Level 2B after haircut".into(), fmt_money_short(t.hqla_l2b)),
            ("Level 2B cap adjustment".into(), minus(t.adjustment_l2b_cap)),
            ("Level 2 cap adjustment".into(), minus(t.adjustment_l2_cap)),
            ("Stock of liquid assets".into(), fmt_money_short(t.stock)),
        ];
        let totals = vec![
            ("Total outflows".into(), fmt_money_short(t.total_outflows)),
            ("Total inflows".into(), fmt_money_short(t.total_inflows)),
            ("Inflows after cap".into(), fmt_money_short(t.capped_inflows)),
            ("Net cash outflows".into(), fmt_money_short(t.net_outflows)),
        ];
        let ccy_rows = raw.by_ccy.iter().map(|c| [c.ccy.clone(), fmt_money_short(c.stock), fmt_money_short(c.net_outflows), pct_or_dash(c.ratio)]).collect();
        Self { headline, standing: st, basis, hqla_rows, outflow_rows: lines(&t.outflows), inflow_rows: lines(&t.inflows), ccy_rows, totals, raw }
    }
}

const DEBOUNCE: Duration = Duration::from_millis(300);

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
    pub lcr: Fetched<LcrView>,
    lcr_dirty: Option<Instant>,
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
        self.mark_dirty();
    }

    pub fn mark_dirty(&mut self) {
        self.lcr_dirty.get_or_insert_with(Instant::now);
    }

    /// True once a change has settled long enough to recompute.
    pub fn take_due(&mut self) -> bool {
        match self.lcr_dirty {
            Some(at) if at.elapsed() >= DEBOUNCE => {
                self.lcr_dirty = None;
                true
            }
            _ => false,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.lcr_dirty.is_some()
    }

    pub fn request(&self) -> domain::AlmRequest {
        domain::AlmRequest { as_of: self.as_of, params: self.overrides.clone() }
    }
}
