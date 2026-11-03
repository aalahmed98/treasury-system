//! The pure matching decision for an observation that is not already linked by its own
//! reference (tier 0) or by a cross reference (tier 1). Deterministic and tiered: strict pass,
//! relaxed pass, otherwise create / wait / ambiguous. Scores only rank suggestions.

use chrono::NaiveDate;
use domain::{DealStatus, LinkKind, Product, Side, Source};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::calendar::Calendar;
use crate::normalise::NormalisedDeal;
use crate::tolerance::Tolerances;

#[derive(Debug, Clone)]
pub struct Observation {
    pub source: Source,
    pub source_ref: String,
    pub norm: NormalisedDeal,
}

/// A golden deal from the candidate bucket, with the sources already linked to it.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub deal_id: Uuid,
    pub deal_ref: String,
    pub product: Product,
    pub side: Side,
    pub ccy1: String,
    pub ccy2: Option<String>,
    pub notional: Decimal,
    pub rate: Decimal,
    pub trade_date: NaiveDate,
    pub value_date: NaiveDate,
    /// Canonical form (the golden counterparty run through the same normaliser).
    pub counterparty: String,
    pub status: DealStatus,
    pub linked_sources: Vec<Source>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoredCandidate {
    pub deal_id: Uuid,
    pub deal_ref: String,
    pub score: i16,
    pub strict: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Link { deal_id: Uuid, kind: LinkKind, score: i16 },
    /// No candidate; the source may originate, so a provisional golden deal is created.
    Create,
    /// No candidate and the source may not originate; wait for one (orphan after the grace window).
    Unlinked,
    Ambiguous { candidates: Vec<ScoredCandidate> },
    /// The unique candidate already carries a link from this source: a second booking in one system.
    Duplicate { deal_id: Uuid },
}

pub struct Matcher {
    pub tol: Tolerances,
    pub cal: Calendar,
}

impl Matcher {
    pub fn new(tol: Tolerances, cal: Calendar) -> Self {
        Self { tol, cal }
    }

    /// Inclusive value-date window used both by the SQL bucket query and by `decide`.
    pub fn value_date_bounds(&self, v: NaiveDate) -> (NaiveDate, NaiveDate) {
        let n = self.tol.value_date_bdays;
        (self.cal.add_business_days(v, -n), self.cal.add_business_days(v, n))
    }

    pub fn decide(&self, obs: &Observation, candidates: &[Candidate], may_originate: bool) -> Decision {
        let viable: Vec<(&Candidate, ScoredCandidate)> =
            candidates.iter().filter(|c| self.viable(obs, c)).map(|c| (c, self.score(obs, c))).collect();

        let strict: Vec<&(&Candidate, ScoredCandidate)> = viable.iter().filter(|(_, s)| s.strict).collect();
        if let Some(d) = self.pick(obs, &strict) {
            return d;
        }
        let all: Vec<&(&Candidate, ScoredCandidate)> = viable.iter().collect();
        if let Some(d) = self.pick(obs, &all) {
            return d;
        }
        if may_originate { Decision::Create } else { Decision::Unlinked }
    }

    fn pick(&self, obs: &Observation, set: &[&(&Candidate, ScoredCandidate)]) -> Option<Decision> {
        match set {
            [] => None,
            [(c, s)] => Some(if c.linked_sources.contains(&obs.source) {
                Decision::Duplicate { deal_id: c.deal_id }
            } else {
                Decision::Link { deal_id: c.deal_id, kind: LinkKind::Auto, score: s.score }
            }),
            many => {
                let mut candidates: Vec<ScoredCandidate> = many.iter().map(|(_, s)| s.clone()).collect();
                candidates.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.deal_ref.cmp(&b.deal_ref)));
                Some(Decision::Ambiguous { candidates })
            }
        }
    }

    /// Hard filters, re-applied here so the decision does not trust the bucket query blindly.
    fn viable(&self, obs: &Observation, c: &Candidate) -> bool {
        let n = &obs.norm;
        let (lo, hi) = self.value_date_bounds(n.value_date);
        c.status != DealStatus::Cancelled
            && c.product == n.product
            && c.side == n.side
            && c.ccy1 == n.ccy1
            && c.ccy2 == n.ccy2
            && c.trade_date == n.trade_date
            && self.tol.notional_within(c.notional, n.notional)
            && self.tol.rate_within(n.product, c.rate, n.rate)
            && c.counterparty == n.counterparty
            && (lo..=hi).contains(&c.value_date)
    }

    fn score(&self, obs: &Observation, c: &Candidate) -> ScoredCandidate {
        let n = &obs.norm;
        let strict = c.rate == n.rate && c.value_date == n.value_date && c.notional == n.notional;
        let rate_pen = (self.tol.rate_distance(n.product, c.rate, n.rate) * 20.0).min(40.0);
        let date_pen = (self.cal.business_days_between(n.value_date, c.value_date).unsigned_abs() as f64 * 15.0).min(30.0);
        let notional_pen = if c.notional == n.notional { 0.0 } else { 10.0 };
        let score = (100.0 - rate_pen - date_pen - notional_pen).round() as i16;
        ScoredCandidate { deal_id: c.deal_id, deal_ref: c.deal_ref.clone(), score, strict }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn dec(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn norm() -> NormalisedDeal {
        NormalisedDeal {
            product: Product::FxSpot,
            side: Side::Buy,
            ccy1: "USD".into(),
            ccy2: Some("BHD".into()),
            notional: dec("1000000"),
            rate: dec("0.37600"),
            trade_date: d("2026-10-05"),
            value_date: d("2026-10-07"),
            maturity_date: None,
            counterparty_raw: "GIB".into(),
            counterparty: "GULF INTERNATIONAL BANK".into(),
            trader_ref: None,
            lifecycle: DealStatus::Executed,
            raw_state: None,
            desk_code: None,
            desk_id: None,
            cross_refs: BTreeMap::new(),
            self_deal_id: None,
        }
    }

    fn obs() -> Observation {
        Observation { source: Source::Bloomberg, source_ref: "VCON1".into(), norm: norm() }
    }

    fn cand(r: &str) -> Candidate {
        let n = norm();
        Candidate {
            deal_id: Uuid::now_v7(),
            deal_ref: r.into(),
            product: n.product,
            side: n.side,
            ccy1: n.ccy1,
            ccy2: n.ccy2,
            notional: n.notional,
            rate: n.rate,
            trade_date: n.trade_date,
            value_date: n.value_date,
            counterparty: n.counterparty,
            status: DealStatus::Booked,
            linked_sources: vec![Source::Kondor],
        }
    }

    fn matcher() -> Matcher {
        Matcher::new(Tolerances::default(), Calendar::default())
    }

    #[test]
    fn exact_single_candidate_links() {
        let c = cand("FX-1");
        match matcher().decide(&obs(), &[c.clone()], true) {
            Decision::Link { deal_id, kind, score } => {
                assert_eq!(deal_id, c.deal_id);
                assert_eq!(kind, LinkKind::Auto);
                assert_eq!(score, 100);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn drift_within_tolerance_links_with_lower_score() {
        let mut c = cand("FX-1");
        c.rate = dec("0.37604"); // 0.4 pip
        c.value_date = d("2026-10-08"); // one business day later
        match matcher().decide(&obs(), &[c], true) {
            Decision::Link { score, .. } => assert!(score < 100 && score > 50, "score {score}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn strict_candidate_beats_relaxed_ones() {
        let exact = cand("FX-EXACT");
        let mut near = cand("FX-NEAR");
        near.rate = dec("0.37603");
        match matcher().decide(&obs(), &[near, exact.clone()], true) {
            Decision::Link { deal_id, .. } => assert_eq!(deal_id, exact.deal_id),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn two_exact_candidates_are_ambiguous_and_ranked() {
        let a = cand("FX-A");
        let b = cand("FX-B");
        match matcher().decide(&obs(), &[b, a], true) {
            Decision::Ambiguous { candidates } => {
                assert_eq!(candidates.len(), 2);
                assert_eq!(candidates[0].deal_ref, "FX-A", "ties break on reference");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn out_of_tolerance_is_not_a_candidate() {
        let m = matcher();
        let mut far = cand("FX-1");
        far.rate = dec("0.37700");
        assert_eq!(m.decide(&obs(), &[far], true), Decision::Create);
        assert_eq!(m.decide(&obs(), &[], false), Decision::Unlinked);
        let mut other_cpty = cand("FX-2");
        other_cpty.counterparty = "SOMEONE ELSE".into();
        assert_eq!(m.decide(&obs(), &[other_cpty], false), Decision::Unlinked);
        let mut cancelled = cand("FX-3");
        cancelled.status = DealStatus::Cancelled;
        assert_eq!(m.decide(&obs(), &[cancelled], true), Decision::Create);
    }

    #[test]
    fn second_observation_from_the_same_source_is_a_duplicate() {
        let mut c = cand("FX-1");
        c.linked_sources.push(Source::Bloomberg);
        assert_eq!(matcher().decide(&obs(), &[c.clone()], true), Decision::Duplicate { deal_id: c.deal_id });
    }

    #[test]
    fn value_date_window_respects_the_weekend() {
        let m = matcher();
        // Thursday 2026-10-08: one business day either side is Wed 7th and Sun 11th.
        assert_eq!(m.value_date_bounds(d("2026-10-08")), (d("2026-10-07"), d("2026-10-11")));
    }
}
