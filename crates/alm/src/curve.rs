//! Rate curves: interpolation by tenor, the Basel standardised shocks, the post-shock floor and
//! discount factors.

use domain::{AlmParams, RateCurve, RateScenario, RateShock};

#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    pub ccy: String,
    /// (years, percent), sorted by years.
    pub points: Vec<(f64, f64)>,
}

impl Curve {
    pub fn from_rate_curve(c: &RateCurve) -> Curve {
        let mut points: Vec<(f64, f64)> = c.points.iter().map(|p| (p.tenor_days as f64 / 365.0, p.rate)).collect();
        points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        Curve { ccy: c.ccy.clone(), points }
    }

    pub fn flat(ccy: &str, rate: f64) -> Curve {
        Curve { ccy: ccy.into(), points: vec![(0.0, rate), (30.0, rate)] }
    }

    /// Percent at `years`, linear between tenors and flat beyond the ends.
    pub fn rate(&self, years: f64) -> f64 {
        let Some(first) = self.points.first() else { return 0.0 };
        if years <= first.0 {
            return first.1;
        }
        for w in self.points.windows(2) {
            let ((t0, r0), (t1, r1)) = (w[0], w[1]);
            if years <= t1 {
                return if (t1 - t0).abs() < 1e-12 { r1 } else { r0 + (r1 - r0) * (years - t0) / (t1 - t0) };
            }
        }
        self.points.last().map(|p| p.1).unwrap_or(0.0)
    }

    /// Discount factor at `years`, annual compounding.
    pub fn df(&self, years: f64) -> f64 {
        (1.0 + self.rate(years) / 100.0).powf(-years.max(0.0))
    }

    /// The curve under a scenario: every point shifted by its shock and held above the floor.
    pub fn shocked(&self, scenario: RateScenario, shock: RateShock, floor_bp: f64, floor_slope_bp_per_year: f64) -> Curve {
        let points = self
            .points
            .iter()
            .map(|&(t, r)| {
                let shifted = r + shock_bp(scenario, shock, t) / 100.0;
                let floor = (floor_bp + floor_slope_bp_per_year * t).min(0.0) / 100.0;
                (t, shifted.max(floor))
            })
            .collect();
        Curve { ccy: self.ccy.clone(), points }
    }
}

/// Basel's shapes in basis points at tenor `t` years; `shock.parallel` carries the custom size.
pub fn shock_bp(scenario: RateScenario, shock: RateShock, t: f64) -> f64 {
    let short = (-t / 4.0).exp();
    let long = 1.0 - short;
    match scenario {
        RateScenario::Base => 0.0,
        RateScenario::ParallelUp => shock.parallel,
        RateScenario::ParallelDown => -shock.parallel,
        RateScenario::ShortUp => shock.short * short,
        RateScenario::ShortDown => -shock.short * short,
        RateScenario::Steepener => -0.65 * shock.short * short + 0.9 * shock.long * long,
        RateScenario::Flattener => 0.8 * shock.short * short - 0.6 * shock.long * long,
        RateScenario::Custom => shock.parallel,
    }
}

/// The shock set for a currency: its own entry, else USD's (BHD, AED and SAR are pegged), else a
/// 200 basis-point parallel.
pub fn shock_for(params: &AlmParams, ccy: &str) -> RateShock {
    params
        .stress
        .shocks
        .get(ccy)
        .or_else(|| params.stress.shocks.get("USD"))
        .copied()
        .unwrap_or(RateShock { parallel: 200.0, short: 300.0, long: 150.0 })
}

/// The curve to use under a scenario; `custom_bp` sizes the custom parallel scenario.
pub fn scenario_curve(base: &Curve, params: &AlmParams, scenario: RateScenario, custom_bp: f64) -> Curve {
    let mut shock = shock_for(params, &base.ccy);
    if scenario == RateScenario::Custom {
        shock.parallel = custom_bp;
    }
    base.shocked(scenario, shock, params.stress.floor_bp, params.stress.floor_slope_bp_per_year)
}
