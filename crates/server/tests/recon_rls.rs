//! Privilege boundaries of the reconciliation schema: what the reconciler role may do, what the
//! API role may not, and how desk entitlement scopes reconciliation rows.

use std::str::FromStr;

use server::{db, seed};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use uuid::Uuid;

async fn pool_for(owner: &PgPool, env: &str) -> PgPool {
    let url = std::env::var(env).unwrap_or_else(|_| panic!("{env} must be set"));
    let creds = PgConnectOptions::from_str(&url).expect("url parses");
    let test_db = owner.connect_options().get_database().expect("test db name").to_string();
    PgPoolOptions::new().max_connections(4).connect_with(creds.database(&test_db)).await.expect("role connects")
}

async fn app_pool(owner: &PgPool) -> PgPool {
    pool_for(owner, "APP_DATABASE_URL").await
}

async fn recon_pool(owner: &PgPool) -> PgPool {
    pool_for(owner, "RECON_DATABASE_URL").await
}

fn sqlstate(e: &sqlx::Error) -> Option<String> {
    match e {
        sqlx::Error::Database(d) => d.code().map(|c| c.to_string()),
        _ => None,
    }
}

/// A golden deal created by the reconciler from an external source (no trader).
async fn recon_deal(recon: &PgPool, desk: &str, origin: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, \
                            trade_date, value_date, counterparty, trader_id, status, origin) \
         SELECT gen_random_uuid(), $1 || '-' || lpad(nextval('deal_ref_seq')::text, 6, '0'), id, \
                'FX_SPOT', 'BUY', 'USD', 'BHD', 1000000, 0.376, current_date, current_date + 2, \
                'Test Bank', NULL, 'BOOKED', $2 \
         FROM desks WHERE code = $1 RETURNING id",
    )
    .bind(desk)
    .bind(origin)
    .fetch_one(recon)
    .await
    .unwrap()
}

/// An internal booking by a trader through the normal path.
async fn internal_deal(app: &PgPool, user: &str, desk: &str) -> Uuid {
    let mut tx = db::begin_as(app, seed::user_id(user)).await.unwrap();
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, \
                            trade_date, value_date, counterparty, trader_id, status) \
         SELECT gen_random_uuid(), $1 || '-' || lpad(nextval('deal_ref_seq')::text, 6, '0'), id, \
                'FX_SPOT', 'BUY', 'USD', 'BHD', 1000000, 0.376, current_date, current_date + 2, \
                'Test Bank', $2, 'BOOKED' \
         FROM desks WHERE code = $1 RETURNING id",
    )
    .bind(desk)
    .bind(seed::user_id(user))
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    id
}

async fn insert_break(recon: &PgPool, desk: &str, kind: &str, source_ref: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO breaks (kind, source, source_ref, desk_id, details, detected_at, updated_at) \
         SELECT $1, 'KONDOR', $2, id, '{}', now(), now() FROM desks WHERE code = $3 RETURNING id",
    )
    .bind(kind)
    .bind(source_ref)
    .bind(desk)
    .fetch_one(recon)
    .await
    .unwrap()
}

// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrations = "../../db/migrations")]
async fn recon_role_creates_external_deals_only(owner: PgPool) {
    let app = app_pool(&owner).await;
    seed::run(&owner, &app).await.unwrap();
    let recon = recon_pool(&owner).await;

    let id = recon_deal(&recon, "FX", "KONDOR").await;
    let (origin, trader): (String, Option<Uuid>) =
        sqlx::query_as("SELECT origin, trader_id FROM deals WHERE id = $1").bind(id).fetch_one(&owner).await.unwrap();
    assert_eq!(origin, "KONDOR");
    assert!(trader.is_none());

    let err = sqlx::query(
        "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, trade_date, value_date, counterparty, trader_id, status, origin) \
         SELECT gen_random_uuid(), 'X-1', id, 'FX_SPOT', 'BUY', 'USD', 'BHD', 1, 0.376, current_date, current_date, 'x', $1, 'BOOKED', 'INTERNAL' FROM desks WHERE code = 'FX'",
    )
    .bind(seed::user_id("fx1"))
    .execute(&recon)
    .await
    .expect_err("the reconciler may not fabricate internal bookings");
    assert_eq!(sqlstate(&err).as_deref(), Some("42501"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn recon_role_annotates_but_never_rewrites_internal_economics(owner: PgPool) {
    let app = app_pool(&owner).await;
    seed::run(&owner, &app).await.unwrap();
    let recon = recon_pool(&owner).await;
    let internal = internal_deal(&app, "fx1", "FX").await;

    // Annotation is fine, on live and on settled deals.
    sqlx::query("UPDATE deals SET match_status = 'MATCHED', confirmed = true WHERE id = $1").bind(internal).execute(&recon).await.unwrap();
    sqlx::query("UPDATE deals SET status = 'SETTLED' WHERE id = $1").bind(internal).execute(&recon).await.unwrap();
    sqlx::query("UPDATE deals SET match_status = 'BREAK' WHERE id = $1").bind(internal).execute(&recon).await.unwrap();
    let (ms, v): (String, i32) = sqlx::query_as("SELECT match_status, version FROM deals WHERE id = $1").bind(internal).fetch_one(&owner).await.unwrap();
    assert_eq!((ms.as_str(), v), ("BREAK", 4));

    // Economics of an internal booking are off limits to the reconciler.
    let err = sqlx::query("UPDATE deals SET notional = 2 WHERE id = $1").bind(internal).execute(&recon).await.expect_err("economics locked");
    assert_eq!(sqlstate(&err).as_deref(), Some("P0001"), "{err}");

    // ... but it owns the economics of external-origin deals.
    let external = recon_deal(&recon, "FX", "BLOOMBERG").await;
    sqlx::query("UPDATE deals SET rate = 0.3761, counterparty = 'Other Bank' WHERE id = $1").bind(external).execute(&recon).await.unwrap();
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn recon_role_cannot_reach_identity_tables(owner: PgPool) {
    let app = app_pool(&owner).await;
    seed::run(&owner, &app).await.unwrap();
    let recon = recon_pool(&owner).await;
    for sql in [
        "SELECT * FROM users",
        "SELECT * FROM user_credentials",
        "SELECT * FROM user_desk_access",
        "SELECT * FROM sessions",
        "SELECT * FROM audit_log",
        "SELECT * FROM deal_history",
        "INSERT INTO user_desk_access (user_id, desk_id) SELECT id, 1 FROM users",
        "UPDATE users SET is_active = false",
    ] {
        let err = sqlx::query(sql).execute(&recon).await.expect_err(sql);
        assert_eq!(sqlstate(&err).as_deref(), Some("42501"), "{sql}: {err}");
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn app_role_cannot_write_reconciliation_tables(owner: PgPool) {
    let app = app_pool(&owner).await;
    seed::run(&owner, &app).await.unwrap();
    let internal = internal_deal(&app, "fx1", "FX").await;

    for sql in [
        "INSERT INTO source_events (source, source_ref, event_kind, source_ts, observed_at, payload, desk_id, dedup_hash) VALUES ('KONDOR','x','NEW',now(),now(),'{}',1,'\\x00')",
        "INSERT INTO source_deals (source, source_ref, lifecycle, applied_event_id, applied_ts, first_seen_at, desk_id, product, side, ccy1, notional, rate, trade_date, value_date, counterparty_raw, counterparty, updated_at) VALUES ('KONDOR','x','BOOKED',1,now(),now(),1,'FX_SPOT','BUY','USD',1,1,current_date,current_date,'x','x',now())",
        "INSERT INTO breaks (kind, desk_id, detected_at, updated_at) VALUES ('ORPHAN', 1, now(), now())",
        "UPDATE breaks SET status = 'RESOLVED'",
        "DELETE FROM breaks",
        "UPDATE feed_status SET events_total = 0",
        "INSERT INTO break_actions (break_id, desk_id, action) VALUES (1, 1, 'x')",
    ] {
        let err = sqlx::query(sql).execute(&app).await.expect_err(sql);
        assert_eq!(sqlstate(&err).as_deref(), Some("42501"), "{sql}: {err}");
    }

    // Column privileges: the API role may amend economics but not reconciliation annotations.
    let mut tx = db::begin_as(&app, seed::user_id("fx1")).await.unwrap();
    let err = sqlx::query("UPDATE deals SET match_status = 'MATCHED' WHERE id = $1").bind(internal).execute(&mut *tx).await.expect_err("annotations are the reconciler's");
    assert_eq!(sqlstate(&err).as_deref(), Some("42501"));
    drop(tx);
    let mut tx = db::begin_as(&app, seed::user_id("fx1")).await.unwrap();
    let err = sqlx::query("UPDATE deals SET confirmed = true WHERE id = $1").bind(internal).execute(&mut *tx).await.expect_err("confirmed is derived");
    assert_eq!(sqlstate(&err).as_deref(), Some("42501"));
    drop(tx);
    let mut tx = db::begin_as(&app, seed::user_id("fx1")).await.unwrap();
    sqlx::query("UPDATE deals SET counterparty = 'Amended' WHERE id = $1").bind(internal).execute(&mut *tx).await.expect("economics may be amended by the booker");
    tx.commit().await.unwrap();

    // Nor may it create external-origin deals.
    let mut tx = db::begin_as(&app, seed::user_id("fx1")).await.unwrap();
    let err = sqlx::query(
        "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, trade_date, value_date, counterparty, trader_id, status, origin) \
         SELECT gen_random_uuid(), 'X-2', id, 'FX_SPOT', 'BUY', 'USD', 'BHD', 1, 0.376, current_date, current_date, 'x', $1, 'BOOKED', 'KONDOR' FROM desks WHERE code = 'FX'",
    )
    .bind(seed::user_id("fx1"))
    .execute(&mut *tx)
    .await
    .expect_err("origin is INTERNAL for bookings");
    assert_eq!(sqlstate(&err).as_deref(), Some("42501"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn internal_bookings_enter_the_event_log(owner: PgPool) {
    let app = app_pool(&owner).await;
    seed::run(&owner, &app).await.unwrap();
    let id = internal_deal(&app, "fx1", "FX").await;

    let events: Vec<(String, String, String, i64)> = sqlx::query_as(
        "SELECT source, event_kind, source_ref, coalesce(source_seq, 0) FROM source_events ORDER BY id",
    )
    .fetch_all(&owner)
    .await
    .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, "INTERNAL");
    assert_eq!(events[0].1, "NEW");
    assert_eq!(events[0].2, id.to_string());
    assert_eq!(events[0].3, 1, "source_seq is the deal version");

    // An amendment is an AMEND event; a cancel is a CANCEL event.
    let mut tx = db::begin_as(&app, seed::user_id("fx1")).await.unwrap();
    sqlx::query("UPDATE deals SET counterparty = 'Changed' WHERE id = $1").bind(id).execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE deals SET status = 'CANCELLED' WHERE id = $1").bind(id).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let kinds: Vec<String> = sqlx::query_scalar("SELECT event_kind FROM source_events ORDER BY id").fetch_all(&owner).await.unwrap();
    assert_eq!(kinds, vec!["NEW", "AMEND", "CANCEL"]);

    let hist: Vec<String> = sqlx::query_scalar("SELECT changed_by_role FROM deal_history WHERE deal_id = $1 ORDER BY version").bind(id).fetch_all(&owner).await.unwrap();
    assert!(hist.iter().all(|r| r == "treasury_app"), "history records the acting role: {hist:?}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn reconciliation_rows_are_desk_scoped(owner: PgPool) {
    let app = app_pool(&owner).await;
    seed::run(&owner, &app).await.unwrap();
    let recon = recon_pool(&owner).await;

    insert_break(&recon, "FX", "ORPHAN", "K-1").await;
    insert_break(&recon, "MM", "ORPHAN", "K-2").await;
    insert_break(&recon, "UNASSIGNED", "ORPHAN", "K-3").await;
    for (desk, r) in [("FX", "K-1"), ("MM", "K-2"), ("UNASSIGNED", "K-3")] {
        sqlx::query(
            "INSERT INTO source_events (source, source_ref, event_kind, source_ts, observed_at, payload, desk_id, dedup_hash) \
             SELECT 'KONDOR', $1, 'NEW', now(), now(), '{}', id, sha256(convert_to($1, 'UTF8')) FROM desks WHERE code = $2",
        )
        .bind(r)
        .bind(desk)
        .execute(&recon)
        .await
        .unwrap();
    }

    let count = |user: &'static str, table: &'static str| {
        let app = app.clone();
        async move {
            let mut tx = db::begin_as(&app, seed::user_id(user)).await.unwrap();
            let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}"))).fetch_one(&mut *tx).await.unwrap();
            tx.commit().await.unwrap();
            n
        }
    };
    assert_eq!(count("fx1", "breaks").await, 1);
    assert_eq!(count("fx1", "source_events").await, 1);
    assert_eq!(count("mm1", "breaks").await, 1);
    assert_eq!(count("alm1", "breaks").await, 2, "ALM reads FX and MM but not UNASSIGNED");
    assert_eq!(count("head1", "breaks").await, 3, "the treasury head sees unmapped rows too");
    assert_eq!(count("head1", "source_events").await, 3);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM breaks").fetch_one(&app).await.unwrap();
    assert_eq!(n, 0, "no identity, no rows");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn break_actions_require_can_resolve_on_the_desk(owner: PgPool) {
    let app = app_pool(&owner).await;
    seed::run(&owner, &app).await.unwrap();
    let recon = recon_pool(&owner).await;
    let fx_break = insert_break(&recon, "FX", "ORPHAN", "K-9").await;
    let target = internal_deal(&app, "fx1", "FX").await;

    let call = |user: &'static str, sql: &'static str, break_id: i64| {
        let app = app.clone();
        async move {
            let mut tx = db::begin_as(&app, seed::user_id(user)).await.unwrap();
            let r = sqlx::query(sql).bind(break_id).bind(target).execute(&mut *tx).await;
            if r.is_ok() {
                tx.commit().await.unwrap();
            }
            r.map(|_| ()).map_err(|e| sqlstate(&e).unwrap_or_default())
        }
    };

    // Wrong desk: the break does not exist as far as mm1 is concerned.
    assert_eq!(call("mm1", "SELECT break_ack($1)", fx_break).await, Err("P0002".into()));
    // Read-only entitlement: visible, but no right to act.
    assert_eq!(call("alm1", "SELECT break_ack($1)", fx_break).await, Err("42501".into()));
    // Booker on the desk: may acknowledge, then resolve by linking.
    assert_eq!(call("fx1", "SELECT break_ack($1)", fx_break).await, Ok(()));
    assert_eq!(call("fx1", "SELECT break_ack($1)", fx_break).await, Err("P0001".into()), "cannot acknowledge twice");
    assert_eq!(call("fx1", "SELECT break_resolve($1, 'DISMISS', $2, NULL)", fx_break).await, Err("P0001".into()), "dismiss needs a note");
    assert_eq!(call("fx1", "SELECT break_resolve($1, 'LINK', $2, NULL)", fx_break).await, Ok(()));

    // The action is in the audit trail and in the event log for the reconciler to apply.
    let actions: Vec<String> = sqlx::query_scalar("SELECT action FROM break_actions WHERE break_id = $1 ORDER BY id").bind(fx_break).fetch_all(&owner).await.unwrap();
    assert_eq!(actions, vec!["ACK", "LINK"]);
    let (src, kind, payload): (String, String, serde_json::Value) =
        sqlx::query_as("SELECT source, event_kind, payload FROM source_events WHERE source = 'USER'").fetch_one(&owner).await.unwrap();
    assert_eq!((src.as_str(), kind.as_str()), ("USER", "USER"));
    assert_eq!(payload["action"], "LINK");
    assert_eq!(payload["deal_id"], target.to_string());

    // The head of treasury may act on any desk.
    let mm_break = insert_break(&recon, "MM", "ORPHAN", "K-10").await;
    assert_eq!(call("head1", "SELECT break_ack($1)", mm_break).await, Ok(()));
}
