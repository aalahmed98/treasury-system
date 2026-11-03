//! The reconciliation engine: turns the event log into the golden record, one batch at a time.
//! Every batch is one transaction; every event gets a savepoint so a bad payload cannot poison
//! its neighbours. All effects and the processed mark commit together.

use std::collections::{BTreeSet, HashMap};
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, bail};
use chrono::{DateTime, Duration, Utc};
use domain::{BreakKind, DealStatus, EventKind, LinkKind, MatchStatus, Resolution, Source};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::{Connection, PgConnection, PgPool};
use uuid::Uuid;

use crate::calendar::Calendar;
use crate::clock::Clock;
use crate::matcher::{Candidate, Decision, Matcher, Observation};
use crate::normalise::{NormalisedDeal, Normaliser};
use crate::sla::{self, CalendarKind, Expectation, LinkedSource};
use crate::store::{self, DealRow, EventRow, GoldenUpdate, SourceDealRow, SourceSystem};
use crate::tolerance::Tolerances;

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub batch_size: i64,
    /// An unlinked observation from a source that cannot originate becomes an ORPHAN after this.
    pub orphan_grace: Duration,
    /// A feed whose heartbeat is older than this is not expected to deliver; no MISSING breaks.
    pub feed_stale_after: Duration,
    /// Only deals traded within this many days are checked for missing sources.
    pub lookback_days: i64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            batch_size: 200,
            orphan_grace: Duration::minutes(45),
            feed_stale_after: Duration::minutes(5),
            lookback_days: 30,
        }
    }
}

pub struct Engine {
    pool: PgPool,
    clock: Arc<dyn Clock>,
    normaliser: Normaliser,
    matcher: Matcher,
    expectations: Vec<Expectation>,
    systems: HashMap<Source, SourceSystem>,
    desk_map: HashMap<(Source, String), i16>,
    desk_codes: HashMap<i16, String>,
    unassigned: i16,
    pub cfg: EngineConfig,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SlaReport {
    pub missing_raised: usize,
    pub orphans_raised: usize,
    pub gated_by_stale_feed: usize,
    pub stale_feeds: Vec<Source>,
    pub pending: i64,
}

struct Outcome {
    name: &'static str,
    detail: Value,
    norm: Option<NormalisedDeal>,
    desk: Option<i16>,
}

impl Engine {
    pub async fn load(pool: PgPool, clock: Arc<dyn Clock>, cfg: EngineConfig) -> anyhow::Result<Self> {
        let aliases: Vec<(String, String)> =
            sqlx::query_as("SELECT alias_norm, canonical FROM counterparty_aliases").fetch_all(&pool).await?;
        let holidays: Vec<chrono::NaiveDate> = sqlx::query_scalar("SELECT day FROM calendar_holidays").fetch_all(&pool).await?;
        let exps: Vec<(String, String, String, f64, String, f64)> = sqlx::query_as(
            "SELECT from_source, to_source, product, extract(epoch FROM expected_within)::float8, calendar, extract(epoch FROM grace)::float8 \
             FROM sla_expectations",
        )
        .fetch_all(&pool)
        .await?;
        let expectations = exps
            .into_iter()
            .map(|(f, t, p, w, c, g)| {
                Ok(Expectation {
                    from: Source::from_str(&f).map_err(anyhow::Error::msg)?,
                    to: Source::from_str(&t).map_err(anyhow::Error::msg)?,
                    product: if p == "*" { None } else { Some(domain::Product::from_str(&p).map_err(anyhow::Error::msg)?) },
                    within: Duration::seconds(w as i64),
                    calendar: if c == "BUSINESS_DAYS" { CalendarKind::BusinessDays } else { CalendarKind::Clock },
                    grace: Duration::seconds(g as i64),
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let systems = store::load_source_systems(&pool).await?.into_iter().map(|s| (s.code, s)).collect();
        let desks: Vec<(i16, String)> = sqlx::query_as("SELECT id, code FROM desks").fetch_all(&pool).await?;
        let unassigned = desks.iter().find(|(_, c)| c == "UNASSIGNED").map(|(i, _)| *i).context("UNASSIGNED desk missing")?;
        let maps: Vec<(String, String, i16)> =
            sqlx::query_as("SELECT source, external_code, desk_id FROM desk_mappings").fetch_all(&pool).await?;
        let desk_map = maps
            .into_iter()
            .filter_map(|(s, c, d)| Source::from_str(&s).ok().map(|s| ((s, c), d)))
            .collect();
        Ok(Self {
            pool,
            clock,
            normaliser: Normaliser::new(aliases),
            matcher: Matcher::new(Tolerances::default(), Calendar::new(holidays)),
            expectations,
            systems,
            desk_map,
            desk_codes: desks.into_iter().collect(),
            unassigned,
            cfg,
        })
    }

    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    pub fn unassigned_desk(&self) -> i16 {
        self.unassigned
    }

    pub async fn pending(&self) -> anyhow::Result<i64> {
        Ok(sqlx::query_scalar("SELECT count(*) FROM source_events WHERE processed_at IS NULL").fetch_one(&self.pool).await?)
    }

    /// Process one batch; returns how many events were handled.
    pub async fn process_batch(&self) -> anyhow::Result<usize> {
        let mut tx = self.pool.begin().await?;
        let events = store::pending_events(&mut tx, self.cfg.batch_size).await?;
        if events.is_empty() {
            tx.commit().await?;
            return Ok(0);
        }
        for ev in &events {
            let now = self.now();
            let mut sp = tx.begin().await?;
            match self.apply(&mut sp, ev, now).await {
                Ok(o) => {
                    store::mark_event(&mut sp, ev.id, now, o.name, o.detail, o.norm.as_ref(), o.desk).await?;
                    sp.commit().await?;
                }
                Err(e) => {
                    sp.rollback().await?;
                    tracing::warn!(event = ev.id, source = %ev.source, source_ref = %ev.source_ref, error = %e, "event failed");
                    store::mark_event(&mut tx, ev.id, now, "ERROR", json!({ "error": e.to_string() }), None, None).await?;
                }
            }
        }
        tx.commit().await?;
        Ok(events.len())
    }

    pub async fn process_until_idle(&self) -> anyhow::Result<usize> {
        let mut total = 0;
        loop {
            let n = self.process_batch().await?;
            total += n;
            if n == 0 {
                return Ok(total);
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Event application
    // -----------------------------------------------------------------------------------------

    async fn apply(&self, tx: &mut PgConnection, ev: &EventRow, now: DateTime<Utc>) -> anyhow::Result<Outcome> {
        let source = Source::from_str(&ev.source).map_err(anyhow::Error::msg)?;
        if ev.event_kind == "USER" {
            let (name, detail) = self.apply_user(tx, ev, now).await?;
            return Ok(Outcome { name, detail, norm: None, desk: None });
        }
        let norm = self.normaliser.normalise(source, &ev.payload)?;
        let desk_id = self.resolve_desk(source, &norm);
        let existing = store::source_deal(tx, source, &ev.source_ref).await?;
        let (name, detail) = match existing {
            Some(sd) => self.apply_existing(tx, ev, source, &norm, sd, desk_id, now).await?,
            None => self.apply_new(tx, ev, source, &norm, desk_id, now).await?,
        };
        Ok(Outcome { name, detail, norm: Some(norm), desk: Some(desk_id) })
    }

    fn resolve_desk(&self, source: Source, n: &NormalisedDeal) -> i16 {
        n.desk_id
            .or_else(|| n.desk_code.as_ref().and_then(|c| self.desk_map.get(&(source, c.clone())).copied()))
            .unwrap_or(self.unassigned)
    }

    async fn apply_new(
        &self,
        tx: &mut PgConnection,
        ev: &EventRow,
        source: Source,
        norm: &NormalisedDeal,
        desk_id: i16,
        now: DateTime<Utc>,
    ) -> anyhow::Result<(&'static str, Value)> {
        store::insert_source_deal(tx, source, &ev.source_ref, norm, ev, desk_id, now).await?;
        if source == Source::Internal {
            let id = norm.self_deal_id.context("internal event without a deal id")?;
            if store::deal(tx, id).await?.is_none() {
                bail!("internal booking {id} not found");
            }
            store::link_source_deal(tx, source, &ev.source_ref, id, LinkKind::SelfRef, ev.id, None, Some(100), now).await?;
            self.recompute_deal(tx, id, now).await?;
            self.merge_provisionals_into(tx, id, now).await?;
            self.adopt_unlinked(tx, id, now).await?;
            return Ok(("LINKED", json!({ "deal_id": id, "link": "SELF" })));
        }
        // A cancellation as the first image may still attach through a reference, but never creates.
        let allow_create = norm.lifecycle != DealStatus::Cancelled;
        self.try_match(tx, ev.id, source, &ev.source_ref, norm, desk_id, ev.observed_at, now, allow_create, allow_create).await
    }

    async fn apply_existing(
        &self,
        tx: &mut PgConnection,
        ev: &EventRow,
        source: Source,
        norm: &NormalisedDeal,
        sd: SourceDealRow,
        desk_id: i16,
        now: DateTime<Utc>,
    ) -> anyhow::Result<(&'static str, Value)> {
        let newer = match (ev.source_seq, sd.applied_seq) {
            (Some(a), Some(b)) => a > b || (a == b && ev.source_ts > sd.applied_ts),
            _ => ev.source_ts > sd.applied_ts,
        };
        let same_image = same_image(&sd.as_norm(), norm);
        if !newer {
            return Ok((if same_image { "DUP" } else { "STALE" }, json!({ "applied_event_id": sd.applied_event_id })));
        }
        store::update_source_deal_image(tx, source, &ev.source_ref, norm, ev, now).await?;
        if same_image {
            return Ok(("DUP", json!({ "applied_event_id": sd.applied_event_id, "resent": true })));
        }
        match sd.deal_id {
            Some(d) => {
                self.recompute_deal(tx, d, now).await?;
                let name = if norm.lifecycle == DealStatus::Cancelled { "CANCELLED" } else { "AMENDED" };
                Ok((name, json!({ "deal_id": d })))
            }
            None if norm.lifecycle == DealStatus::Cancelled => Ok(("CANCELLED", json!({ "unlinked": true }))),
            None => self.try_match(tx, ev.id, source, &ev.source_ref, norm, desk_id, ev.observed_at, now, true, true).await,
        }
    }

    /// Tiers 1 and 2 for an observation that is not linked by its own reference.
    #[allow(clippy::too_many_arguments)]
    async fn try_match(
        &self,
        tx: &mut PgConnection,
        event_id: i64,
        source: Source,
        source_ref: &str,
        norm: &NormalisedDeal,
        desk_id: i16,
        observed_at: DateTime<Utc>,
        now: DateTime<Utc>,
        allow_create: bool,
        adopt: bool,
    ) -> anyhow::Result<(&'static str, Value)> {
        // Tier 1: a cross reference to a source row that is already linked, in either direction.
        let mut by_ref: Option<Uuid> = None;
        for (src, r) in &norm.cross_refs {
            if let Some(other) = store::source_deal(tx, *src, r).await? {
                if let Some(d) = other.deal_id {
                    by_ref = Some(d);
                    break;
                }
            }
        }
        if by_ref.is_none() {
            by_ref = store::deal_by_reverse_cross_ref(tx, source, source_ref).await?;
        }
        if let Some(d) = by_ref {
            return self.link_or_duplicate(tx, source, source_ref, d, LinkKind::CrossRef, event_id, None, now, adopt).await;
        }

        // Tier 2: the economic bucket.
        let (nlo, nhi) = self.notional_bounds(norm.notional);
        let (vlo, vhi) = self.matcher.value_date_bounds(norm.value_date);
        let bucket = store::bucket_deals(tx, norm, nlo, nhi, vlo, vhi).await?;
        let candidates: Vec<Candidate> = bucket
            .into_iter()
            .map(|(d, linked)| Candidate {
                deal_id: d.id,
                deal_ref: d.deal_ref.clone(),
                product: d.product(),
                side: domain::Side::from_str(&d.side).expect("constrained"),
                ccy1: d.ccy1.trim().to_string(),
                ccy2: d.ccy2.as_deref().map(|c| c.trim().to_string()),
                notional: d.notional,
                rate: d.rate,
                trade_date: d.trade_date,
                value_date: d.value_date,
                counterparty: self.normaliser.canonical_counterparty(&d.counterparty),
                status: d.status(),
                linked_sources: linked.iter().filter_map(|s| Source::from_str(s).ok()).collect(),
            })
            .collect();
        let may_originate = allow_create && self.systems.get(&source).is_some_and(|s| s.may_originate);
        let obs = Observation { source, source_ref: source_ref.to_string(), norm: norm.clone() };
        match self.matcher.decide(&obs, &candidates, may_originate) {
            Decision::Link { deal_id, kind, score } => {
                store::link_source_deal(tx, source, source_ref, deal_id, kind, event_id, None, Some(score), now).await?;
                self.recompute_deal(tx, deal_id, now).await?;
                if adopt {
                    self.adopt_unlinked(tx, deal_id, now).await?;
                }
                Ok(("LINKED", json!({ "deal_id": deal_id, "score": score })))
            }
            Decision::Create => {
                let desk_code = self.desk_codes.get(&desk_id).cloned().unwrap_or_else(|| "UNA".into());
                let id = store::create_golden(tx, norm, desk_id, &desk_code, source, observed_at).await?;
                store::link_source_deal(tx, source, source_ref, id, LinkKind::Auto, event_id, None, Some(100), now).await?;
                self.recompute_deal(tx, id, now).await?;
                if adopt {
                    self.adopt_unlinked(tx, id, now).await?;
                }
                Ok(("CREATED", json!({ "deal_id": id })))
            }
            Decision::Unlinked => Ok(("WAITING", json!({ "candidates": 0 }))),
            Decision::Ambiguous { candidates } => {
                store::raise_break(
                    tx,
                    BreakKind::Ambiguous,
                    None,
                    Some(source),
                    Some(source_ref),
                    None,
                    desk_id,
                    json!({ "candidates": candidates }),
                    now,
                )
                .await?;
                Ok(("BREAK", json!({ "kind": "AMBIGUOUS", "candidates": candidates.len() })))
            }
            Decision::Duplicate { deal_id } => {
                let desk = store::deal(tx, deal_id).await?.map(|d| d.desk_id).unwrap_or(desk_id);
                store::raise_break(
                    tx,
                    BreakKind::Duplicate,
                    Some(deal_id),
                    Some(source),
                    Some(source_ref),
                    None,
                    desk,
                    json!({ "observed": source_ref }),
                    now,
                )
                .await?;
                self.recompute_deal(tx, deal_id, now).await?;
                Ok(("BREAK", json!({ "kind": "DUPLICATE", "deal_id": deal_id })))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn link_or_duplicate(
        &self,
        tx: &mut PgConnection,
        source: Source,
        source_ref: &str,
        deal_id: Uuid,
        kind: LinkKind,
        event_id: i64,
        by: Option<Uuid>,
        now: DateTime<Utc>,
        adopt: bool,
    ) -> anyhow::Result<(&'static str, Value)> {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT source_ref FROM source_deals WHERE deal_id = $1 AND source = $2")
                .bind(deal_id)
                .bind(source.as_str())
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(other) = existing.filter(|r| r != source_ref) {
            let desk = store::deal(tx, deal_id).await?.map(|d| d.desk_id).unwrap_or(self.unassigned);
            store::raise_break(
                tx,
                BreakKind::Duplicate,
                Some(deal_id),
                Some(source),
                Some(source_ref),
                None,
                desk,
                json!({ "observed": source_ref, "existing": other }),
                now,
            )
            .await?;
            self.recompute_deal(tx, deal_id, now).await?;
            return Ok(("BREAK", json!({ "kind": "DUPLICATE", "deal_id": deal_id })));
        }
        store::link_source_deal(tx, source, source_ref, deal_id, kind, event_id, by, None, now).await?;
        self.recompute_deal(tx, deal_id, now).await?;
        if adopt {
            self.adopt_unlinked(tx, deal_id, now).await?;
        }
        Ok(("LINKED", json!({ "deal_id": deal_id, "link": kind.as_str() })))
    }

    fn notional_bounds(&self, n: Decimal) -> (Decimal, Decimal) {
        let t = self.matcher.tol.notional_rel;
        (n * (Decimal::ONE - t), n * (Decimal::ONE + t))
    }

    // -----------------------------------------------------------------------------------------
    // Golden record maintenance
    // -----------------------------------------------------------------------------------------

    /// Re-derive everything about a golden deal from its linked sources: economics ownership,
    /// status, confirmation, field mismatches, auto-resolved breaks, match status.
    pub async fn recompute_deal(&self, tx: &mut PgConnection, deal_id: Uuid, now: DateTime<Utc>) -> anyhow::Result<()> {
        let Some(deal) = store::deal(tx, deal_id).await? else { bail!("deal {deal_id} vanished") };
        let links = store::source_deals_of(tx, deal_id).await?;
        let precedence = |l: &SourceDealRow| self.systems.get(&l.source()).map(|s| s.precedence).unwrap_or(i16::MAX);

        let owner: Option<&SourceDealRow> = if deal.origin() == Source::Internal {
            links.iter().find(|l| l.source() == Source::Internal)
        } else {
            links.iter().min_by_key(|l| precedence(l))
        };
        let economics: Option<NormalisedDeal> = if deal.origin() == Source::Internal { None } else { owner.map(|o| o.as_norm()) };
        let golden = economics.clone().unwrap_or_else(|| self.deal_as_norm(&deal));

        let has_kondor = links.iter().any(|l| l.source() == Source::Kondor);
        let cancel_authority = |l: &SourceDealRow| {
            self.systems.get(&l.source()).is_some_and(|s| s.cancels)
                || (l.source() == Source::Internal && deal.origin() == Source::Internal && !has_kondor)
        };
        let status = if deal.status().is_terminal() {
            deal.status()
        } else if links.iter().any(|l| l.lifecycle() == DealStatus::Cancelled && cancel_authority(l)) {
            DealStatus::Cancelled
        } else {
            links
                .iter()
                .map(|l| l.lifecycle())
                .filter(|s| *s != DealStatus::Cancelled)
                .max_by_key(|s| s.rank())
                .unwrap_or(deal.status())
        };
        let confirmed = links.iter().any(|l| {
            self.systems.get(&l.source()).is_some_and(|s| s.confirms)
                && l.lifecycle() != DealStatus::Cancelled
                && l.lifecycle().rank() >= DealStatus::Confirmed.rank()
        });

        for l in &links {
            if owner.is_some_and(|o| std::ptr::eq(o, l)) {
                continue;
            }
            self.check_mismatches(tx, &deal, &golden, status, l, now).await?;
        }
        for l in &links {
            store::resolve_missing(tx, deal_id, l.source(), now).await?;
            store::resolve_breaks(tx, Some(BreakKind::Orphan), None, Some(l.source()), Some(&l.source_ref), None, Resolution::Auto, None, json!({}), now)
                .await?;
        }

        let linked: Vec<Source> = links.iter().map(|l| l.source()).collect();
        let infos: Vec<LinkedSource> = links.iter().map(|l| LinkedSource { source: l.source(), source_ts: l.first_seen_at }).collect();
        let expected: BTreeSet<Source> =
            sla::deadlines(&infos, deal.product(), &self.expectations, &self.matcher.cal).keys().copied().collect();
        let open = store::open_break_count(tx, deal_id).await?;
        let match_status = if open > 0 {
            MatchStatus::Break
        } else if linked.len() <= 1 {
            MatchStatus::Unmatched
        } else if expected.iter().all(|s| linked.contains(s)) {
            MatchStatus::Matched
        } else {
            MatchStatus::Partial
        };

        store::update_golden(tx, deal_id, &GoldenUpdate { status, confirmed, match_status, economics: economics.as_ref(), merged_into: None })
            .await?;
        Ok(())
    }

    async fn check_mismatches(
        &self,
        tx: &mut PgConnection,
        deal: &DealRow,
        golden: &NormalisedDeal,
        golden_status: DealStatus,
        l: &SourceDealRow,
        now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let n = l.as_norm();
        let tol = &self.matcher.tol;
        let checks: [(&str, bool, String, String); 10] = [
            ("product", n.product == golden.product, golden.product.to_string(), n.product.to_string()),
            ("side", n.side == golden.side, golden.side.to_string(), n.side.to_string()),
            ("ccy1", n.ccy1 == golden.ccy1, golden.ccy1.clone(), n.ccy1.clone()),
            ("ccy2", n.ccy2 == golden.ccy2, golden.ccy2.clone().unwrap_or_default(), n.ccy2.clone().unwrap_or_default()),
            ("notional", tol.notional_within(n.notional, golden.notional), golden.notional.normalize().to_string(), n.notional.normalize().to_string()),
            ("rate", tol.rate_within(golden.product, n.rate, golden.rate), golden.rate.normalize().to_string(), n.rate.normalize().to_string()),
            ("value_date", n.value_date == golden.value_date, golden.value_date.to_string(), n.value_date.to_string()),
            ("maturity_date", n.maturity_date == golden.maturity_date, fmt_opt(golden.maturity_date), fmt_opt(n.maturity_date)),
            ("counterparty", n.counterparty == golden.counterparty, golden.counterparty.clone(), n.counterparty.clone()),
            (
                "status",
                (n.lifecycle == DealStatus::Cancelled) == (golden_status == DealStatus::Cancelled),
                golden_status.to_string(),
                n.lifecycle.to_string(),
            ),
        ];
        for (field, ok, expected, observed) in checks {
            if ok {
                store::resolve_breaks(
                    tx,
                    Some(BreakKind::Mismatch),
                    Some(deal.id),
                    Some(l.source()),
                    Some(&l.source_ref),
                    Some(field),
                    Resolution::Auto,
                    None,
                    json!({}),
                    now,
                )
                .await?;
            } else {
                store::raise_break(
                    tx,
                    BreakKind::Mismatch,
                    Some(deal.id),
                    Some(l.source()),
                    Some(&l.source_ref),
                    Some(field),
                    deal.desk_id,
                    json!({ "expected": expected, "observed": observed }),
                    now,
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Retry every unlinked observation in the deal's bucket, so the outcome does not depend on
    /// arrival order. Never creates deals.
    pub async fn adopt_unlinked(&self, tx: &mut PgConnection, deal_id: Uuid, now: DateTime<Utc>) -> anyhow::Result<()> {
        let Some(deal) = store::deal(tx, deal_id).await? else { return Ok(()) };
        let (nlo, nhi) = self.notional_bounds(deal.notional);
        let (vlo, vhi) = self.matcher.value_date_bounds(deal.value_date);
        let rows = store::unlinked_in_bucket(tx, &deal, nlo, nhi, vlo, vhi).await?;
        for sd in rows {
            let norm = sd.as_norm();
            // Boxed: adoption calls matching, which may call adoption (never for the same row twice).
            let (name, detail) = Box::pin(self.try_match(
                tx, sd.applied_event_id, sd.source(), &sd.source_ref, &norm, sd.desk_id, sd.first_seen_at, now, false, false,
            ))
            .await?;
            tracing::debug!(source = %sd.source, source_ref = %sd.source_ref, outcome = name, detail = %detail, "adoption");
        }
        Ok(())
    }

    /// An internal booking arrived: fold a provisional golden deal with the same economics into it.
    async fn merge_provisionals_into(&self, tx: &mut PgConnection, internal_id: Uuid, now: DateTime<Utc>) -> anyhow::Result<()> {
        let Some(g) = store::deal(tx, internal_id).await? else { return Ok(()) };
        let gn = self.deal_as_norm(&g);
        let (nlo, nhi) = self.notional_bounds(gn.notional);
        let (vlo, vhi) = self.matcher.value_date_bounds(gn.value_date);
        let bucket = store::bucket_deals(tx, &gn, nlo, nhi, vlo, vhi).await?;
        let candidates: Vec<Candidate> = bucket
            .into_iter()
            .filter(|(d, _)| d.id != internal_id && d.origin() != Source::Internal)
            .map(|(d, linked)| Candidate {
                deal_id: d.id,
                deal_ref: d.deal_ref.clone(),
                product: d.product(),
                side: domain::Side::from_str(&d.side).expect("constrained"),
                ccy1: d.ccy1.trim().to_string(),
                ccy2: d.ccy2.as_deref().map(|c| c.trim().to_string()),
                notional: d.notional,
                rate: d.rate,
                trade_date: d.trade_date,
                value_date: d.value_date,
                counterparty: self.normaliser.canonical_counterparty(&d.counterparty),
                status: d.status(),
                linked_sources: linked.iter().filter_map(|s| Source::from_str(s).ok()).collect(),
            })
            .collect();
        if candidates.is_empty() {
            return Ok(());
        }
        let obs = Observation { source: Source::Internal, source_ref: internal_id.to_string(), norm: gn };
        match self.matcher.decide(&obs, &candidates, false) {
            Decision::Link { deal_id: p, .. } | Decision::Duplicate { deal_id: p } => {
                sqlx::query("UPDATE source_deals SET deal_id = $1, linked_at = $3, updated_at = $3 WHERE deal_id = $2")
                    .bind(internal_id)
                    .bind(p)
                    .bind(now)
                    .execute(&mut *tx)
                    .await?;
                store::resolve_breaks(tx, None, Some(p), None, None, None, Resolution::Superseded, None, json!({ "merged_into": internal_id }), now).await?;
                store::update_golden(
                    tx,
                    p,
                    &GoldenUpdate { status: DealStatus::Cancelled, confirmed: false, match_status: MatchStatus::Matched, economics: None, merged_into: Some(internal_id) },
                )
                .await?;
                self.recompute_deal(tx, internal_id, now).await?;
            }
            Decision::Ambiguous { candidates } => {
                store::raise_break(
                    tx,
                    BreakKind::Ambiguous,
                    Some(internal_id),
                    Some(Source::Internal),
                    Some(&internal_id.to_string()),
                    None,
                    g.desk_id,
                    json!({ "candidates": candidates, "reason": "several provisional deals match this booking" }),
                    now,
                )
                .await?;
                self.recompute_deal(tx, internal_id, now).await?;
            }
            Decision::Create | Decision::Unlinked => {}
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // User actions
    // -----------------------------------------------------------------------------------------

    async fn apply_user(&self, tx: &mut PgConnection, ev: &EventRow, now: DateTime<Utc>) -> anyhow::Result<(&'static str, Value)> {
        let p = &ev.payload;
        let break_id = p["break_id"].as_i64().context("user event without break_id")?;
        let action = p["action"].as_str().context("user event without action")?;
        let user = p["user_id"].as_str().and_then(|s| Uuid::parse_str(s).ok());
        let target = p["deal_id"].as_str().and_then(|s| Uuid::parse_str(s).ok());
        let note = p["note"].as_str().map(str::to_owned);

        let Some(b) = store::break_by_id(tx, break_id).await? else {
            return Ok(("STALE", json!({ "reason": "break not found" })));
        };
        if b.status == "RESOLVED" {
            return Ok(("STALE", json!({ "reason": "already resolved" })));
        }
        match action {
            "DISMISS" => {
                store::resolve_break_by_id(tx, break_id, Resolution::Dismissed, user, note.as_deref(), now).await?;
                if let Some(d) = b.deal_id {
                    self.recompute_deal(tx, d, now).await?;
                }
                Ok(("APPLIED", json!({ "break_id": break_id, "action": "DISMISS" })))
            }
            "LINK" => {
                let target = target.context("LINK without a target deal")?;
                match b.kind.as_str() {
                    "ORPHAN" | "AMBIGUOUS" | "DUPLICATE" => {
                        let src = Source::from_str(b.source.as_deref().unwrap_or("")).map_err(anyhow::Error::msg)?;
                        let r = b.source_ref.clone().context("break without a source reference")?;
                        let already: Option<String> =
                            sqlx::query_scalar("SELECT source_ref FROM source_deals WHERE deal_id = $1 AND source = $2")
                                .bind(target)
                                .bind(src.as_str())
                                .fetch_optional(&mut *tx)
                                .await?;
                        if let Some(other) = already.filter(|o| *o != r) {
                            bail!("deal already linked to {} {other}", src.as_str());
                        }
                        let sd = store::source_deal(tx, src, &r).await?.context("observation vanished")?;
                        let previous = sd.deal_id;
                        store::link_source_deal(tx, src, &r, target, LinkKind::Manual, ev.id, user, None, now).await?;
                        store::resolve_break_by_id(tx, break_id, Resolution::Linked, user, note.as_deref(), now).await?;
                        if let Some(prev) = previous.filter(|p| *p != target) {
                            self.recompute_deal(tx, prev, now).await?;
                        }
                        self.recompute_deal(tx, target, now).await?;
                        self.adopt_unlinked(tx, target, now).await?;
                        Ok(("APPLIED", json!({ "break_id": break_id, "action": "LINK", "deal_id": target })))
                    }
                    "MISSING" => bail!("a MISSING break cannot be linked; link the orphan observation instead"),
                    _ => bail!("a {} break can only be dismissed", b.kind),
                }
            }
            other => bail!("unknown action {other}"),
        }
    }

    // -----------------------------------------------------------------------------------------
    // SLA tick
    // -----------------------------------------------------------------------------------------

    pub async fn sla_tick(&self) -> anyhow::Result<SlaReport> {
        let now = self.now();
        let mut report = SlaReport::default();
        let mut tx = self.pool.begin().await?;

        let feeds: Vec<(String, Option<DateTime<Utc>>)> =
            sqlx::query_as("SELECT source, last_heartbeat_at FROM feed_status").fetch_all(&mut *tx).await?;
        report.stale_feeds = feeds
            .into_iter()
            .filter_map(|(s, hb)| {
                let src = Source::from_str(&s).ok()?;
                hb.filter(|h| now - *h > self.cfg.feed_stale_after).map(|_| src)
            })
            .collect();

        let since = now.date_naive() - Duration::days(self.cfg.lookback_days);
        let deals: Vec<DealRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM deals WHERE match_status IN ('UNMATCHED', 'PARTIAL') AND merged_into IS NULL \
               AND status <> 'CANCELLED' AND trade_date >= $1 ORDER BY trade_date",
            store::DEAL_COLS
        )))
        .bind(since)
        .fetch_all(&mut *tx)
        .await?;
        for d in deals {
            let links = store::source_deals_of(&mut tx, d.id).await?;
            let linked: Vec<Source> = links.iter().map(|l| l.source()).collect();
            let infos: Vec<LinkedSource> = links.iter().map(|l| LinkedSource { source: l.source(), source_ts: l.first_seen_at }).collect();
            let deadlines = sla::deadlines(&infos, d.product(), &self.expectations, &self.matcher.cal);
            let mut raised = false;
            for (to, deadline) in sla::overdue(&deadlines, &linked, now) {
                if report.stale_feeds.contains(&to) {
                    report.gated_by_stale_feed += 1;
                    continue;
                }
                let id = store::raise_break(
                    &mut tx,
                    BreakKind::Missing,
                    Some(d.id),
                    Some(to),
                    None,
                    None,
                    d.desk_id,
                    json!({ "deadline": deadline, "linked": linked.iter().map(|s| s.as_str()).collect::<Vec<_>>() }),
                    now,
                )
                .await?;
                if id.is_some() {
                    report.missing_raised += 1;
                    raised = true;
                }
            }
            if raised {
                self.recompute_deal(&mut tx, d.id, now).await?;
            }
        }

        let orphans: Vec<SourceDealRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM source_deals WHERE deal_id IS NULL AND lifecycle <> 'CANCELLED' AND first_seen_at < $1 ORDER BY first_seen_at",
            store::SD_COLS
        )))
        .bind(now - self.cfg.orphan_grace)
        .fetch_all(&mut *tx)
        .await?;
        for o in orphans {
            let id = store::raise_break(
                &mut tx,
                BreakKind::Orphan,
                None,
                Some(o.source()),
                Some(&o.source_ref),
                None,
                o.desk_id,
                json!({ "first_seen_at": o.first_seen_at, "lifecycle": o.lifecycle }),
                now,
            )
            .await?;
            if id.is_some() {
                report.orphans_raised += 1;
            }
        }

        report.pending = sqlx::query_scalar("SELECT count(*) FROM source_events WHERE processed_at IS NULL").fetch_one(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO recon_status (name, now, last_tick_at, pending, detail) VALUES ('worker', $1, $1, $2, $3) \
             ON CONFLICT (name) DO UPDATE SET now = EXCLUDED.now, last_tick_at = EXCLUDED.last_tick_at, pending = EXCLUDED.pending, detail = EXCLUDED.detail",
        )
        .bind(now)
        .bind(report.pending)
        .bind(json!({ "stale_feeds": report.stale_feeds.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "gated": report.gated_by_stale_feed }))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(report)
    }

    /// Wipe the projection (links, breaks, external-origin deals, annotations) and replay the whole
    /// log in order. Internal bookings and their history are untouched.
    pub async fn rebuild(&self) -> anyhow::Result<usize> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM break_actions").execute(&mut *tx).await?;
        sqlx::query("DELETE FROM breaks").execute(&mut *tx).await?;
        sqlx::query("DELETE FROM source_deals").execute(&mut *tx).await?;
        sqlx::query("UPDATE deals SET merged_into = NULL WHERE merged_into IS NOT NULL").execute(&mut *tx).await?;
        sqlx::query("DELETE FROM deals WHERE origin <> 'INTERNAL'").execute(&mut *tx).await?;
        sqlx::query("UPDATE deals SET match_status = 'UNMATCHED', confirmed = false WHERE match_status <> 'UNMATCHED' OR confirmed")
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE source_events SET processed_at = NULL, outcome = NULL, outcome_detail = NULL").execute(&mut *tx).await?;
        tx.commit().await?;
        self.process_until_idle().await
    }

    /// Publish the clock and queue depth without running SLA checks (between ticks).
    pub async fn heartbeat(&self) -> anyhow::Result<()> {
        let now = self.now();
        let pending = self.pending().await?;
        sqlx::query(
            "INSERT INTO recon_status (name, now, last_tick_at, pending) VALUES ('worker', $1, $1, $2) \
             ON CONFLICT (name) DO UPDATE SET now = EXCLUDED.now, pending = EXCLUDED.pending",
        )
        .bind(now)
        .bind(pending)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    fn deal_as_norm(&self, d: &DealRow) -> NormalisedDeal {
        NormalisedDeal {
            product: d.product(),
            side: domain::Side::from_str(&d.side).expect("constrained"),
            ccy1: d.ccy1.trim().to_string(),
            ccy2: d.ccy2.as_deref().map(|c| c.trim().to_string()),
            notional: d.notional,
            rate: d.rate,
            trade_date: d.trade_date,
            value_date: d.value_date,
            maturity_date: d.maturity_date,
            counterparty_raw: d.counterparty.clone(),
            counterparty: self.normaliser.canonical_counterparty(&d.counterparty),
            trader_ref: None,
            lifecycle: d.status(),
            raw_state: None,
            desk_code: None,
            desk_id: Some(d.desk_id),
            cross_refs: Default::default(),
            self_deal_id: Some(d.id),
        }
    }
}

fn same_image(a: &NormalisedDeal, b: &NormalisedDeal) -> bool {
    a.product == b.product
        && a.side == b.side
        && a.ccy1 == b.ccy1
        && a.ccy2 == b.ccy2
        && a.notional == b.notional
        && a.rate == b.rate
        && a.trade_date == b.trade_date
        && a.value_date == b.value_date
        && a.maturity_date == b.maturity_date
        && a.counterparty == b.counterparty
        && a.lifecycle == b.lifecycle
}

fn fmt_opt(d: Option<chrono::NaiveDate>) -> String {
    d.map(|x| x.to_string()).unwrap_or_default()
}

/// Drives the engine in tests and in the synthetic feed: a manual clock plus direct ingestion.
pub struct Harness {
    pub engine: Engine,
    pub clock: crate::clock::ManualClock,
    pub pool: PgPool,
    pub unassigned: i16,
}

impl Harness {
    pub async fn new(pool: PgPool, start: DateTime<Utc>, cfg: EngineConfig) -> anyhow::Result<Self> {
        let clock = crate::clock::ManualClock::at(start);
        let engine = Engine::load(pool.clone(), Arc::new(clock.clone()), cfg).await?;
        let unassigned = engine.unassigned_desk();
        Ok(Self { engine, clock, pool, unassigned })
    }

    /// Append an observation stamped with the harness clock.
    pub async fn ingest(&self, source: Source, source_ref: &str, kind: EventKind, seq: Option<i64>, payload: Value) -> anyhow::Result<Option<i64>> {
        let now = self.clock.now();
        Ok(crate::ingest::insert_event(&self.pool, source, source_ref, kind, seq, now, now, &payload, self.unassigned).await?)
    }

    pub async fn run(&self) -> anyhow::Result<usize> {
        self.engine.process_until_idle().await
    }

    pub async fn tick(&self) -> anyhow::Result<SlaReport> {
        self.engine.sla_tick().await
    }
}
