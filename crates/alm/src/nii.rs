//! Net interest income projection: monthly steps over the horizon. On a constant balance sheet
//! (the default) a cohort that matures is replaced: fixed cohorts keep their rate until maturity
//! and roll at the scenario curve keeping their margin over today's curve; floating cohorts
//! reprice at each reset to the scenario index plus spread. With the balance sheet running off, a
//! cohort earns or pays nothing after its maturity and the cash it releases (an asset) or takes
//! (a liability) is placed or funded at the scenario's one-month rate in its currency, shown as the
//! BALANCING book; NIM is then on the average earning assets over the horizon. On a dynamic
//! balance sheet cohorts are replaced as on a constant one and every balance also grows at its
//! yearly rate, the cash that takes or brings going through the same BALANCING book. Where a
//! product has a prepayment or early-withdrawal rate, the share that has left early reprices (or,
//! running off, is released) before the cohort matures. Administered
//! products move by their beta times the shock at the short end; the betas are ALM assumptions
//! per product (and segment), not the source data's.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use domain::{AlmParams, BalanceSheetMode, BsPosition, BsSide, NiiBreakdown, NiiMonth, NiiModeRow, NiiProduct, NiiResult, RateScenario, RateType};

use crate::behaviour;
use crate::curve::{Curve, CustomShock, scenario_curve, shock_label};
use crate::hedge::HEDGE_BOOK;
use crate::nmd;
use crate::lcr::Spots;
use crate::schedule::add_months;

/// Base curves per currency.
pub type Curves = BTreeMap<String, Curve>;

fn curve_for<'a>(curves: &'a Curves, ccy: &str, fallback: &'a Curve) -> &'a Curve {
    curves.get(ccy).unwrap_or(fallback)
}

/// What a fixed cohort is replaced at when it rolls over: the scenario curve at its original term
/// plus the cohort's margin over today's curve for that term.
pub fn rolled_rate(pos: &BsPosition, maturity: NaiveDate, base: &Curve, shocked: &Curve) -> f64 {
    let term_years = pos.origination.map(|o| (maturity - o).num_days().max(30) as f64 / 365.0).unwrap_or(1.0);
    pos.rate - base.rate(term_years) + shocked.rate(term_years)
}

/// The rate a cohort earns or pays in the month starting at `start`, under a scenario.
pub fn rate_in_month(pos: &BsPosition, beta: f64, as_of: NaiveDate, start: NaiveDate, base: &Curve, shocked: &Curve) -> f64 {
    match pos.rate_type {
        RateType::NotSensitive => 0.0,
        RateType::Fixed => match pos.maturity {
            Some(m) if m <= start => rolled_rate(pos, m, base, shocked),
            _ => pos.rate,
        },
        RateType::Float => {
            let reset = pos.reset_months.unwrap_or(3).max(1) as i64;
            match pos.next_repricing {
                Some(n) if n <= start => shocked.rate(reset as f64 / 12.0) + pos.spread,
                Some(_) => pos.rate,
                None => shocked.rate(reset as f64 / 12.0) + pos.spread,
            }
        }
        RateType::Admin => {
            let short = shocked.rate(1.0 / 12.0) - base.rate(1.0 / 12.0);
            (pos.rate + beta * short).max(0.0)
        }
    }
    .max(0.0)
        * if pos.maturity.is_some_and(|m| m <= as_of) { 0.0 } else { 1.0 }
}

#[allow(clippy::too_many_arguments)]
pub fn compute(
    positions: &[BsPosition],
    params: &AlmParams,
    spots: &Spots,
    curves: &Curves,
    as_of: NaiveDate,
    scenario: RateScenario,
    custom_bp: impl Into<CustomShock>,
    params_version: Option<i32>,
) -> NiiResult {
    let custom_bp = custom_bp.into();
    let horizon = params.nii.horizon_months.clamp(1, 60);
    let fx = |ccy: &str| if ccy == params.reporting_ccy { 1.0 } else { spots.get(ccy).copied().unwrap_or(1.0) };
    let fallback = Curve::flat("", 0.0);
    let shocked: BTreeMap<String, Curve> = curves.iter().map(|(c, k)| (c.clone(), scenario_curve(k, params, scenario, custom_bp))).collect();

    let mut months: Vec<NiiMonth> = Vec::with_capacity(horizon as usize);
    let mut by_book: BTreeMap<String, f64> = BTreeMap::new();
    let mut by_ccy: BTreeMap<String, f64> = BTreeMap::new();
    let mode = params.nii.mode();
    let run_off = mode == BalanceSheetMode::RunOff;
    let dynamic = mode == BalanceSheetMode::Dynamic;
    let mut earning_assets_sum = 0.0;
    let mut migrated = 0.0f64;
    for m in 0..horizon as i64 {
        let start = add_months(as_of, m);
        let end = add_months(as_of, m + 1);
        let days = (end - start).num_days().max(1) as f64;
        let mut income = 0.0;
        let mut expense = 0.0;
        let mut earning = 0.0;
        // Running off: per currency, the cash released by matured assets less that taken by
        // matured liabilities, in that currency.
        let mut released: BTreeMap<&str, f64> = BTreeMap::new();
        for pos in positions {
            if pos.side == BsSide::OffBalance || pos.rate_type == RateType::NotSensitive {
                continue;
            }
            if run_off && pos.maturity.is_some_and(|mat| mat <= start) {
                if pos.maturity.is_some_and(|mat| mat > as_of) {
                    let sign = if pos.side == BsSide::Asset { 1.0 } else { -1.0 };
                    *released.entry(pos.ccy.as_str()).or_default() += sign * pos.balance;
                }
                continue;
            }
            // Growing: the cohort's balance compounds at its yearly rate, new volume priced as the
            // cohort is, and the cash the growth takes (an asset) or brings (a liability) is
            // funded or placed with the balancing book.
            let balance = if dynamic {
                let grown = pos.balance * (1.0 + params.nii.growth_for(&pos.product, pos.side) / 100.0).max(0.0).powf(m as f64 / 12.0);
                let sign = if pos.side == BsSide::Asset { -1.0 } else { 1.0 };
                *released.entry(pos.ccy.as_str()).or_default() += sign * (grown - pos.balance);
                grown
            } else {
                pos.balance
            };
            let base = curve_for(curves, &pos.ccy, &fallback);
            let sh = curve_for(&shocked, &pos.ccy, base);
            let mut rate = rate_in_month(pos, params.nii.beta_for(&pos.product, &pos.segment), as_of, start, base, sh);
            // Prepaid loans and broken term deposits: the share that has left early by this month,
            // at the speed the scenario's move at the cohort's maturity gives it. Running off, it
            // goes to the balancing book like a maturity; otherwise it is replaced, so it earns
            // or pays the rolled rate ahead of time.
            let mut balance = balance;
            let early = behaviour::speed(pos, params);
            if let Some(mat) = pos.maturity.filter(|mat| early > 0.0 && *mat > start && *mat > as_of) {
                let t = (mat - as_of).num_days() as f64 / 365.0;
                let yearly = early * behaviour::multiplier(pos, params, sh.rate(t) - base.rate(t));
                let gone = 1.0 - behaviour::surviving(yearly, (start - as_of).num_days() as f64 / 365.0);
                if run_off {
                    let sign = if pos.side == BsSide::Asset { 1.0 } else { -1.0 };
                    *released.entry(pos.ccy.as_str()).or_default() += sign * balance * gone;
                    balance *= 1.0 - gone;
                } else {
                    rate = ((1.0 - gone) * rate + gone * rolled_rate(pos, mat, base, sh)).max(0.0);
                }
            }
            // Deposit migration: the share of a non-maturity deposit that a rise in short rates
            // moves into term deposits pays the scenario's rate for the term, not the account's.
            let moved = nmd::migrated_share(pos, params, sh.rate(nmd::SHORT_YEARS) - base.rate(nmd::SHORT_YEARS));
            if moved > 0.0 {
                rate = (1.0 - moved) * rate + moved * sh.rate(nmd::migration_term_years(params)).max(0.0);
                if m == 0 {
                    migrated += pos.balance * fx(&pos.ccy) * moved;
                }
            }
            if pos.side == BsSide::Asset && pos.book != HEDGE_BOOK && pos.maturity.is_none_or(|mat| mat > as_of) {
                earning += balance * fx(&pos.ccy);
            }
            let interest = balance * fx(&pos.ccy) * rate / 100.0 * days / pos.day_count.basis();
            let signed = match pos.side {
                BsSide::Asset => {
                    income += interest;
                    interest
                }
                _ => {
                    expense += interest;
                    -interest
                }
            };
            *by_book.entry(pos.book.clone()).or_default() += signed;
            *by_ccy.entry(pos.ccy.clone()).or_default() += signed;
        }
        for (ccy, amount) in released {
            if amount.abs() < 1e-9 {
                continue;
            }
            let base = curve_for(curves, ccy, &fallback);
            let sh = curve_for(&shocked, ccy, base);
            let rate = sh.rate(1.0 / 12.0).max(0.0);
            let interest = amount.abs() * fx(ccy) * rate / 100.0 * days / 360.0;
            let signed = if amount > 0.0 {
                income += interest;
                earning += amount * fx(ccy);
                interest
            } else {
                expense += interest;
                -interest
            };
            *by_book.entry("BALANCING".into()).or_default() += signed;
            *by_ccy.entry(ccy.to_string()).or_default() += signed;
        }
        earning_assets_sum += earning;
        months.push(NiiMonth { period: start, income, expense, nii: income - expense });
    }
    let earning_assets = earning_assets_sum / horizon as f64;
    let income: f64 = months.iter().map(|m| m.income).sum();
    let expense: f64 = months.iter().map(|m| m.expense).sum();
    let nii = income - expense;
    let annualised = nii * 12.0 / horizon as f64;
    NiiResult {
        as_of,
        params_version,
        reporting_ccy: params.reporting_ccy.clone(),
        scenario,
        horizon_months: horizon,
        months,
        income,
        expense,
        nii,
        earning_assets,
        nim: (earning_assets > 0.0).then(|| annualised / earning_assets * 100.0),
        by_book: by_book.into_iter().map(|(key, nii)| NiiBreakdown { key, nii }).collect(),
        by_ccy: by_ccy.into_iter().map(|(key, nii)| NiiBreakdown { key, nii }).collect(),
        base_nii: None,
        shock_label: shock_label(scenario, custom_bp),
        migrated: (migrated > 0.0).then_some(migrated),
        mode,
        modes: Vec::new(),
        products: Vec::new(),
    }
}

/// The projection under each balance-sheet assumption, with the base scenario's beside a shocked one.
#[allow(clippy::too_many_arguments)]
pub fn modes(positions: &[BsPosition], params: &AlmParams, spots: &Spots, curves: &Curves, as_of: NaiveDate, scenario: RateScenario, custom: impl Into<CustomShock>, params_version: Option<i32>) -> Vec<NiiModeRow> {
    let custom = custom.into();
    BalanceSheetMode::ALL
        .iter()
        .map(|&mode| {
            let mut p = params.clone();
            p.nii.set_mode(mode);
            let r = compute(positions, &p, spots, curves, as_of, scenario, custom, params_version);
            let base_nii = (scenario != RateScenario::Base).then(|| compute(positions, &p, spots, curves, as_of, RateScenario::Base, 0.0, params_version).nii);
            NiiModeRow { mode, nii: r.nii, nim: r.nim, base_nii }
        })
        .collect()
}

/// The rate-sensitive products on the balance sheet, largest first within each side.
pub fn products(positions: &[BsPosition], params: &AlmParams, spots: &Spots) -> Vec<NiiProduct> {
    let fx = |ccy: &str| if ccy == params.reporting_ccy { 1.0 } else { spots.get(ccy).copied().unwrap_or(1.0) };
    let mut by: BTreeMap<(bool, String), f64> = BTreeMap::new();
    for pos in positions {
        if pos.side == BsSide::OffBalance || pos.rate_type == RateType::NotSensitive || pos.book == HEDGE_BOOK {
            continue;
        }
        *by.entry((pos.side == BsSide::Liability, pos.product.clone())).or_default() += pos.balance * fx(&pos.ccy);
    }
    let mut out: Vec<NiiProduct> = by.into_iter().map(|((liability, product), balance)| NiiProduct { product, side: if liability { BsSide::Liability } else { BsSide::Asset }, balance }).collect();
    out.sort_by(|a, b| (a.side == BsSide::Liability).cmp(&(b.side == BsSide::Liability)).then(b.balance.partial_cmp(&a.balance).unwrap_or(std::cmp::Ordering::Equal)));
    out
}
