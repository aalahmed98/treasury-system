//! Ladder maths and scoping, nostro actuals, amendments and history over HTTP.

use std::net::SocketAddr;
use std::str::FromStr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, Response, StatusCode, header};
use chrono::{Duration, Utc};
use domain::{Deal, HistoryEntry, LadderResponse, LoginResponse};
use http_body_util::BodyExt;
use rust_decimal::Decimal;
use server::config::Config;
use server::{AppState, db, seed};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tower::ServiceExt;
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

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

/// fx1 buys USD 1,000,000 against BHD at 0.376 for value T+2; mm1 lends BHD 2,000,000 at 5% for 30 days from T+2.
async fn fixture(owner: &PgPool) -> (axum::Router, PgPool, PgPool, Uuid, Uuid) {
    let app = pool_for(owner, "APP_DATABASE_URL").await;
    let recon = pool_for(owner, "RECON_DATABASE_URL").await;
    seed::run(owner, &app).await.unwrap();
    let today = Utc::now().date_naive();
    let v = today + Duration::days(2);
    let m = v + Duration::days(30);

    let mut tx = db::begin_as(&app, seed::user_id("fx1")).await.unwrap();
    let fx: Uuid = sqlx::query_scalar(
        "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, trade_date, value_date, counterparty, trader_id, status) \
         SELECT gen_random_uuid(), 'FX-000001', id, 'FX_SPOT', 'BUY', 'USD', 'BHD', 1000000, 0.376, $1, $2, 'Gulf International Bank', $3, 'BOOKED' \
         FROM desks WHERE code = 'FX' RETURNING id",
    )
    .bind(today)
    .bind(v)
    .bind(seed::user_id("fx1"))
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let mut tx = db::begin_as(&app, seed::user_id("mm1")).await.unwrap();
    let mm: Uuid = sqlx::query_scalar(
        "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, trade_date, value_date, maturity_date, counterparty, trader_id, status) \
         SELECT gen_random_uuid(), 'MM-000001', id, 'MM_DEPO', 'LEND', 'BHD', NULL, 2000000, 5.0, $1, $2, $3, 'BBK', $4, 'BOOKED' \
         FROM desks WHERE code = 'MM' RETURNING id",
    )
    .bind(today)
    .bind(v)
    .bind(m)
    .bind(seed::user_id("mm1"))
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let router = server::app(AppState::new(app.clone(), Config::default()));
    (router, app, recon, fx, mm)
}

async fn ladder(router: &axum::Router, tok: &str, ccy: &str) -> LadderResponse {
    let res = router.clone().oneshot(req(Method::GET, &format!("/ladder?ccy={ccy}&days=40"), Some(tok), None)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    json(res).await
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn ladder_projects_cash_per_currency_and_respects_desks(owner: PgPool) {
    let (router, _app, _recon, _fx, _mm) = fixture(&owner).await;
    let fx1 = token(&router, "fx1").await;
    let head1 = token(&router, "head1").await;
    let today = Utc::now().date_naive();
    let v = today + Duration::days(2);
    let m = v + Duration::days(30);

    // The FX trader sees only the FX legs.
    let usd = ladder(&router, &fx1, "USD").await;
    assert!(!usd.liquidity_access, "a desk trader has no nostro access");
    assert!(usd.opening.is_none());
    let row = usd.rows.iter().find(|r| r.date == v).unwrap();
    assert_eq!(row.provisional_in, dec("1000000.00"), "unconfirmed, so provisional");
    assert_eq!(row.confirmed_in, Decimal::ZERO);
    assert_eq!(row.net_expected, dec("1000000.00"));
    assert_eq!(row.deals, 1);
    let bhd = ladder(&router, &fx1, "BHD").await;
    assert_eq!(bhd.rows.iter().find(|r| r.date == v).unwrap().net_expected, dec("-376000.00"), "the BHD leg of the FX deal only");
    assert!(bhd.rows.iter().find(|r| r.date == m).unwrap().net_expected.is_zero(), "the MM maturity is not fx1's to see");

    // The treasury head sees both desks: BHD out for the FX leg and the deposit, principal plus interest back at maturity.
    let bhd = ladder(&router, &head1, "BHD").await;
    assert!(bhd.liquidity_access);
    assert_eq!(bhd.rows.iter().find(|r| r.date == v).unwrap().net_expected, dec("-2376000.00"));
    let interest = Decimal::from(2_000_000) * dec("0.05") * Decimal::from(30) / Decimal::from(360);
    assert_eq!(bhd.rows.iter().find(|r| r.date == m).unwrap().net_expected, (Decimal::from(2_000_000) + interest).round_dp(2));
    assert_eq!(bhd.rows.last().unwrap().cumulative_expected, (Decimal::from(2_000_000) + interest - dec("2376000")).round_dp(2));
    assert!(bhd.currencies.contains(&"EUR".to_string()), "nostro accounts widen the currency list for liquidity users");

    // Drill-down lists the flows behind a row.
    let res = router.clone().oneshot(req(Method::GET, &format!("/ladder/flows?ccy=BHD&date={v}"), Some(&head1), None)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let flows: Vec<domain::CashFlow> = json(res).await;
    assert_eq!(flows.len(), 2);
    assert_eq!(flows.iter().map(|f| f.leg.as_str()).collect::<std::collections::BTreeSet<_>>(), ["FX_CCY2", "MM_START"].into_iter().collect());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn nostro_actuals_are_visible_to_liquidity_users_only(owner: PgPool) {
    let (router, _app, recon, _fx, _mm) = fixture(&owner).await;
    let fx1 = token(&router, "fx1").await;
    let alm1 = token(&router, "alm1").await;
    let today = Utc::now().date_naive();
    let v = today + Duration::days(2);

    sqlx::query(
        "INSERT INTO nostro_statements (account_id, statement_date, kind, opening_balance, closing_balance, received_at) \
         SELECT id, $1, 'MT940', 10000000.00, 10500000.00, now() FROM nostro_accounts WHERE ccy = 'USD'",
    )
    .bind(today - Duration::days(3))
    .execute(&recon)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO nostro_movements (account_id, value_date, booked_at, amount, reference, counterparty, deal_ref_hint) \
         SELECT id, $1, now(), 999950.00, 'NOSTRO-USD-1', 'GIB', 'FX-000001' FROM nostro_accounts WHERE ccy = 'USD'",
    )
    .bind(v)
    .execute(&recon)
    .await
    .unwrap();

    let usd = ladder(&router, &alm1, "USD").await;
    assert!(usd.liquidity_access);
    assert_eq!(usd.opening.as_ref().map(|o| o.balance), Some(dec("10500000.00")), "closing balance of the latest statement");
    let row = usd.rows.iter().find(|r| r.date == v).unwrap();
    assert_eq!(row.actual_net, Some(dec("999950.00")));
    assert_eq!(row.difference, Some(dec("-50.00")), "actual minus expected");
    assert_eq!(usd.rows[0].cumulative_expected, dec("10500000.00"), "cumulative starts from the opening balance");

    let usd = ladder(&router, &fx1, "USD").await;
    assert!(usd.rows.iter().all(|r| r.actual_net.is_none()), "no nostro data without liquidity access");
    let res = router.clone().oneshot(req(Method::GET, &format!("/ladder/movements?ccy=USD&date={v}"), Some(&fx1), None)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let m: Vec<domain::NostroMovement> = json(res).await;
    assert!(m.is_empty(), "movements are filtered away, not refused");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn amendments_follow_booking_rights_and_trigger_rules(owner: PgPool) {
    let (router, _app, recon, fx, _mm) = fixture(&owner).await;
    let fx1 = token(&router, "fx1").await;
    let mm1 = token(&router, "mm1").await;
    let alm1 = token(&router, "alm1").await;

    let amend = |tok: String, id: Uuid, body: serde_json::Value| {
        let router = router.clone();
        async move { router.oneshot(req(Method::PATCH, &format!("/deals/{id}"), Some(&tok), Some(body.to_string()))).await.unwrap() }
    };
    assert_eq!(amend(mm1.clone(), fx, serde_json::json!({"counterparty": "X"})).await.status(), StatusCode::NOT_FOUND, "another desk cannot see it");
    assert_eq!(amend(alm1.clone(), fx, serde_json::json!({"counterparty": "X"})).await.status(), StatusCode::FORBIDDEN, "read-only entitlement");
    assert_eq!(amend(fx1.clone(), fx, serde_json::json!({})).await.status(), StatusCode::BAD_REQUEST, "nothing to change");

    let res = amend(fx1.clone(), fx, serde_json::json!({"counterparty": "Gulf International Bank B.S.C.", "rate": "0.37610"})).await;
    assert_eq!(res.status(), StatusCode::OK);
    let d: Deal = json(res).await;
    assert_eq!(d.version, 2);
    assert_eq!(d.rate, dec("0.37610000"));
    let kinds: Vec<String> = sqlx::query_scalar("SELECT event_kind FROM source_events WHERE source = 'INTERNAL' ORDER BY id").fetch_all(&owner).await.unwrap();
    assert_eq!(kinds, vec!["NEW", "NEW", "AMEND"], "the amendment enters the reconciliation log");

    // External-origin deals are the reconciler's.
    let external: Uuid = sqlx::query_scalar(
        "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, trade_date, value_date, counterparty, trader_id, status, origin) \
         SELECT gen_random_uuid(), 'FX-000009', id, 'FX_SPOT', 'SELL', 'EUR', 'USD', 500000, 1.085, current_date, current_date + 2, 'HSBC', NULL, 'BOOKED', 'KONDOR' \
         FROM desks WHERE code = 'FX' RETURNING id",
    )
    .fetch_one(&recon)
    .await
    .unwrap();
    assert_eq!(amend(fx1.clone(), external, serde_json::json!({"counterparty": "X"})).await.status(), StatusCode::FORBIDDEN);

    // Cancel, then nothing more may change.
    let res = router.clone().oneshot(req(Method::POST, &format!("/deals/{fx}/cancel"), Some(&fx1), None)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(json::<Deal>(res).await.status, domain::DealStatus::Cancelled);
    let res = amend(fx1.clone(), fx, serde_json::json!({"counterparty": "Y"})).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST, "a cancelled deal is frozen");
    let audits: Vec<String> = sqlx::query_scalar("SELECT action FROM audit_log WHERE action LIKE 'deal_%' ORDER BY id").fetch_all(&owner).await.unwrap();
    assert_eq!(audits, vec!["deal_amend", "deal_cancel"]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn history_lists_every_version_with_field_changes(owner: PgPool) {
    let (router, _app, _recon, fx, _mm) = fixture(&owner).await;
    let fx1 = token(&router, "fx1").await;
    let mm1 = token(&router, "mm1").await;

    let body = serde_json::json!({"counterparty": "Gulf International Bank B.S.C."}).to_string();
    let res = router.clone().oneshot(req(Method::PATCH, &format!("/deals/{fx}"), Some(&fx1), Some(body))).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let res = router.clone().oneshot(req(Method::GET, &format!("/deals/{fx}/history"), Some(&fx1), None)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let h: Vec<HistoryEntry> = json(res).await;
    assert_eq!(h.len(), 2);
    assert_eq!((h[0].version, h[0].op.as_str()), (1, "INSERT"));
    assert!(h[0].changes.is_empty());
    assert_eq!((h[1].version, h[1].op.as_str()), (2, "UPDATE"));
    assert_eq!(h[1].changed_by.as_deref(), Some("FX Trader One"));
    assert_eq!(h[1].changed_by_role, "treasury_app");
    assert_eq!(h[1].changes.len(), 1);
    assert_eq!((h[1].changes[0].field.as_str(), h[1].changes[0].from.as_deref(), h[1].changes[0].to.as_deref()),
               ("counterparty", Some("Gulf International Bank"), Some("Gulf International Bank B.S.C.")));

    let res = router.clone().oneshot(req(Method::GET, &format!("/deals/{fx}/history"), Some(&mm1), None)).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
