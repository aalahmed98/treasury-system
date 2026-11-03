//! Bahrain business days: Sunday to Thursday, minus holidays. The only place business-day
//! arithmetic lives; SQL never computes it.

use std::collections::BTreeSet;

use chrono::{Datelike, Duration, NaiveDate, Weekday};

#[derive(Debug, Clone, Default)]
pub struct Calendar {
    holidays: BTreeSet<NaiveDate>,
}

impl Calendar {
    pub fn new(holidays: impl IntoIterator<Item = NaiveDate>) -> Self {
        Self { holidays: holidays.into_iter().collect() }
    }

    pub fn is_business_day(&self, d: NaiveDate) -> bool {
        !matches!(d.weekday(), Weekday::Fri | Weekday::Sat) && !self.holidays.contains(&d)
    }

    /// `d` itself if it is a business day, otherwise the next one.
    pub fn roll_forward(&self, mut d: NaiveDate) -> NaiveDate {
        while !self.is_business_day(d) {
            d += Duration::days(1);
        }
        d
    }

    /// Move `n` business days (negative moves back). Starts counting from `d` itself when it is
    /// not a business day, so `add_business_days(sat, 1)` is the following Sunday.
    pub fn add_business_days(&self, d: NaiveDate, n: i64) -> NaiveDate {
        let step = if n >= 0 { 1 } else { -1 };
        let mut left = n.abs();
        let mut cur = d;
        while left > 0 {
            cur += Duration::days(step);
            if self.is_business_day(cur) {
                left -= 1;
            }
        }
        cur
    }

    /// Number of business days strictly after `a` up to and including `b` (negative if b < a).
    pub fn business_days_between(&self, a: NaiveDate, b: NaiveDate) -> i64 {
        if b >= a {
            let mut n = 0;
            let mut cur = a;
            while cur < b {
                cur += Duration::days(1);
                if self.is_business_day(cur) {
                    n += 1;
                }
            }
            n
        } else {
            -self.business_days_between(b, a)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn weekend_is_friday_and_saturday() {
        let c = Calendar::default();
        assert!(c.is_business_day(d("2026-11-05"))); // Thursday
        assert!(!c.is_business_day(d("2026-11-06"))); // Friday
        assert!(!c.is_business_day(d("2026-11-07"))); // Saturday
        assert!(c.is_business_day(d("2026-11-08"))); // Sunday
    }

    #[test]
    fn arithmetic_skips_weekends_and_holidays() {
        let c = Calendar::new([d("2026-12-16"), d("2026-12-17")]);
        assert_eq!(c.add_business_days(d("2026-11-05"), 1), d("2026-11-08"), "Thu + 1 = Sun");
        assert_eq!(c.add_business_days(d("2026-11-08"), -1), d("2026-11-05"), "Sun - 1 = Thu");
        assert_eq!(c.add_business_days(d("2026-11-07"), 1), d("2026-11-08"), "from a Saturday");
        assert_eq!(c.add_business_days(d("2026-12-15"), 1), d("2026-12-20"), "Tue 15th + 1 skips two holidays and the weekend");
        assert_eq!(c.roll_forward(d("2026-11-06")), d("2026-11-08"));
        assert_eq!(c.business_days_between(d("2026-11-05"), d("2026-11-10")), 3);
        assert_eq!(c.business_days_between(d("2026-11-10"), d("2026-11-05")), -3);
        assert_eq!(c.business_days_between(d("2026-11-05"), d("2026-11-05")), 0);
    }
}
