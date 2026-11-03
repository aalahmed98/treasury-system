//! Reconciliation endpoints over HTTP: desk scoping, provenance, break actions, feeds.

use std::net::SocketAddr;
use std::str::FromStr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, Response, StatusCode, header};
use domain::{BreakDetail, BreakView, FeedsResponse, LoginResponse, ProvenanceResponse};
use http_body_util::BodyExt;
use server::config::Config;
use server::{AppState, db, seed};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use uuid::Uuid;

const PEER: &str = "127.0.0.1:40000";

async fn pool_for(owner: &PgPool, env: &str) -> PgPool {
    let url = std::env::var(env).unwrap_or_else(|_| panic!("{env} must be set"));
    let creds = PgConnectOptions::from_str(&url).expect("url parses");
    let test_db = owner.connect_options().get_database().expect("test db name").to_string();
    PgPoolOptions::new().max_connections(4).connect_with(creds.database(&test_db)).await.expect("role connects")
}

fn req(method: Method, uri: &str, token: Option<&str>, body: Option<String>) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .extension(ConnectInfo(PEER.parse::<SocketAddr>().unwrap()))
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    b.body(body.map(Body::from).unwrap_or_else(Body::empty)).unwrap()
}

async fn json<T: serde::de::DeserializeOwned>(res: Response<Body>) -> T {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("json body: {e}: {}", String::from_utf8_lossy(&bytes)))
}

async fn token(router: &axum::Router, user: &str) -> String {
    let body = serde_json::json!({ "username": user, "password": seed::DEV_PASSWORD }).to_string();
    let res = router.clone().oneshot(req(Method::POST, "/auth/login", None, Some(body))).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    json::<LoginResponse>(res).await.token
}

use tower::ServiceExt;

/// Users, one internal FX deal by fx1, a Kondor-origin FX deal with a linked Quantum row and a
/// mismatch break, an orphan on MM, and an orphan on UNASSIGNED. All written by the recon role.
async fn fixture(owner: &PgPool) -> (axum::Router, PgPool, Uuid, Uuid, i64, i64) {
    let app = pool_for(owner, "APP_DATABASE_URL").await;
    let recon = pool_for(owner, "RECON_DATABASE_URL").await;
    seed::run(owner, &app).await.unwrap();

    let mut tx = db::begin_as(&app, seed::user_id("fx1")).await.unwrap();
    let internal: Uuid = sqlx::query_scalar(
        "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, trade_date, value_date, counterparty, trader_id, status) \
         SELECT gen_random_uuid(), 'FX-000001', id, 'FX_SPOT', 'BUY', 'USD', 'BHD', 1000000, 0.376, current_date, current_date + 2, 'Gulf International Bank', $1, 'BOOKED' \
         FROM desks WHERE code = 'FX' RETURNING id",
    )
    .bind(seed::user_id("fx1"))
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let external: Uuid = sqlx::query_scalar(
        "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, trade_date, value_date, counterparty, trader_id, status, origin, match_status) \
         SELECT gen_random_uuid(), 'FX-000002', id, 'FX_SPOT', 'SELL', 'EUR', 'USD', 2000000, 1.085, current_date, current_date + 2, 'HSBC BAHRAIN', NULL, 'BOOKED', 'KONDOR', 'BREAK' \
         FROM desks WHERE code = 'FX' RETURNING id",
    )
    .fetch_one(&recon)
    .await
    .unwrap();
    for (source, r, rate, link) in [("KONDOR", "K1", "1.08500", "AUTO"), ("QUANTUM", "Q1", "1.08560", "CROSS_REF")] {
        sqlx::query(
            "INSERT INTO source_deals (source, source_ref, deal_id, link_kind, linked_at, lifecycle, applied_event_id, applied_ts, first_seen_at, desk_id, \
                                       product, side, ccy1, ccy2, notional, rate, trade_date, value_date, counterparty_raw, counterparty, updated_at) \
             SELECT $1, $2, $3, $4, now(), 'BOOKED', 0, now(), now(), id, 'FX_SPOT', 'SELL', 'EUR', 'USD', 2000000, $5::numeric, current_date, current_date + 2, \
                    'HSBC Bank Middle East', 'HSBC BAHRAIN', now() FROM desks WHERE code = 'FX'",
        )
        .bind(source)
        .bind(r)
        .bind(external)
        .bind(link)
        .bind(rate)
        .execute(&recon)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO source_events (source, source_ref, event_kind, source_ts, observed_at, payload, desk_id, dedup_hash) \
             SELECT $1, $2, 'NEW', now(), now(), '{}', id, sha256(convert_to($1 || $2, 'UTF8')) FROM desks WHERE code = 'FX'",
        )
        .bind(source)
        .bind(r)
        .execute(&recon)
        .await
        .unwrap();
    }
    let mismatch: i64 = sqlx::query_scalar(
        "INSERT INTO breaks (kind, deal_id, source, source_ref, field, desk_id, details, detected_at, updated_at) \
         SELECT 'MISMATCH', $1, 'QUANTUM', 'Q1', 'rate', id, '{\"expected\":\"1.085\",\"observed\":\"1.0856\"}', now(), now() FROM desks WHERE code = 'FX' RETURNING id",
    )
    .bind(external)
    .fetch_one(&recon)
    .await
    .unwrap();
    let mm_orphan: i64 = sqlx::query_scalar(
        "INSERT INTO breaks (kind, source, source_ref, desk_id, details, detected_at, updated_at) \
         SELECT 'ORPHAN', 'QUANTUM', 'Q7', id, '{}', now(), now() FROM desks WHERE code = 'MM' RETURNING id",
    )
    .fetch_one(&recon)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO breaks (kind, source, source_ref, desk_id, details, detected_at, updated_at) \
         SELECT 'ORPHAN', 'QUANTUM', 'Q8', id, '{}', now(), now() FROM desks WHERE code = 'UNASSIGNED'",
    )
    .execute(&recon)
    .await
    .unwrap();
    sqlx::query("UPDATE feed_status SET last_heartbeat_at = now(), last_event_at = now(), events_total = 42 WHERE source = 'KONDOR'").execute(&recon).await.unwrap();
    sqlx::query("INSERT INTO recon_status (name, now, last_tick_at, pending) VALUES ('worker', now(), now(), 3)").execute(&recon).await.unwrap();

    let router = server::app(AppState::new(app.clone(), Config::default()));
    (router, app, internal, external, mismatch, mm_orphan)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn provenance_and_breaks_are_desk_scoped(owner: PgPool) {
    let (router, _app, _internal, external, _mismatch, _mm_orphan) = fixture(&owner).await;
    let fx1 = token(&router, "fx1").await;
    let mm1 = token(&router, "mm1").await;
    let alm1 = token(&router, "alm1").await;
    let head1 = token(&router, "head1").await;

    let res = router.clone().oneshot(req(Method::GET, &format!("/deals/{external}/provenance"), Some(&fx1), None)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let p: ProvenanceResponse = json(res).await;
    assert_eq!(p.deal.deal_ref, "FX-000002");
    assert_eq!(p.deal.trader_name, "", "no trader for a reconciler-created deal");
    assert_eq!(p.sources.len(), 2);
    assert_eq!(p.breaks.len(), 1);
    assert_eq!(p.events.len(), 2);
    assert_eq!(p.sources[1].counterparty_raw, "HSBC Bank Middle East");

    let res = router.clone().oneshot(req(Method::GET, &format!("/deals/{external}/provenance"), Some(&mm1), None)).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "other desks cannot even see that it exists");

    let count = |tok: String, uri: &'static str| {
        let router = router.clone();
        async move {
            let res = router.oneshot(req(Method::GET, uri, Some(&tok), None)).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK);
            json::<Vec<BreakView>>(res).await.len()
        }
    };
    assert_eq!(count(fx1.clone(), "/breaks").await, 1);
    assert_eq!(count(mm1.clone(), "/breaks").await, 1);
    assert_eq!(count(alm1.clone(), "/breaks").await, 2, "ALM reads FX and MM but not UNASSIGNED");
    assert_eq!(count(head1.clone(), "/breaks").await, 3);
    assert_eq!(count(head1.clone(), "/breaks?desk_id=2").await, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn acknowledging_and_resolving_follow_entitlements(owner: PgPool) {
    let (router, _app, _internal, _external, mismatch, mm_orphan) = fixture(&owner).await;
    let fx1 = token(&router, "fx1").await;
    let mm1 = token(&router, "mm1").await;
    let alm1 = token(&router, "alm1").await;
    let head1 = token(&router, "head1").await;

    let ack = |tok: String, id: i64| {
        let router = router.clone();
        async move { router.oneshot(req(Method::POST, &format!("/breaks/{id}/ack"), Some(&tok), None)).await.unwrap().status() }
    };
    assert_eq!(ack(mm1.clone(), mismatch).await, StatusCode::NOT_FOUND, "not their desk");
    assert_eq!(ack(alm1.clone(), mismatch).await, StatusCode::FORBIDDEN, "read-only entitlement");
    assert_eq!(ack(fx1.clone(), mismatch).await, StatusCode::ACCEPTED);
    assert_eq!(ack(fx1.clone(), mismatch).await, StatusCode::BAD_REQUEST, "already acknowledged");

    let body = serde_json::json!({ "action": "DISMISS" }).to_string();
    let res = router.clone().oneshot(req(Method::POST, &format!("/breaks/{mismatch}/resolve"), Some(&fx1), Some(body))).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST, "dismiss needs a note");

    let body = serde_json::json!({ "action": "DISMISS", "note": "accepted difference" }).to_string();
    let res = router.clone().oneshot(req(Method::POST, &format!("/breaks/{mismatch}/resolve"), Some(&fx1), Some(body))).await.unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let view: BreakView = json(res).await;
    assert_eq!(view.status, domain::BreakStatus::Acknowledged, "the reconciler applies the resolution; the API only accepts it");

    let res = router.clone().oneshot(req(Method::GET, &format!("/breaks/{mismatch}"), Some(&fx1), None)).await.unwrap();
    let detail: BreakDetail = json(res).await;
    assert_eq!(detail.actions.iter().map(|a| a.action.as_str()).collect::<Vec<_>>(), vec!["ACK", "DISMISS"]);
    assert_eq!(detail.actions[0].user.as_deref(), Some("FX Trader One"));

    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM source_events WHERE source = 'USER' AND processed_at IS NULL").fetch_one(&owner).await.unwrap();
    assert_eq!(pending, 1, "the user action is queued for the worker");

    // The head of treasury may act on the money-market desk too.
    assert_eq!(ack(head1.clone(), mm_orphan).await, StatusCode::ACCEPTED);
    let audits: Vec<String> = sqlx::query_scalar("SELECT action FROM audit_log WHERE action LIKE 'break_%' ORDER BY id").fetch_all(&owner).await.unwrap();
    assert_eq!(audits, vec!["break_ack", "break_resolve", "break_ack"]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn feeds_report_freshness(owner: PgPool) {
    let (router, _app, _internal, _external, _mismatch, _mm_orphan) = fixture(&owner).await;
    let fx1 = token(&router, "fx1").await;
    let res = router.clone().oneshot(req(Method::GET, "/feeds", Some(&fx1), None)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let f: FeedsResponse = json(res).await;
    assert_eq!(f.feeds.len(), 4, "KONDOR, BLOOMBERG, QUANTUM, INTERNAL");
    let kondor = f.feeds.iter().find(|x| x.source == domain::Source::Kondor).unwrap();
    assert_eq!(kondor.events_total, 42);
    assert!(kondor.last_heartbeat_at.is_some());
    assert_eq!(f.recon.as_ref().map(|r| r.pending), Some(3));
}
