//! Time buckets shared by the repricing gap and the stable-funding maturity bands.

pub const GAP_BUCKETS: [(&str, i64); 9] = [("ON", 1), ("1W", 7), ("1M", 30), ("3M", 91), ("6M", 182), ("1Y", 365), ("2Y", 730), ("5Y", 1825), ("10Y+", i64::MAX)];

/// Index of the gap bucket a tenor in days falls into.
pub fn gap_bucket(days: i64) -> usize {
    GAP_BUCKETS.iter().position(|(_, upto)| days <= *upto).unwrap_or(GAP_BUCKETS.len() - 1)
}

/// NSFR residual-maturity band: 0 under six months, 1 six months to a year, 2 a year or more.
pub fn nsfr_band(days: Option<i64>) -> usize {
    match days {
        None => 0,
        Some(d) if d < 182 => 0,
        Some(d) if d < 365 => 1,
        Some(_) => 2,
    }
}

pub const NSFR_BANDS: [&str; 3] = ["< 6M", "6M-1Y", ">= 1Y"];
