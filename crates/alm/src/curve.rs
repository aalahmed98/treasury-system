//! Rate curves: interpolation by tenor, the Basel standardised shocks, the post-shock floor and
//! discount factors.

use domain::{AlmParams, RateCurve, RateScenario, RateShock};

use crate::bucket::GAP_BUCKETS;

/// The upper edge of each gap bucket in years, for slotting a tenor into the bucket a custom
/// shock is aimed at. They agree with the day edges of `GAP_BUCKETS` on whole days and put the
/// one-month and three-month index tenors (1/12 and 3/12 of a year) in their own buckets.
pub const BUCKET_EDGE_YEARS: [f64; 8] = [1.0 / 365.0, 7.0 / 365.0, 1.0 / 12.0, 0.25, 0.5, 1.0, 2.0, 5.0];

/// Index of the gap bucket a tenor in years falls into.
pub fn bucket_of_years(t: f64) -> usize {
    BUCKET_EDGE_YEARS.iter().position(|edge| t <= edge + 1e-12).unwrap_or(BUCKET_EDGE_YEARS.len())
}

/// The custom scenario: a size in basis points and the gap buckets it moves. Every bucket is a
/// parallel shift; fewer move only the rates at those tenors and leave the rest of the curve alone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CustomShock {
    pub bp: f64,
    /// Bit `i` set moves bucket `i` of `GAP_BUCKETS`.
    pub mask: u16,
}

impl CustomShock {
    pub const ALL: u16 = (1 << GAP_BUCKETS.len()) - 1;

    pub fn parallel(bp: f64) -> CustomShock {
        CustomShock { bp, mask: Self::ALL }
    }

    /// From bucket labels (`3M`, `1Y`); none means every bucket.
    pub fn from_labels(bp: f64, labels: &[String]) -> Result<CustomShock, String> {
        if labels.is_empty() {
            return Ok(Self::parallel(bp));
        }
        let mut mask = 0u16;
        for l in labels {
            let i = GAP_BUCKETS.iter().position(|(name, _)| name == l).ok_or_else(|| format!("unknown tenor bucket {l}"))?;
            mask |= 1 << i;
        }
        Ok(CustomShock { bp, mask })
    }

    pub fn is_parallel(self) -> bool {
        self.mask & Self::ALL == Self::ALL
    }

    pub fn moves(self, bucket: usize) -> bool {
        self.mask & (1 << bucket) != 0
    }

    /// Basis points applied at `t` years.
    pub fn bp_at(self, t: f64) -> f64 {
        if self.moves(bucket_of_years(t)) { self.bp } else { 0.0 }
    }

    /// The buckets it moves, as a run (`3M to 1Y`) when they are neighbours, else a list.
    pub fn tenors(self) -> String {
        let on: Vec<usize> = (0..GAP_BUCKETS.len()).filter(|i| self.moves(*i)).collect();
        if on.len() == GAP_BUCKETS.len() {
            return "all tenors".into();
        }
        match (on.first(), on.last()) {
            (Some(a), Some(b)) if a == b => GAP_BUCKETS[*a].0.into(),
            (Some(a), Some(b)) if b - a + 1 == on.len() => format!("{} to {}", GAP_BUCKETS[*a].0, GAP_BUCKETS[*b].0),
            _ => on.iter().map(|i| GAP_BUCKETS[*i].0).collect::<Vec<_>>().join(", "),
        }
    }

    pub fn label(self) -> String {
        if self.is_parallel() { format!("Custom parallel {:+.0} bp", self.bp) } else { format!("Custom {:+.0} bp at {}", self.bp, self.tenors()) }
    }

    /// For cache keys.
    pub fn key(self) -> String {
        format!("{}:{}", self.bp, self.mask & Self::ALL)
    }
}

impl From<f64> for CustomShock {
    fn from(bp: f64) -> Self {
        CustomShock::parallel(bp)
    }
}

/// What a result ran under, for its heading: nothing on the base curve.
pub fn shock_label(scenario: RateScenario, custom: CustomShock) -> Option<String> {
    match scenario {
        RateScenario::Base => None,
        RateScenario::Custom => Some(custom.label()),
        s => Some(s.label().into()),
    }
}

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

    /// The curve with a custom shock on some buckets only: a step at each bucket edge, so a tenor
    /// moves by the full size when its bucket is chosen and not at all otherwise.
    pub fn bucket_shocked(&self, custom: CustomShock, floor_bp: f64, floor_slope_bp_per_year: f64) -> Curve {
        const EPS: f64 = 1e-6;
        let mut tenors: Vec<f64> = self.points.iter().map(|p| p.0).collect();
        for edge in BUCKET_EDGE_YEARS {
            tenors.push(edge);
            tenors.push(edge + EPS);
        }
        tenors.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        tenors.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        let points = tenors
            .into_iter()
            .map(|t| {
                let shifted = self.rate(t) + custom.bp_at(t) / 100.0;
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

/// The curve to use under a scenario; `custom` sizes and aims the custom scenario.
pub fn scenario_curve(base: &Curve, params: &AlmParams, scenario: RateScenario, custom: impl Into<CustomShock>) -> Curve {
    let custom = custom.into();
    let mut shock = shock_for(params, &base.ccy);
    if scenario == RateScenario::Custom {
        if !custom.is_parallel() {
            return base.bucket_shocked(custom, params.stress.floor_bp, params.stress.floor_slope_bp_per_year);
        }
        shock.parallel = custom.bp;
    }
    base.shocked(scenario, shock, params.stress.floor_bp, params.stress.floor_slope_bp_per_year)
}
