//! Interest rate differential: the forward implied by two money-market rates under covered
//! interest parity, and the rate implied by a forward. Rates in percent, simple interest on the
//! currency's day basis.

use domain::IrdQuote;

pub fn pip_factor(quote_ccy: &str) -> f64 {
    if quote_ccy == "JPY" { 100.0 } else { 10_000.0 }
}

pub fn basis(ccy: &str) -> f64 {
    if ccy == "GBP" { 365.0 } else { 360.0 }
}

/// Outright forward for a pair quoted as units of the quote currency per one base unit.
pub fn forward(spot: f64, days: i64, base_rate: f64, base_basis: f64, quote_rate: f64, quote_basis: f64) -> f64 {
    let d = days.max(0) as f64;
    spot * (1.0 + quote_rate / 100.0 * d / quote_basis) / (1.0 + base_rate / 100.0 * d / base_basis)
}

/// The quote-currency rate that makes `forward` fair, given the base rate.
pub fn implied_quote_rate(spot: f64, forward: f64, days: i64, base_rate: f64, base_basis: f64, quote_basis: f64) -> Option<f64> {
    let d = days as f64;
    if spot <= 0.0 || d <= 0.0 {
        return None;
    }
    Some(((forward / spot) * (1.0 + base_rate / 100.0 * d / base_basis) - 1.0) * quote_basis / d * 100.0)
}

/// The base-currency rate that makes `forward` fair, given the quote rate.
pub fn implied_base_rate(spot: f64, forward: f64, days: i64, quote_rate: f64, quote_basis: f64, base_basis: f64) -> Option<f64> {
    let d = days as f64;
    if forward <= 0.0 || d <= 0.0 {
        return None;
    }
    Some(((spot / forward) * (1.0 + quote_rate / 100.0 * d / quote_basis) - 1.0) * base_basis / d * 100.0)
}

/// Daily carry in the quote currency of holding `notional` of the base currency funded in the
/// quote currency: positive when the base currency yields more.
pub fn carry_per_day(notional: f64, spot: f64, base_rate: f64, base_basis: f64, quote_rate: f64, quote_basis: f64) -> f64 {
    notional * spot * (base_rate / 100.0 / base_basis - quote_rate / 100.0 / quote_basis)
}

/// A forward and its points for one tenor.
pub fn quote(tenor: &str, days: i64, spot: f64, base_rate: f64, base_ccy: &str, quote_rate: f64, quote_ccy: &str) -> IrdQuote {
    let f = forward(spot, days, base_rate, basis(base_ccy), quote_rate, basis(quote_ccy));
    IrdQuote { tenor: tenor.into(), days, base_rate, quote_rate, forward: f, points: (f - spot) * pip_factor(quote_ccy) }
}
