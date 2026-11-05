//! What the engine reads besides the portfolio: spots, the market curves day by day (for floating
//! fixings), today's curves (for valuation and forward fixings), bond prices, and the FTP
//! publications.

use std::collections::BTreeMap;

use alm::curve::Curve;
use chrono::NaiveDate;
use domain::FtpCurve;

/// BHD per unit of a currency, by day; the latest on or before a date is used.
pub type Spots = BTreeMap<NaiveDate, BTreeMap<String, f64>>;

/// A curve's rate at a term in days (the market and FTP curves are kept by tenor in days).
pub fn curve_rate(c: &Curve, days: i64) -> f64 {
    c.rate(days.max(1) as f64 / 365.0)
}

fn shifted(c: &Curve, bp: f64) -> Curve {
    Curve { ccy: c.ccy.clone(), points: c.points.iter().map(|(t, r)| (*t, r + bp / 100.0)).collect() }
}

/// FTP publications by date: (publication id, curve per currency).
#[derive(Debug, Clone, Default)]
pub struct Ftp {
    pub by_date: BTreeMap<NaiveDate, (i32, BTreeMap<String, Curve>)>,
}

impl Ftp {
    pub fn from_publications(pubs: impl IntoIterator<Item = (NaiveDate, i32, Vec<FtpCurve>)>) -> Ftp {
        let mut by_date = BTreeMap::new();
        for (d, id, curves) in pubs {
            let m: BTreeMap<String, Curve> = curves
                .into_iter()
                .map(|c| {
                    let mut points: Vec<(f64, f64)> = c.points.iter().map(|p| (p.tenor_days as f64 / 365.0, p.rate)).collect();
                    points.sort_by(|a, b| a.0.total_cmp(&b.0));
                    (c.ccy.clone(), Curve { ccy: c.ccy, points })
                })
                .collect();
            by_date.insert(d, (id, m));
        }
        Ftp { by_date }
    }

    /// The rate for funds of `term_days` in `ccy` from the publication in force on `on` (the latest
    /// on or before it), with that publication's id. `None` when nothing was published by then or
    /// the publication has no curve for the currency.
    pub fn rate(&self, ccy: &str, on: NaiveDate, term_days: i64) -> Option<(f64, i32)> {
        let (_, (id, curves)) = self.by_date.range(..=on).next_back()?;
        curves.get(ccy).map(|c| (curve_rate(c, term_days), *id))
    }
}

#[derive(Debug, Clone, Default)]
pub struct Market {
    pub as_of: NaiveDate,
    pub spots: Spots,
    /// Market curves by currency and day, for fixings on past dates.
    pub history: BTreeMap<String, BTreeMap<NaiveDate, Curve>>,
    /// Today's curve per currency: valuation, and forward fixings for future dates.
    pub today: BTreeMap<String, Curve>,
    /// Clean price and yield by bond and day.
    pub prices: BTreeMap<String, BTreeMap<NaiveDate, (f64, f64)>>,
    pub ftp: Ftp,
}

impl Market {
    pub fn spot(&self, ccy: &str, on: NaiveDate) -> f64 {
        if ccy == "BHD" {
            return 1.0;
        }
        let hit = self.spots.range(..=on).rev().find_map(|(_, m)| m.get(ccy).copied());
        hit.or_else(|| self.spots.values().find_map(|m| m.get(ccy).copied())).unwrap_or(0.0)
    }

    /// Today's curve for a currency moved by `bp`; a flat 4% when none is held.
    pub fn curve(&self, ccy: &str, bp: f64) -> Curve {
        let base = self.today.get(ccy).cloned().unwrap_or_else(|| Curve::flat(ccy, 4.0));
        if bp == 0.0 { base } else { shifted(&base, bp) }
    }

    /// The fixing of an index of `tenor_days` on `on`: the close of that day (or the nearest held)
    /// for a past date, the forward from today's curve for a future one.
    pub fn fixing(&self, ccy: &str, on: NaiveDate, tenor_days: i64) -> f64 {
        if on <= self.as_of
            && let Some(h) = self.history.get(ccy)
        {
            let c = h.range(..=on).next_back().or_else(|| h.iter().next()).map(|(_, c)| c);
            if let Some(c) = c {
                return curve_rate(c, tenor_days);
            }
        }
        let c = self.curve(ccy, 0.0);
        let t1 = (on - self.as_of).num_days().max(0) as f64 / 365.0;
        let t2 = t1 + tenor_days.max(1) as f64 / 365.0;
        let (d1, d2) = (c.df(t1), c.df(t2));
        if d2 <= 0.0 {
            return c.rate(t2);
        }
        (d1 / d2 - 1.0) / (tenor_days.max(1) as f64 / 360.0) * 100.0
    }

    /// Clean price and yield of a bond on `on` (the latest held on or before it).
    pub fn price(&self, isin: &str, on: NaiveDate) -> Option<(f64, f64)> {
        self.prices.get(isin)?.range(..=on).next_back().map(|(_, p)| *p)
    }
}
