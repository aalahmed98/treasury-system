//! What counts as "the same number" across sources.

use domain::Product;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

#[derive(Debug, Clone)]
pub struct Tolerances {
    /// FX rate tolerance in pips (a pip is 0.0001 for every pair we trade).
    pub fx_pips: Decimal,
    /// Money-market rate tolerance in percentage points (1 bp = 0.01).
    pub mm_rate: Decimal,
    /// Relative notional tolerance, to absorb rounding when a reversed quote is re-expressed.
    pub notional_rel: Decimal,
    /// Value date may differ by this many business days.
    pub value_date_bdays: i64,
}

impl Default for Tolerances {
    fn default() -> Self {
        Self {
            fx_pips: Decimal::new(5, 1),          // 0.5 pip
            mm_rate: Decimal::new(1, 2),          // 1 bp
            notional_rel: Decimal::new(1, 4),     // 0.01 %
            value_date_bdays: 1,
        }
    }
}

pub const PIP: Decimal = Decimal::from_parts(1, 0, 0, false, 4); // 0.0001

impl Tolerances {
    pub fn rate_within(&self, product: Product, a: Decimal, b: Decimal) -> bool {
        let limit = if product.is_fx() { self.fx_pips * PIP } else { self.mm_rate };
        (a - b).abs() <= limit
    }

    pub fn notional_within(&self, a: Decimal, b: Decimal) -> bool {
        let scale = a.abs().max(b.abs());
        (a - b).abs() <= scale * self.notional_rel
    }

    /// Distance in pips (FX) or basis points (MM), for scoring.
    pub fn rate_distance(&self, product: Product, a: Decimal, b: Decimal) -> f64 {
        let unit = if product.is_fx() { PIP } else { Decimal::new(1, 2) };
        ((a - b).abs() / unit).to_f64().unwrap_or(f64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[test]
    fn fx_half_pip_and_mm_one_bp() {
        let t = Tolerances::default();
        assert!(t.rate_within(Product::FxSpot, dec("0.37600"), dec("0.37604")));
        assert!(!t.rate_within(Product::FxSpot, dec("0.37600"), dec("0.37606")));
        assert!(t.rate_within(Product::MmDepo, dec("5.25"), dec("5.26")));
        assert!(!t.rate_within(Product::MmDepo, dec("5.25"), dec("5.27")));
    }

    #[test]
    fn notional_relative() {
        let t = Tolerances::default();
        assert!(t.notional_within(dec("1000000"), dec("1000050")));
        assert!(!t.notional_within(dec("1000000"), dec("1000200")));
        assert!(!t.notional_within(dec("1000000"), dec("10000000")));
    }
}
