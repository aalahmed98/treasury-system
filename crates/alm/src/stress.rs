//! Stress testing: the six Basel rate scenarios and a custom one through the NII and EVE engines,
//! a liquidity stress on the LCR with a day-by-day survival horizon, and FX shocks on net
//! currency exposures.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use domain::{AlmParams, BsPosition, BsSide, FxStressRow, LiquidityStressResult, RateScenario, StressResult, StressScenarioRow, SurvivalPoint};

use crate::lcr::Spots;
use crate::nii::Curves;
use crate::schedule::cash_flows;
use crate::{eve, lcr, nii};

/// The parameter set under the liquidity stress: run-off and haircuts up, inflows down.
pub fn stressed_params(params: &AlmParams) -> AlmParams {
    let mut p = params.clone();
    let s = &params.stress.liquidity;
    for v in p.lcr.outflow_rates.values_mut() {
        *v = (*v * s.outflow_multiplier).min(100.0);
    }
    for v in p.lcr.haircuts.values_mut() {
        *v = (*v + s.extra_hqla_haircut_pct).min(100.0);
    }
    for v in p.lcr.inflow_rates.values_mut() {
        *v = (*v * s.inflow_multiplier).clamp(0.0, 100.0);
    }
    p
}

/// Cumulative stressed net outflow per day over ninety days against the stressed stock.
pub fn survival(positions: &[BsPosition], params: &AlmParams, stressed: &AlmParams, spots: &Spots, as_of: NaiveDate, stock: f64) -> (Option<u32>, Vec<SurvivalPoint>) {
    let fx = |ccy: &str| if params.reporting_ccy == ccy { 1.0 } else { spots.get(ccy).copied().unwrap_or(1.0) };
    let mut daily = vec![0.0f64; 91];
    for pos in positions {
        let v = fx(&pos.ccy);
        match pos.side {
            BsSide::Liability => {
                let Some(class) = &pos.lcr_class else { continue };
                let Some(rate) = stressed.lcr.outflow_rates.get(class) else { continue };
                let out = pos.balance * v * rate / 100.0;
                match pos.maturity {
                    // Non-maturing money runs off evenly over the first thirty days.
                    None => {
                        for d in daily.iter_mut().take(31).skip(1) {
                            *d += out / 30.0;
                        }
                    }
                    Some(m) => {
                        let day = (m - as_of).num_days();
                        if (1..=90).contains(&day) {
                            daily[day as usize] += out;
                        }
                    }
                }
            }
            BsSide::OffBalance => {
                let Some(class) = &pos.lcr_class else { continue };
                let Some(rate) = stressed.lcr.outflow_rates.get(class) else { continue };
                let out = pos.undrawn * v * rate / 100.0;
                for d in daily.iter_mut().take(31).skip(1) {
                    *d += out / 30.0;
                }
            }
            BsSide::Asset => {
                if pos.hqla_level.is_some() {
                    continue;
                }
                let Some(class) = &pos.lcr_class else { continue };
                let Some(rate) = stressed.lcr.inflow_rates.get(class) else { continue };
                for f in cash_flows(pos, as_of) {
                    let day = (f.date - as_of).num_days();
                    if (1..=90).contains(&day) {
                        daily[day as usize] -= f.total() * v * rate / 100.0;
                    }
                }
            }
        }
    }
    let mut cumulative = 0.0;
    let mut survival = None;
    let mut horizon = Vec::with_capacity(90);
    for (day, net) in daily.iter().enumerate().skip(1) {
        cumulative += net;
        if survival.is_none() && cumulative > stock {
            survival = Some(day as u32);
        }
        horizon.push(SurvivalPoint { day: day as u32, cumulative_net_outflow: cumulative });
    }
    (survival, horizon)
}

#[allow(clippy::too_many_arguments)]
pub fn compute(
    positions: &[BsPosition],
    params: &AlmParams,
    spots: &Spots,
    curves: &Curves,
    as_of: NaiveDate,
    custom_bp: Option<f64>,
    params_version: Option<i32>,
) -> StressResult {
    let base_nii = nii::compute(positions, params, spots, curves, as_of, RateScenario::Base, 0.0, params_version);
    let base_eve = eve::compute(positions, params, spots, curves, as_of, RateScenario::Base, 0.0, params_version);
    let mut scenarios: Vec<RateScenario> = RateScenario::BASEL.to_vec();
    if custom_bp.is_some() {
        scenarios.push(RateScenario::Custom);
    }
    let bp = custom_bp.unwrap_or(0.0);
    let tier1 = params.tier1_capital;
    let rows: Vec<StressScenarioRow> = scenarios
        .into_iter()
        .map(|s| {
            let n = nii::compute(positions, params, spots, curves, as_of, s, bp, params_version);
            let e = eve::compute(positions, params, spots, curves, as_of, s, bp, params_version);
            let nii_delta = n.nii - base_nii.nii;
            let nii_delta_pct = (base_nii.nii.abs() > 1e-9).then(|| nii_delta / base_nii.nii.abs() * 100.0);
            let eve_loss_pct_tier1 = (tier1 > 0.0).then(|| e.basel_loss / tier1 * 100.0);
            StressScenarioRow {
                scenario: s,
                label: if s == RateScenario::Custom { format!("Custom parallel {bp:+.0} bp") } else { s.label().into() },
                nii: n.nii,
                nii_delta,
                nii_delta_pct,
                eve_delta: e.delta,
                eve_loss: e.basel_loss,
                eve_loss_pct_tier1,
                nii_breach: nii_delta_pct.is_some_and(|p| p < -params.gap.nii_sensitivity_limit_pct),
                eve_breach: eve_loss_pct_tier1.is_some_and(|p| p > params.gap.eve_limit_pct_tier1),
            }
        })
        .collect();
    let worst_eve = rows.iter().max_by(|a, b| a.eve_loss.partial_cmp(&b.eve_loss).unwrap_or(std::cmp::Ordering::Equal)).filter(|r| r.eve_loss > 0.0).map(|r| r.scenario);

    let base_lcr = lcr::compute(positions, params, spots, as_of, params_version);
    let stressed = stressed_params(params);
    let stressed_lcr = lcr::compute(positions, &stressed, spots, as_of, None);
    let (survival_days, horizon) = survival(positions, params, &stressed, spots, as_of, stressed_lcr.total.stock);
    let liquidity = LiquidityStressResult {
        lcr_base: base_lcr.total.ratio,
        lcr_stressed: stressed_lcr.total.ratio,
        stock_stressed: stressed_lcr.total.stock,
        net_outflows_stressed: stressed_lcr.total.net_outflows,
        survival_days,
        horizon,
    };

    let fx_rate = |ccy: &str| if params.reporting_ccy == ccy { 1.0 } else { spots.get(ccy).copied().unwrap_or(1.0) };
    let mut exposure: BTreeMap<String, f64> = BTreeMap::new();
    for pos in positions {
        if pos.ccy == params.reporting_ccy || pos.side == BsSide::OffBalance {
            continue;
        }
        let sign = if pos.side == BsSide::Asset { 1.0 } else { -1.0 };
        *exposure.entry(pos.ccy.clone()).or_default() += sign * pos.balance * fx_rate(&pos.ccy);
    }
    let fx = params
        .stress
        .fx_shock_pct
        .iter()
        .filter_map(|(ccy, shock)| exposure.get(ccy).map(|e| FxStressRow { ccy: ccy.clone(), shock_pct: *shock, exposure: *e, pnl: -e * shock / 100.0 }))
        .collect();

    StressResult {
        as_of,
        params_version,
        reporting_ccy: params.reporting_ccy.clone(),
        base_nii: base_nii.nii,
        base_eve: base_eve.base,
        tier1_capital: tier1,
        nii_limit_pct: params.gap.nii_sensitivity_limit_pct,
        eve_limit_pct_tier1: params.gap.eve_limit_pct_tier1,
        rate_scenarios: rows,
        worst_eve,
        liquidity,
        fx,
    }
}
