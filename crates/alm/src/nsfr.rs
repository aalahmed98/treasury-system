//! Net stable funding ratio: available stable funding over required stable funding, with the
//! factors the bank's parameters set per class and residual-maturity band.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use domain::{AlmParams, BsPosition, BsSide, NsfrLine, NsfrResult};

use crate::bucket::{NSFR_BANDS, nsfr_band};
use crate::lcr::Spots;

pub fn compute(positions: &[BsPosition], params: &AlmParams, spots: &Spots, as_of: NaiveDate, params_version: Option<i32>) -> NsfrResult {
    let p = &params.nsfr;
    let fx = |ccy: &str| if ccy == params.reporting_ccy { 1.0 } else { spots.get(ccy).copied().unwrap_or(1.0) };
    let mut asf: BTreeMap<(String, usize), (f64, f64, f64)> = BTreeMap::new();
    let mut rsf: BTreeMap<(String, usize), (f64, f64, f64)> = BTreeMap::new();
    let add = |m: &mut BTreeMap<(String, usize), (f64, f64, f64)>, class: String, band: usize, amount: f64, factor: f64| {
        let e = m.entry((class, band)).or_insert((0.0, factor, 0.0));
        e.0 += amount;
        e.1 = factor;
        e.2 += amount * factor / 100.0;
    };

    for pos in positions {
        let amount = pos.balance * fx(&pos.ccy);
        let band = nsfr_band(pos.maturity.map(|m| (m - as_of).num_days()));
        match pos.side {
            BsSide::Liability => {
                let class = pos.nsfr_class.clone().unwrap_or_else(|| "OTHER".into());
                let factor = p.asf.get(&class).map(|f| f[band]).unwrap_or(0.0);
                add(&mut asf, class, band, amount, factor);
            }
            BsSide::Asset => {
                // Cash and central-bank reserves carry their own factor (0% under Basel III), not
                // the Level 1 securities factor.
                if let Some(level) = pos.hqla_level.filter(|_| pos.nsfr_class.as_deref() != Some("CASH_CB")) {
                    let factor = if pos.encumbered && band == 2 { p.encumbered_rsf_pct } else { p.hqla_rsf.get(level.as_str()).copied().unwrap_or(100.0) };
                    add(&mut rsf, format!("HQLA {}", level.as_str()), band, amount, factor);
                } else {
                    let class = pos.nsfr_class.clone().unwrap_or_else(|| "OTHER".into());
                    let factor = if pos.encumbered && band == 2 { p.encumbered_rsf_pct } else { p.rsf.get(&class).map(|f| f[band]).unwrap_or(100.0) };
                    add(&mut rsf, class, band, amount, factor);
                }
            }
            BsSide::OffBalance => {
                let undrawn = pos.undrawn * fx(&pos.ccy);
                add(&mut rsf, "OFF_BALANCE".into(), 0, undrawn, p.off_balance_rsf_pct);
            }
        }
    }
    let lines = |m: BTreeMap<(String, usize), (f64, f64, f64)>| -> Vec<NsfrLine> {
        m.into_iter().map(|((class, band), (amount, factor, weighted))| NsfrLine { class, bucket: NSFR_BANDS[band].into(), amount, factor, weighted }).collect()
    };
    let asf_lines = lines(asf);
    let rsf_lines = lines(rsf);
    let asf_total: f64 = asf_lines.iter().map(|l| l.weighted).sum();
    let rsf_total: f64 = rsf_lines.iter().map(|l| l.weighted).sum();
    NsfrResult {
        as_of,
        params_version,
        reporting_ccy: params.reporting_ccy.clone(),
        minimum: p.minimum,
        internal_minimum: p.internal_minimum,
        asf_lines,
        rsf_lines,
        asf: asf_total,
        rsf: rsf_total,
        ratio: (rsf_total > 0.0).then(|| asf_total / rsf_total * 100.0),
    }
}
