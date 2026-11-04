//! Early-warning indicators for liquidity, read off the recent snapshots and the market, and the
//! contingency funding plan's stage they add up to.

use domain::{AlmParams, EwiIndicator, EwiPoint, EwiResult, EwiStatus, EwiThreshold};

/// One indicator's history, oldest first, and how to read it.
pub struct Series {
    pub key: &'static str,
    pub label: &'static str,
    pub what: &'static str,
    pub unit: &'static str,
    pub rising_is_worse: bool,
    pub points: Vec<EwiPoint>,
}

pub fn status(value: Option<f64>, t: EwiThreshold, rising_is_worse: bool) -> EwiStatus {
    let Some(v) = value else { return EwiStatus::Green };
    let past = |limit: f64| if rising_is_worse { v >= limit } else { v <= limit };
    if past(t.red) {
        EwiStatus::Red
    } else if past(t.amber) {
        EwiStatus::Amber
    } else {
        EwiStatus::Green
    }
}

/// The plan's stage: two reds is a crisis; one red or two ambers is an alert.
pub fn stage(statuses: &[EwiStatus]) -> &'static str {
    let reds = statuses.iter().filter(|s| **s == EwiStatus::Red).count();
    let ambers = statuses.iter().filter(|s| **s == EwiStatus::Amber).count();
    if reds >= 2 {
        "CRISIS"
    } else if reds == 1 || ambers >= 2 {
        "ALERT"
    } else {
        "NORMAL"
    }
}

pub fn compute(series: Vec<Series>, params: &AlmParams, as_of: chrono::NaiveDate, params_version: Option<i32>) -> EwiResult {
    let indicators: Vec<EwiIndicator> = series
        .into_iter()
        .map(|s| {
            let t = params.ewi.thresholds.get(s.key).copied().unwrap_or_default();
            let value = s.points.last().and_then(|p| p.value);
            EwiIndicator {
                key: s.key.into(),
                label: s.label.into(),
                what: s.what.into(),
                unit: s.unit.into(),
                value,
                amber: t.amber,
                red: t.red,
                rising_is_worse: s.rising_is_worse,
                status: status(value, t, s.rising_is_worse),
                series: s.points,
            }
        })
        .collect();
    let st = stage(&indicators.iter().map(|i| i.status).collect::<Vec<_>>());
    EwiResult { as_of, params_version, stage: st.into(), actions: params.ewi.actions.get(st).cloned().unwrap_or_default(), indicators }
}

/// Change over `n` observations, percent, of a series of levels.
pub fn pct_change(levels: &[(chrono::NaiveDate, f64)], n: usize) -> Vec<EwiPoint> {
    levels
        .iter()
        .enumerate()
        .map(|(i, (d, v))| EwiPoint { as_of: *d, value: (i >= n && levels[i - n].1.abs() > 1e-9).then(|| (v / levels[i - n].1 - 1.0) * 100.0) })
        .collect()
}

/// Change over `n` observations in the series' own unit.
pub fn change(levels: &[(chrono::NaiveDate, Option<f64>)], n: usize, scale: f64) -> Vec<EwiPoint> {
    levels
        .iter()
        .enumerate()
        .map(|(i, (d, v))| EwiPoint { as_of: *d, value: if i >= n { v.zip(levels[i - n].1).map(|(a, b)| (a - b) * scale) } else { None } })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds_read_in_the_right_direction_and_add_up_to_a_stage() {
        let falls = EwiThreshold { amber: -2.0, red: -5.0 };
        assert_eq!(status(Some(-1.0), falls, false), EwiStatus::Green);
        assert_eq!(status(Some(-3.0), falls, false), EwiStatus::Amber);
        assert_eq!(status(Some(-6.0), falls, false), EwiStatus::Red);
        let rises = EwiThreshold { amber: 25.0, red: 50.0 };
        assert_eq!(status(Some(30.0), rises, true), EwiStatus::Amber);
        assert_eq!(status(None, rises, true), EwiStatus::Green);
        assert_eq!(stage(&[EwiStatus::Green, EwiStatus::Amber]), "NORMAL");
        assert_eq!(stage(&[EwiStatus::Amber, EwiStatus::Amber]), "ALERT");
        assert_eq!(stage(&[EwiStatus::Red, EwiStatus::Green]), "ALERT");
        assert_eq!(stage(&[EwiStatus::Red, EwiStatus::Red]), "CRISIS");
    }

    #[test]
    fn five_day_changes() {
        let d = |i: i64| chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap() + chrono::Duration::days(i);
        let levels: Vec<(chrono::NaiveDate, f64)> = (0..7).map(|i| (d(i), 100.0 - i as f64)).collect();
        let c = pct_change(&levels, 5);
        assert_eq!(c[4].value, None);
        assert!((c[5].value.unwrap() - -5.0).abs() < 1e-9);
    }
}
