//! End-to-end scenarios for the reconciliation engine against a fresh database per test.
//! DATABASE_URL = owner (sqlx::test), APP_DATABASE_URL = booking role, RECON_DATABASE_URL = reconciler.

use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use domain::{EventKind, Source};
use recon::pipeline::{EngineConfig, Harness};
use serde_json::{Value, json};
use server::{db, seed};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use uuid::Uuid;

const T0: &str = "2026-10-05T08:00:00Z";

fn t(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

async fn pool_for(owner: &PgPool, env: &str) -> PgPool {
    let url = std::env::var(env).unwrap_or_else(|_| panic!("{env} must be set"));
    let creds = PgConnectOptions::from_str(&url).expect("url parses");
    let test_db = owner.connect_options().get_database().expect("test db name").to_string();
    PgPoolOptions::new().max_connections(4).connect_with(creds.database(&test_db)).await.expect("role connects")
}

async fn setup(owner: &PgPool) -> (Harness, PgPool) {
    let app = pool_for(owner, "APP_DATABASE_URL").await;
    let recon = pool_for(owner, "RECON_DATABASE_URL").await;
    seed::run(owner, &app).await.unwrap();
    let h = Harness::new(recon, t(T0), EngineConfig::default()).await.unwrap();
    (h, app)
}

// --- payload builders (the shapes the synthetic adapters and, later, real adapters produce) ---

fn kondor(deal_id: &str, amount: i64, rate: &str, status: &str, refs: Value) -> Value {
    json!({"deal_id": deal_id, "portfolio": "FX-BOOK", "instrument": "FXSPOT", "direction": "BUY", "ccy1": "USD", "ccy2": "BHD",
           "amount": amount, "rate": rate, "trade_date": "2026-10-05", "value_date": "2026-10-07", "maturity_date": null,
           "counterparty": "Gulf International Bank B.S.C.", "trader": "FXT1", "status": status, "refs": refs})
}

fn bloomberg(ticket: &str, state: &str) -> Value {
    // Same economics as kondor(_, 1_000_000, "0.376", ..) quoted the other way round.
    json!({"ticket": ticket, "account": "NBB-FX", "product": "SPOT", "side": "S", "pair": "BHD/USD", "qty": 376000, "price": 2.65957,
           "trade_dt": "20261005", "settle_dt": "20261007", "cpty": "GIB", "state": state})
}

fn quantum(no: &str, front_ref: Option<&str>, amount: i64, rate: &str, status: &str) -> Value {
    json!({"trade_no": no, "book": "TRS-FX", "type": "FX_SPOT", "direction": "BUY", "ccy1": "USD", "ccy2": "BHD", "amount": amount,
           "rate": rate, "trade_date": "2026-10-05", "value_date": "2026-10-07", "maturity_date": null,
           "counterparty": "GULF INTERNATIONAL BANK", "status": status, "front_ref": front_ref})
}

#[derive(Debug, sqlx::FromRow)]
struct Golden {
    id: Uuid,
    origin: String,
    status: String,
    match_status: String,
    confirmed: bool,
    rate: rust_decimal::Decimal,
    notional: rust_decimal::Decimal,
    merged_into: Option<Uuid>,
}

async fn golden_of(owner: &PgPool, source: Source, source_ref: &str) -> Golden {
    sqlx::query_as(
        "SELECT d.id, d.origin, d.status, d.match_status, d.confirmed, d.rate, d.notional, d.merged_into \
         FROM source_deals s JOIN deals d ON d.id = s.deal_id WHERE s.source = $1 AND s.source_ref = $2",
    )
    .bind(source.as_str())
    .bind(source_ref)
    .fetch_one(owner)
    .await
    .unwrap_or_else(|e| panic!("{} {source_ref} has no golden deal: {e}", source.as_str()))
}

async fn outcome(owner: &PgPool, id: i64) -> String {
    sqlx::query_scalar("SELECT coalesce(outcome, '<unprocessed>') FROM source_events WHERE id = $1").bind(id).fetch_one(owner).await.unwrap()
}

/// (kind, status, source, source_ref, field, resolution)
async fn breaks(owner: &PgPool) -> Vec<(String, String, Option<String>, Option<String>, Option<String>, Option<String>)> {
    sqlx::query_as("SELECT kind, status, source, source_ref, field, resolution FROM breaks ORDER BY id").fetch_all(owner).await.unwrap()
}

async fn open_breaks(owner: &PgPool) -> Vec<(String, Option<String>, Option<String>)> {
    sqlx::query_as("SELECT kind, source, field FROM breaks WHERE status <> 'RESOLVED' ORDER BY id").fetch_all(owner).await.unwrap()
}

async fn book_internal(app: &PgPool, user: &str, amount: i64, rate: &str) -> Uuid {
    let mut tx = db::begin_as(app, seed::user_id(user)).await.unwrap();
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, trade_date, value_date, counterparty, trader_id, status) \
         SELECT gen_random_uuid(), 'FX-' || lpad(nextval('deal_ref_seq')::text, 6, '0'), id, 'FX_SPOT', 'BUY', 'USD', 'BHD', $1, $2::numeric, \
                '2026-10-05', '2026-10-07', 'Gulf International Bank', $3, 'BOOKED' \
         FROM desks WHERE code = 'FX' RETURNING id",
    )
    .bind(rust_decimal::Decimal::from(amount))
    .bind(rate)
    .bind(seed::user_id(user))
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    id
}

// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrations = "../../db/migrations")]
async fn three_way_match_through_cross_references(owner: PgPool) {
    let (h, _app) = setup(&owner).await;

    let b = h.ingest(Source::Bloomberg, "VCON1", EventKind::New, None, bloomberg("VCON1", "DONE")).await.unwrap().unwrap();
    h.run().await.unwrap();
    assert_eq!(outcome(&owner, b).await, "CREATED");
    let g = golden_of(&owner, Source::Bloomberg, "VCON1").await;
    assert_eq!((g.origin.as_str(), g.status.as_str(), g.match_status.as_str()), ("BLOOMBERG", "EXECUTED", "UNMATCHED"));

    h.clock.advance(Duration::minutes(2));
    let k = h.ingest(Source::Kondor, "K1", EventKind::New, Some(1), kondor("K1", 1_000_000, "0.37600", "NEW", json!({"BLOOMBERG": "VCON1"}))).await.unwrap().unwrap();
    h.run().await.unwrap();
    assert_eq!(outcome(&owner, k).await, "LINKED");
    let g = golden_of(&owner, Source::Kondor, "K1").await;
    assert_eq!(g.id, golden_of(&owner, Source::Bloomberg, "VCON1").await.id, "one golden deal for both observations");
    assert_eq!((g.status.as_str(), g.match_status.as_str(), g.confirmed), ("BOOKED", "PARTIAL", false));
    assert_eq!(g.rate.to_string(), "0.37600000", "Kondor owns the economics once linked");

    h.clock.advance(Duration::minutes(10));
    let q = h.ingest(Source::Quantum, "Q1", EventKind::New, Some(1), quantum("Q1", Some("K1"), 1_000_000, "0.376", "PENDING")).await.unwrap().unwrap();
    h.run().await.unwrap();
    assert_eq!(outcome(&owner, q).await, "LINKED");
    let g = golden_of(&owner, Source::Quantum, "Q1").await;
    assert_eq!((g.status.as_str(), g.match_status.as_str(), g.confirmed), ("BOOKED", "MATCHED", false));

    h.ingest(Source::Quantum, "Q1", EventKind::Status, Some(2), quantum("Q1", Some("K1"), 1_000_000, "0.376", "CONFIRMED")).await.unwrap();
    h.run().await.unwrap();
    let g = golden_of(&owner, Source::Quantum, "Q1").await;
    assert_eq!((g.status.as_str(), g.confirmed), ("CONFIRMED", true));

    h.ingest(Source::Quantum, "Q1", EventKind::Status, Some(3), quantum("Q1", Some("K1"), 1_000_000, "0.376", "SETTLED")).await.unwrap();
    h.run().await.unwrap();
    assert_eq!(golden_of(&owner, Source::Quantum, "Q1").await.status, "SETTLED");
    assert!(breaks(&owner).await.is_empty(), "a clean deal raises nothing");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn reversed_quote_within_tolerance_matches_by_economics(owner: PgPool) {
    let (h, _app) = setup(&owner).await;
    h.ingest(Source::Kondor, "K1", EventKind::New, Some(1), kondor("K1", 1_000_000, "0.37600", "NEW", json!({}))).await.unwrap();
    h.run().await.unwrap();
    let b = h.ingest(Source::Bloomberg, "VCON1", EventKind::New, None, bloomberg("VCON1", "DONE")).await.unwrap().unwrap();
    h.run().await.unwrap();
    assert_eq!(outcome(&owner, b).await, "LINKED", "no cross reference: matched on economics after re-expressing the quote");
    assert_eq!(golden_of(&owner, Source::Bloomberg, "VCON1").await.id, golden_of(&owner, Source::Kondor, "K1").await.id);
    assert!(open_breaks(&owner).await.is_empty(), "rounding inside tolerance is not a mismatch");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn mismatch_is_raised_and_auto_resolved_by_a_correcting_amend(owner: PgPool) {
    let (h, _app) = setup(&owner).await;
    h.ingest(Source::Kondor, "K1", EventKind::New, Some(1), kondor("K1", 1_000_000, "0.37600", "NEW", json!({}))).await.unwrap();
    h.ingest(Source::Quantum, "Q1", EventKind::New, Some(1), quantum("Q1", Some("K1"), 1_000_000, "0.37650", "PENDING")).await.unwrap();
    h.run().await.unwrap();
    let g = golden_of(&owner, Source::Quantum, "Q1").await;
    assert_eq!(g.match_status, "BREAK");
    assert_eq!(open_breaks(&owner).await, vec![("MISMATCH".into(), Some("QUANTUM".into()), Some("rate".into()))]);

    h.clock.advance(Duration::minutes(1));
    h.ingest(Source::Quantum, "Q1", EventKind::Amend, Some(2), quantum("Q1", Some("K1"), 1_000_000, "0.37600", "PENDING")).await.unwrap();
    h.run().await.unwrap();
    assert!(open_breaks(&owner).await.is_empty());
    let all = breaks(&owner).await;
    assert_eq!(all[0].5.as_deref(), Some("AUTO"));
    assert_eq!(golden_of(&owner, Source::Quantum, "Q1").await.match_status, "MATCHED");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn missing_after_sla_then_late_arrival_resolves_with_lateness(owner: PgPool) {
    let (h, _app) = setup(&owner).await;
    h.ingest(Source::Kondor, "K1", EventKind::New, Some(1), kondor("K1", 1_000_000, "0.37600", "NEW", json!({}))).await.unwrap();
    h.run().await.unwrap();

    h.clock.advance(Duration::minutes(10));
    let r = h.tick().await.unwrap();
    assert_eq!(r.missing_raised, 0, "Quantum is allowed 30 minutes plus grace");

    h.clock.advance(Duration::minutes(22));
    let r = h.tick().await.unwrap();
    assert_eq!(r.missing_raised, 1);
    assert_eq!(open_breaks(&owner).await, vec![("MISSING".into(), Some("QUANTUM".into()), None)]);
    assert_eq!(golden_of(&owner, Source::Kondor, "K1").await.match_status, "BREAK");
    assert_eq!(h.tick().await.unwrap().missing_raised, 0, "idempotent: the same break is not raised twice");

    h.clock.advance(Duration::minutes(3));
    h.ingest(Source::Quantum, "Q1", EventKind::New, Some(1), quantum("Q1", Some("K1"), 1_000_000, "0.376", "PENDING")).await.unwrap();
    h.run().await.unwrap();
    let late: Option<i64> = sqlx::query_scalar("SELECT (details->>'late_by_secs')::bigint FROM breaks WHERE kind = 'MISSING'").fetch_one(&owner).await.unwrap();
    assert!(late.is_some_and(|s| (200..=260).contains(&s)), "late by about four minutes: {late:?}");
    assert_eq!(breaks(&owner).await[0].5.as_deref(), Some("AUTO"));
    assert_eq!(golden_of(&owner, Source::Kondor, "K1").await.match_status, "MATCHED");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn orphan_after_grace_then_manual_link(owner: PgPool) {
    let (h, app) = setup(&owner).await;
    // A back-office record whose front-office reference is unknown and whose economics match nothing.
    let q = h.ingest(Source::Quantum, "Q9", EventKind::New, Some(1), quantum("Q9", Some("K-NOPE"), 5_000_000, "0.37700", "PENDING")).await.unwrap().unwrap();
    h.run().await.unwrap();
    assert_eq!(outcome(&owner, q).await, "WAITING");
    assert_eq!(h.tick().await.unwrap().orphans_raised, 0, "inside the grace window");
    h.clock.advance(Duration::minutes(50));
    assert_eq!(h.tick().await.unwrap().orphans_raised, 1);
    let break_id: i64 = sqlx::query_scalar("SELECT id FROM breaks WHERE kind = 'ORPHAN'").fetch_one(&owner).await.unwrap();

    // The desk head links it to the Kondor deal it belongs to (different economics: a fat finger upstream).
    h.ingest(Source::Kondor, "K9", EventKind::New, Some(1), kondor("K9", 5_000_000, "0.37600", "NEW", json!({}))).await.unwrap();
    h.run().await.unwrap();
    let target = golden_of(&owner, Source::Kondor, "K9").await.id;
    let mut tx = db::begin_as(&app, seed::user_id("head1")).await.unwrap();
    sqlx::query("SELECT break_resolve($1, 'LINK', $2, 'same ticket, rate keyed wrongly in Quantum')").bind(break_id).bind(target).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    h.run().await.unwrap();

    let g = golden_of(&owner, Source::Quantum, "Q9").await;
    assert_eq!(g.id, target, "manual link applied by the worker");
    let b = breaks(&owner).await;
    assert_eq!((b[0].0.as_str(), b[0].1.as_str(), b[0].5.as_deref()), ("ORPHAN", "RESOLVED", Some("LINKED")));
    assert!(open_breaks(&owner).await.iter().any(|(k, _, f)| k == "MISMATCH" && f.as_deref() == Some("rate")), "the rate difference now shows as a mismatch");
    let user_outcome: String = sqlx::query_scalar("SELECT outcome FROM source_events WHERE source = 'USER'").fetch_one(&owner).await.unwrap();
    assert_eq!(user_outcome, "APPLIED");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn ambiguous_when_two_identical_candidates(owner: PgPool) {
    let (h, app) = setup(&owner).await;
    // Two distinct front-office deals 0.8 pip apart; the confirmation sits in the middle, within
    // tolerance of both. (Identical economics would be a double booking, not an ambiguity.)
    h.ingest(Source::Kondor, "K1", EventKind::New, Some(1), kondor("K1", 1_000_000, "0.37596", "NEW", json!({}))).await.unwrap();
    h.ingest(Source::Kondor, "K2", EventKind::New, Some(1), kondor("K2", 1_000_000, "0.37604", "NEW", json!({}))).await.unwrap();
    let q = h.ingest(Source::Quantum, "Q1", EventKind::New, Some(1), quantum("Q1", None, 1_000_000, "0.3760", "PENDING")).await.unwrap().unwrap();
    h.run().await.unwrap();
    assert_eq!(outcome(&owner, q).await, "BREAK");
    let (kind, details): (String, Value) = sqlx::query_as("SELECT kind, details FROM breaks").fetch_one(&owner).await.unwrap();
    assert_eq!(kind, "AMBIGUOUS");
    assert_eq!(details["candidates"].as_array().map(Vec::len), Some(2));

    let break_id: i64 = sqlx::query_scalar("SELECT id FROM breaks").fetch_one(&owner).await.unwrap();
    let target = golden_of(&owner, Source::Kondor, "K2").await.id;
    let mut tx = db::begin_as(&app, seed::user_id("head1")).await.unwrap();
    sqlx::query("SELECT break_resolve($1, 'LINK', $2, NULL)").bind(break_id).bind(target).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    h.run().await.unwrap();
    assert_eq!(golden_of(&owner, Source::Quantum, "Q1").await.id, target);
    assert!(open_breaks(&owner).await.is_empty());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn duplicates_are_caught_at_every_layer(owner: PgPool) {
    let (h, _app) = setup(&owner).await;
    let p = kondor("K1", 1_000_000, "0.37600", "NEW", json!({}));
    assert!(h.ingest(Source::Kondor, "K1", EventKind::New, Some(1), p.clone()).await.unwrap().is_some());
    assert!(h.ingest(Source::Kondor, "K1", EventKind::New, Some(1), p.clone()).await.unwrap().is_none(), "identical event: dropped by the hash");

    h.clock.advance(Duration::seconds(30));
    let resend = h.ingest(Source::Kondor, "K1", EventKind::Amend, Some(2), p.clone()).await.unwrap().unwrap();
    h.run().await.unwrap();
    assert_eq!(outcome(&owner, resend).await, "DUP", "same image, newer stamp: recorded, no effect");

    let twice = h.ingest(Source::Kondor, "K1-BIS", EventKind::New, Some(1), kondor("K1-BIS", 1_000_000, "0.37600", "NEW", json!({}))).await.unwrap().unwrap();
    h.run().await.unwrap();
    assert_eq!(outcome(&owner, twice).await, "BREAK", "a second Kondor ticket for the same economics is a double booking");
    assert_eq!(open_breaks(&owner).await, vec![("DUPLICATE".into(), Some("KONDOR".into()), None)]);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM deals").fetch_one(&owner).await.unwrap();
    assert_eq!(n, 1, "no second golden deal");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn amend_before_new_keeps_the_newer_image(owner: PgPool) {
    let (h, _app) = setup(&owner).await;
    let amend = h.ingest(Source::Kondor, "K1", EventKind::Amend, Some(2), kondor("K1", 1_000_000, "0.37700", "NEW", json!({}))).await.unwrap().unwrap();
    h.clock.advance(Duration::seconds(5));
    let new = h.ingest(Source::Kondor, "K1", EventKind::New, Some(1), kondor("K1", 1_000_000, "0.37600", "NEW", json!({}))).await.unwrap().unwrap();
    h.run().await.unwrap();
    assert_eq!(outcome(&owner, amend).await, "CREATED");
    assert_eq!(outcome(&owner, new).await, "STALE");
    assert_eq!(golden_of(&owner, Source::Kondor, "K1").await.rate.to_string(), "0.37700000");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn cancellation_propagates_only_from_the_authoritative_source(owner: PgPool) {
    let (h, _app) = setup(&owner).await;
    h.ingest(Source::Kondor, "K1", EventKind::New, Some(1), kondor("K1", 1_000_000, "0.37600", "NEW", json!({}))).await.unwrap();
    h.ingest(Source::Quantum, "Q1", EventKind::New, Some(1), quantum("Q1", Some("K1"), 1_000_000, "0.376", "PENDING")).await.unwrap();
    h.run().await.unwrap();
    assert_eq!(golden_of(&owner, Source::Kondor, "K1").await.status, "BOOKED");

    h.clock.advance(Duration::minutes(1));
    h.ingest(Source::Kondor, "K1", EventKind::Cancel, Some(2), kondor("K1", 1_000_000, "0.37600", "CANCELLED", json!({}))).await.unwrap();
    h.run().await.unwrap();
    let g = golden_of(&owner, Source::Kondor, "K1").await;
    assert_eq!((g.status.as_str(), g.match_status.as_str()), ("CANCELLED", "BREAK"));
    assert_eq!(open_breaks(&owner).await, vec![("MISMATCH".into(), Some("QUANTUM".into()), Some("status".into()))], "Quantum still shows it live");

    h.ingest(Source::Quantum, "Q1", EventKind::Cancel, Some(2), quantum("Q1", Some("K1"), 1_000_000, "0.376", "CANCELLED")).await.unwrap();
    h.run().await.unwrap();
    assert!(open_breaks(&owner).await.is_empty());

    // Bloomberg has no cancel authority: its cancel on another deal is only a mismatch.
    h.ingest(Source::Kondor, "K2", EventKind::New, Some(1), kondor("K2", 2_000_000, "0.37600", "NEW", json!({"BLOOMBERG": "VCON2"}))).await.unwrap();
    h.ingest(Source::Bloomberg, "VCON2", EventKind::Cancel, None, json!({"ticket":"VCON2","account":"NBB-FX","product":"SPOT","side":"B","pair":"USD/BHD","qty":2000000,"price":0.376,"trade_dt":"20261005","settle_dt":"20261007","cpty":"GIB","state":"CXL"})).await.unwrap();
    h.run().await.unwrap();
    let g = golden_of(&owner, Source::Kondor, "K2").await;
    assert_eq!(g.status, "BOOKED");
    assert!(open_breaks(&owner).await.iter().any(|(k, s, f)| k == "MISMATCH" && s.as_deref() == Some("BLOOMBERG") && f.as_deref() == Some("status")));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn internal_booking_anchors_and_absorbs_a_provisional_deal(owner: PgPool) {
    let (h, app) = setup(&owner).await;
    // Execution seen first: a provisional golden deal from Bloomberg.
    h.ingest(Source::Bloomberg, "VCON1", EventKind::New, None, bloomberg("VCON1", "DONE")).await.unwrap();
    h.run().await.unwrap();
    let provisional = golden_of(&owner, Source::Bloomberg, "VCON1").await;
    assert_eq!(provisional.origin, "BLOOMBERG");

    // The trader books it internally: the booking becomes the anchor, the provisional is folded in.
    let booked = book_internal(&app, "fx1", 1_000_000, "0.376").await;
    h.run().await.unwrap();
    let g = golden_of(&owner, Source::Bloomberg, "VCON1").await;
    assert_eq!(g.id, booked, "Bloomberg's link moved to the internal booking");
    assert_eq!((g.origin.as_str(), g.status.as_str(), g.match_status.as_str()), ("INTERNAL", "BOOKED", "PARTIAL"));
    let (p_status, p_merged): (String, Option<Uuid>) = sqlx::query_as("SELECT status, merged_into FROM deals WHERE id = $1").bind(provisional.id).fetch_one(&owner).await.unwrap();
    assert_eq!((p_status.as_str(), p_merged), ("CANCELLED", Some(booked)));
    let internal_link: String = sqlx::query_scalar("SELECT link_kind FROM source_deals WHERE source = 'INTERNAL'").fetch_one(&owner).await.unwrap();
    assert_eq!(internal_link, "SELF");

    // Kondor follows with the Bloomberg reference and lands on the same deal.
    h.ingest(Source::Kondor, "K1", EventKind::New, Some(1), kondor("K1", 1_000_000, "0.37600", "NEW", json!({"BLOOMBERG": "VCON1"}))).await.unwrap();
    h.run().await.unwrap();
    assert_eq!(golden_of(&owner, Source::Kondor, "K1").await.id, booked);
    assert_eq!(golden_of(&owner, Source::Kondor, "K1").await.rate.to_string(), "0.37600000", "internal economics are never rewritten");
    assert!(open_breaks(&owner).await.is_empty());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn poisoned_event_does_not_block_its_batch(owner: PgPool) {
    let (h, _app) = setup(&owner).await;
    let bad = h.ingest(Source::Kondor, "BAD", EventKind::New, Some(1), json!({"deal_id": "BAD", "instrument": "WHATEVER"})).await.unwrap().unwrap();
    let good = h.ingest(Source::Kondor, "K1", EventKind::New, Some(1), kondor("K1", 1_000_000, "0.37600", "NEW", json!({}))).await.unwrap().unwrap();
    h.run().await.unwrap();
    assert_eq!(outcome(&owner, bad).await, "ERROR");
    assert_eq!(outcome(&owner, good).await, "CREATED");
    let detail: Value = sqlx::query_scalar("SELECT outcome_detail FROM source_events WHERE id = $1").bind(bad).fetch_one(&owner).await.unwrap();
    assert!(detail["error"].as_str().unwrap().contains("instrument"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn rebuild_replays_to_the_same_state(owner: PgPool) {
    let (h, app) = setup(&owner).await;
    h.ingest(Source::Bloomberg, "VCON1", EventKind::New, None, bloomberg("VCON1", "DONE")).await.unwrap();
    h.run().await.unwrap();
    book_internal(&app, "fx1", 1_000_000, "0.376").await;
    h.ingest(Source::Kondor, "K1", EventKind::New, Some(1), kondor("K1", 1_000_000, "0.37600", "NEW", json!({"BLOOMBERG": "VCON1"}))).await.unwrap();
    h.ingest(Source::Kondor, "K2", EventKind::New, Some(1), kondor("K2", 3_000_000, "0.37600", "NEW", json!({}))).await.unwrap();
    h.ingest(Source::Quantum, "Q2", EventKind::New, Some(1), quantum("Q2", Some("K2"), 3_000_000, "0.37650", "PENDING")).await.unwrap();
    h.ingest(Source::Quantum, "Q9", EventKind::New, Some(1), quantum("Q9", None, 9_000_000, "0.37700", "PENDING")).await.unwrap();
    h.run().await.unwrap();
    h.clock.advance(Duration::minutes(50));
    h.tick().await.unwrap();
    let orphan: i64 = sqlx::query_scalar("SELECT id FROM breaks WHERE kind = 'ORPHAN'").fetch_one(&owner).await.unwrap();
    let mut tx = db::begin_as(&app, seed::user_id("head1")).await.unwrap();
    sqlx::query("SELECT break_resolve($1, 'DISMISS', NULL, 'test deal, ignore')").bind(orphan).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    h.run().await.unwrap();

    let snapshot = |owner: PgPool| async move {
        let deals: Vec<(String, String, String, bool)> =
            sqlx::query_as("SELECT origin, status, match_status, confirmed FROM deals WHERE merged_into IS NULL ORDER BY deal_ref").fetch_all(&owner).await.unwrap();
        let links: Vec<(String, String, String)> =
            sqlx::query_as("SELECT source, source_ref, coalesce(link_kind, '-') FROM source_deals ORDER BY source, source_ref").fetch_all(&owner).await.unwrap();
        // Breaks derived from observations and SLA checks; user dismissals are compared separately.
        let brks: Vec<(String, Option<String>, Option<String>)> =
            sqlx::query_as("SELECT kind, source, field FROM breaks WHERE kind <> 'ORPHAN' AND status <> 'RESOLVED' ORDER BY kind, source, field").fetch_all(&owner).await.unwrap();
        (deals, links, brks)
    };
    let before = snapshot(owner.clone()).await;
    assert!(before.2.iter().any(|b| b.0 == "MISSING"), "the tick raised a MISSING break before the rebuild: {before:?}");

    let replayed = h.engine.rebuild().await.unwrap();
    assert!(replayed >= 7, "every event was replayed: {replayed}");
    h.tick().await.unwrap(); // tick-derived breaks are re-derived at the same clock
    let after = snapshot(owner.clone()).await;
    assert_eq!(before.0, after.0, "deals");
    assert_eq!(before.1, after.1, "links");
    assert_eq!(before.2, after.2, "breaks");

    // Known limitation: a user action on a break that was raised by an SLA tick cannot be replayed,
    // because the break is re-raised with a new id after the user event has already been processed.
    let user_outcome: String = sqlx::query_scalar("SELECT outcome FROM source_events WHERE source = 'USER'").fetch_one(&owner).await.unwrap();
    assert_eq!(user_outcome, "STALE");
    let orphan_open: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM breaks WHERE kind = 'ORPHAN' AND status <> 'RESOLVED')").fetch_one(&owner).await.unwrap();
    assert!(orphan_open, "the dismissed orphan is open again and must be re-dismissed");
}
