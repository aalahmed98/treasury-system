//! Expected-lag maths: which sources a deal is expected in, by when, and which are overdue.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use domain::{Product, Source};

use crate::calendar::Calendar;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarKind {
    Clock,
    BusinessDays,
}

#[derive(Debug, Clone)]
pub struct Expectation {
    pub from: Source,
    pub to: Source,
    /// None means every product.
    pub product: Option<Product>,
    pub within: Duration,
    pub calendar: CalendarKind,
    pub grace: Duration,
}

#[derive(Debug, Clone)]
pub struct LinkedSource {
    pub source: Source,
    /// When the source first reported the deal (its own clock).
    pub source_ts: DateTime<Utc>,
}

/// For every source the deal is expected in, the earliest deadline implied by the linked sources.
pub fn deadlines(
    links: &[LinkedSource],
    product: Product,
    expectations: &[Expectation],
    cal: &Calendar,
) -> BTreeMap<Source, DateTime<Utc>> {
    let mut out: BTreeMap<Source, DateTime<Utc>> = BTreeMap::new();
    for e in expectations {
        if e.product.is_some_and(|p| p != product) {
            continue;
        }
        for l in links.iter().filter(|l| l.source == e.from) {
            let deadline = match e.calendar {
                CalendarKind::Clock => l.source_ts + e.within + e.grace,
                CalendarKind::BusinessDays => {
                    let day = cal.add_business_days(l.source_ts.date_naive(), e.within.num_days());
                    day.and_hms_opt(23, 59, 59).expect("valid time").and_utc() + e.grace
                }
            };
            out.entry(e.to).and_modify(|d| *d = (*d).min(deadline)).or_insert(deadline);
        }
    }
    out
}

/// Sources that are expected, not yet linked, and past their deadline.
pub fn overdue(deadlines: &BTreeMap<Source, DateTime<Utc>>, linked: &[Source], now: DateTime<Utc>) -> Vec<(Source, DateTime<Utc>)> {
    deadlines
        .iter()
        .filter(|(to, deadline)| !linked.contains(to) && **deadline < now)
        .map(|(to, d)| (*to, *d))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exps() -> Vec<Expectation> {
        let g = Duration::seconds(60);
        vec![
            Expectation { from: Source::Bloomberg, to: Source::Kondor, product: None, within: Duration::minutes(5), calendar: CalendarKind::Clock, grace: g },
            Expectation { from: Source::Internal, to: Source::Kondor, product: None, within: Duration::minutes(10), calendar: CalendarKind::Clock, grace: g },
            Expectation { from: Source::Kondor, to: Source::Quantum, product: None, within: Duration::minutes(30), calendar: CalendarKind::Clock, grace: g },
        ]
    }

    fn t(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn expectations_are_directional_and_take_the_earliest_deadline() {
        let cal = Calendar::default();
        let links = vec![
            LinkedSource { source: Source::Bloomberg, source_ts: t("2026-10-05T08:00:00Z") },
            LinkedSource { source: Source::Internal, source_ts: t("2026-10-05T08:02:00Z") },
        ];
        let d = deadlines(&links, Product::FxSpot, &exps(), &cal);
        assert_eq!(d.len(), 1, "only Kondor is expected: Quantum follows Kondor, which is not linked yet");
        assert_eq!(d[&Source::Kondor], t("2026-10-05T08:06:00Z"), "Bloomberg's 5 min + grace wins over Internal's 10 min");

        let linked = vec![Source::Bloomberg, Source::Internal];
        assert!(overdue(&d, &linked, t("2026-10-05T08:05:59Z")).is_empty());
        assert_eq!(overdue(&d, &linked, t("2026-10-05T08:06:01Z")), vec![(Source::Kondor, t("2026-10-05T08:06:00Z"))]);
        assert!(overdue(&d, &[Source::Bloomberg, Source::Internal, Source::Kondor], t("2026-10-05T09:00:00Z")).is_empty(), "linked is never overdue");
    }

    #[test]
    fn business_day_expectations_end_at_close_of_the_target_day() {
        let cal = Calendar::default();
        let exp = vec![Expectation {
            from: Source::Kondor,
            to: Source::Quantum,
            product: Some(Product::MmDepo),
            within: Duration::days(1),
            calendar: CalendarKind::BusinessDays,
            grace: Duration::zero(),
        }];
        let links = vec![LinkedSource { source: Source::Kondor, source_ts: t("2026-10-08T15:00:00Z") }]; // Thursday
        let d = deadlines(&links, Product::MmDepo, &exp, &cal);
        assert_eq!(d[&Source::Quantum], t("2026-10-11T23:59:59Z"), "next business day after Thursday is Sunday");
        assert!(deadlines(&links, Product::FxSpot, &exp, &cal).is_empty(), "product-specific expectation");
    }
}
