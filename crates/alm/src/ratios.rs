//! Balance-sheet health ratios: loans to deposits, liquid assets, cost of funds and asset yield.

use domain::{AlmParams, BsPosition, BsSide, Ratios};

use crate::lcr::Spots;

pub fn compute(positions: &[BsPosition], params: &AlmParams, spots: &Spots, wholesale_funding_pct: f64) -> Ratios {
    let fx = |ccy: &str| if ccy == params.reporting_ccy { 1.0 } else { spots.get(ccy).copied().unwrap_or(1.0) };
    let (mut loans, mut deposits, mut liquid, mut assets, mut equity) = (0.0, 0.0, 0.0, 0.0, 0.0);
    let (mut cost_w, mut cost_b, mut yield_w, mut yield_b) = (0.0, 0.0, 0.0, 0.0);
    for p in positions {
        let v = p.balance * fx(&p.ccy);
        match p.side {
            BsSide::Asset => {
                assets += v;
                if p.book == "LOANS" {
                    loans += v;
                }
                if p.hqla_level.is_some() && !p.encumbered || p.book == "CASH" {
                    liquid += v;
                }
                if p.book != "OTHER" && p.book != "CASH" {
                    yield_w += v * p.rate;
                    yield_b += v;
                }
            }
            BsSide::Liability => {
                if p.book == "EQUITY" {
                    equity += v;
                    continue;
                }
                if p.book == "DEPOSITS" {
                    deposits += v;
                }
                cost_w += v * p.rate;
                cost_b += v;
            }
            BsSide::OffBalance => {}
        }
    }
    let pct = |a: f64, b: f64| (b > 0.0).then(|| a / b * 100.0);
    let cost = (cost_b > 0.0).then(|| cost_w / cost_b);
    let yld = (yield_b > 0.0).then(|| yield_w / yield_b);
    Ratios {
        loan_to_deposit: pct(loans, deposits),
        liquid_assets_to_assets: pct(liquid, assets),
        cost_of_funds: cost,
        asset_yield: yld,
        net_spread: yld.zip(cost).map(|(y, c)| y - c),
        wholesale_funding: wholesale_funding_pct,
        equity_to_assets: pct(equity, assets),
    }
}
