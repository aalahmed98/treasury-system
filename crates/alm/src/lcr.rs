//! Liquidity coverage ratio: the stock of high-quality liquid assets over net cash outflows in the
//! next thirty days, per currency and for the whole book in the reporting currency, with the
//! Basel caps on Level 2 assets and on inflows applied the way the bank's parameters say.

use std::collections::BTreeMap;

use chrono::{Duration, NaiveDate};
use domain::{AlmParams, BsPosition, BsSide, HqlaLevel, LcrFigures, LcrLine, LcrResult};

use crate::schedule::due_within;

/// Spot rates as units of the reporting currency per one unit of each currency.
pub type Spots = BTreeMap<String, f64>;

struct Raw {
    l1: f64,
    l2a: f64,
    l2b: f64,
    outflows: BTreeMap<String, (f64, f64)>,
    inflows: BTreeMap<String, (f64, f64)>,
}

impl Raw {
    fn new() -> Self {
        Self { l1: 0.0, l2a: 0.0, l2b: 0.0, outflows: BTreeMap::new(), inflows: BTreeMap::new() }
    }

    fn add(&mut self, other: &Raw, fx: f64) {
        self.l1 += other.l1 * fx;
        self.l2a += other.l2a * fx;
        self.l2b += other.l2b * fx;
        for (k, (a, w)) in &other.outflows {
            let e = self.outflows.entry(k.clone()).or_insert((0.0, 0.0));
            e.0 += a * fx;
            e.1 += w * fx;
        }
        for (k, (a, w)) in &other.inflows {
            let e = self.inflows.entry(k.clone()).or_insert((0.0, 0.0));
            e.0 += a * fx;
            e.1 += w * fx;
        }
    }
}

/// The adjustment that keeps Level 2B under its cap and Level 2 under its cap, from amounts
/// already after haircuts. Caps are percentages of the stock.
pub fn cap_adjustments(l1: f64, l2a: f64, l2b: f64, cap_l2_pct: f64, cap_l2b_pct: f64) -> (f64, f64) {
    let c2 = cap_l2_pct.clamp(0.0, 99.999);
    let c2b = cap_l2b_pct.clamp(0.0, 99.999);
    let adj_l2b = (l2b - c2b / (100.0 - c2b) * (l1 + l2a)).max(l2b - c2b / (100.0 - c2) * l1).max(0.0);
    let adj_l2 = ((l2a + l2b - adj_l2b) - c2 / (100.0 - c2) * l1).max(0.0);
    (adj_l2b, adj_l2)
}

fn figures(ccy: &str, raw: &Raw, params: &AlmParams) -> LcrFigures {
    let p = &params.lcr;
    let (adj_l2b, adj_l2) = cap_adjustments(raw.l1, raw.l2a, raw.l2b, p.cap_l2_total_pct, p.cap_l2b_pct);
    let stock = raw.l1 + raw.l2a + raw.l2b - adj_l2b - adj_l2;
    let line = |(class, (amount, weighted)): (&String, &(f64, f64))| LcrLine {
        class: class.clone(),
        amount: *amount,
        rate: if *amount > 0.0 { weighted / amount * 100.0 } else { 0.0 },
        weighted: *weighted,
    };
    let outflows: Vec<LcrLine> = raw.outflows.iter().map(line).collect();
    let inflows: Vec<LcrLine> = raw.inflows.iter().map(line).collect();
    let total_outflows: f64 = outflows.iter().map(|l| l.weighted).sum();
    let total_inflows: f64 = inflows.iter().map(|l| l.weighted).sum();
    let capped_inflows = total_inflows.min(total_outflows * p.inflow_cap_pct / 100.0);
    let net_outflows = total_outflows - capped_inflows;
    let ratio = (net_outflows > 0.0).then(|| stock / net_outflows * 100.0);
    LcrFigures {
        ccy: ccy.to_string(),
        hqla_l1: raw.l1,
        hqla_l2a: raw.l2a,
        hqla_l2b: raw.l2b,
        adjustment_l2b_cap: adj_l2b,
        adjustment_l2_cap: adj_l2,
        stock,
        outflows,
        total_outflows,
        inflows,
        total_inflows,
        capped_inflows,
        net_outflows,
        ratio,
    }
}

/// Positions are one snapshot; `spots` convert each currency into the reporting currency.
pub fn compute(positions: &[BsPosition], params: &AlmParams, spots: &Spots, as_of: NaiveDate, params_version: Option<i32>) -> LcrResult {
    let p = &params.lcr;
    let haircut = |level: HqlaLevel| 1.0 - p.haircuts.get(level.as_str()).copied().unwrap_or(0.0) / 100.0;
    let horizon = as_of + Duration::days(30);
    let mut per_ccy: BTreeMap<String, Raw> = BTreeMap::new();

    for pos in positions {
        let raw = per_ccy.entry(pos.ccy.clone()).or_insert_with(Raw::new);
        match pos.side {
            BsSide::Asset => {
                if let Some(level) = pos.hqla_level {
                    if !pos.encumbered {
                        let v = pos.balance * haircut(level);
                        match level {
                            HqlaLevel::L1 => raw.l1 += v,
                            HqlaLevel::L2A => raw.l2a += v,
                            HqlaLevel::L2B => raw.l2b += v,
                        }
                    }
                    // A liquid asset is counted in the stock, never again as an inflow.
                    continue;
                }
                if let Some(class) = &pos.lcr_class {
                    if let Some(rate) = p.inflow_rates.get(class) {
                        let amount = due_within(pos, as_of, 30);
                        if amount > 0.0 {
                            let e = raw.inflows.entry(class.clone()).or_insert((0.0, 0.0));
                            e.0 += amount;
                            e.1 += amount * rate / 100.0;
                        }
                    }
                }
            }
            BsSide::Liability => {
                let within = pos.maturity.is_none_or(|m| m <= horizon);
                if let (true, Some(class)) = (within, &pos.lcr_class) {
                    if let Some(rate) = p.outflow_rates.get(class) {
                        let e = raw.outflows.entry(class.clone()).or_insert((0.0, 0.0));
                        e.0 += pos.balance;
                        e.1 += pos.balance * rate / 100.0;
                    }
                }
            }
            BsSide::OffBalance => {
                if let Some(class) = &pos.lcr_class {
                    if let Some(rate) = p.outflow_rates.get(class) {
                        let e = raw.outflows.entry(class.clone()).or_insert((0.0, 0.0));
                        e.0 += pos.undrawn;
                        e.1 += pos.undrawn * rate / 100.0;
                    }
                }
            }
        }
    }

    let mut total = Raw::new();
    for (ccy, raw) in &per_ccy {
        let fx = if *ccy == params.reporting_ccy { 1.0 } else { spots.get(ccy).copied().unwrap_or(1.0) };
        total.add(raw, fx);
    }
    let by_ccy = per_ccy.iter().map(|(ccy, raw)| figures(ccy, raw, params)).collect();
    LcrResult {
        as_of,
        params_version,
        reporting_ccy: params.reporting_ccy.clone(),
        minimum: p.minimum,
        internal_minimum: p.internal_minimum,
        early_warning: p.early_warning,
        total: figures(&params.reporting_ccy, &total, params),
        by_ccy,
    }
}
