//! Row-level security and auth behaviour, run against a fresh database per test.
//! `DATABASE_URL` must point at the owner role so sqlx::test can create databases.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use domain::{DealsResponse, LoginResponse};
use http_body_util::BodyExt;
use server::auth::session;
use server::{AppState, db, seed};
use sqlx::postgres::{PgListener, PgPoolOptions};
use sqlx::PgPool;
use std::time::Duration;
use tokio::sync::broadcast;
use tower::ServiceExt;
use uuid::Uuid;

const MIGRATIONS: &str = "../../db/migrations";

/// A pool connected as the application role (no RLS bypass) to the same test database.
async fn app_pool(owner: &PgPool) -> PgPool {
    let opts = (*owner.connect_options()).clone().username("treasury_app").password("app_dev");
    PgPoolOptions::new().max_connections(4).connect_with(opts).await.expect("app role connects")
}

async fn seeded(owner: &PgPool) -> PgPool {
    let app = app_pool(owner).await;
    seed::run(&app).await.expect("seed runs under the app role");
    app
}

async fn count_as(app: &PgPool, user: &str) -> i64 {
    let mut tx = db::begin_as(app, seed::user_id(user)).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM deals").fetch_one(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    n
}

async fn desks_seen(app: &PgPool, user: &str) -> Vec<String> {
    let mut tx = db::begin_as(app, seed::user_id(user)).await.unwrap();
    let v: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT k.code FROM deals d JOIN desks k ON k.id = d.desk_id ORDER BY 1",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    v
}

fn state(app: PgPool) -> AppState {
    let (events, _) = broadcast::channel(16);
    AppState { pool: app, events, session_ttl: chrono::Duration::hours(1) }
}

async fn insert_as(app: &PgPool, user: &str, desk_code: &str) -> sqlx::Result<()> {
    let mut tx = db::begin_as(app, seed::user_id(user)).await?;
    sqlx::query(
        "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, \
                            trade_date, value_date, counterparty, trader_id, status) \
         SELECT $1, $2, id, 'MM_DEPO', 'LEND', 'BHD', NULL, 1000000, 5.0, \
                current_date, current_date, 'Test Bank', $3, 'BOOKED' \
         FROM desks WHERE code = $4",
    )
    .bind(Uuid::now_v7())
    .bind(format!("T-{}", Uuid::now_v7()))
    .bind(seed::user_id(user))
    .bind(desk_code)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn fx_trader_sees_only_fx(owner: PgPool) {
    let app = seeded(&owner).await;
    assert_eq!(count_as(&app, "fx1").await, 300);
    assert_eq!(desks_seen(&app, "fx1").await, vec!["FX"]);
    assert_eq!(count_as(&app, "mm1").await, 200);
    assert_eq!(desks_seen(&app, "mm1").await, vec!["MM"]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn oversight_roles_see_all_desks(owner: PgPool) {
    let app = seeded(&owner).await;
    assert_eq!(count_as(&app, "alm1").await, 500);
    assert_eq!(count_as(&app, "head1").await, 500);
    assert_eq!(desks_seen(&app, "head1").await, vec!["FX", "MM"]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn unset_identity_sees_nothing(owner: PgPool) {
    let app = seeded(&owner).await;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM deals").fetch_one(&app).await.unwrap();
    assert_eq!(n, 0, "app role without app.user_id must see no rows");
    let p: i64 = sqlx::query_scalar("SELECT count(*) FROM positions_v").fetch_one(&app).await.unwrap();
    assert_eq!(p, 0, "view must inherit RLS via security_invoker");
    // The owner (superuser) bypasses RLS, which is why the server never connects as it.
    let all: i64 = sqlx::query_scalar("SELECT count(*) FROM deals").fetch_one(&owner).await.unwrap();
    assert_eq!(all, 500);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn booking_requires_can_book_on_that_desk(owner: PgPool) {
    let app = seeded(&owner).await;
    let err = insert_as(&app, "mm1", "FX").await.expect_err("MM trader cannot book FX");
    let code = match &err {
        sqlx::Error::Database(d) => d.code().map(|c| c.to_string()),
        _ => None,
    };
    assert_eq!(code.as_deref(), Some("42501"), "expected RLS violation, got {err}");

    insert_as(&app, "alm1", "MM").await.expect_err("ALM has read-only entitlement");
    insert_as(&app, "mm1", "MM").await.expect("MM trader books MM");
    assert_eq!(count_as(&app, "mm1").await, 201);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn sessions_expire_and_tokens_are_hashed(owner: PgPool) {
    let app = seeded(&owner).await;
    let router = server::app(state(app.clone()));

    let live = session::create(&app, seed::user_id("fx1"), chrono::Duration::hours(1)).await.unwrap();
    let dead = session::create(&app, seed::user_id("fx1"), chrono::Duration::hours(-1)).await.unwrap();

    let get_me = |tok: String| {
        Request::builder().uri("/me").header(header::AUTHORIZATION, format!("Bearer {tok}")).body(Body::empty()).unwrap()
    };
    assert_eq!(router.clone().oneshot(get_me(live.token.clone())).await.unwrap().status(), StatusCode::OK);
    assert_eq!(router.clone().oneshot(get_me(dead.token.clone())).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(router.clone().oneshot(get_me("garbage".into())).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    let no_auth = Request::builder().uri("/me").body(Body::empty()).unwrap();
    assert_eq!(router.clone().oneshot(no_auth).await.unwrap().status(), StatusCode::UNAUTHORIZED);

    // The raw token never reaches the database.
    let stored: Vec<Vec<u8>> = sqlx::query_scalar("SELECT token_hash FROM sessions").fetch_all(&owner).await.unwrap();
    assert!(stored.iter().all(|h| h.len() == 32 && h != live.token.as_bytes()));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn login_then_list_deals_over_http(owner: PgPool) {
    let app = seeded(&owner).await;
    let router = server::app(state(app.clone()));

    let bad = Request::post("/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"username":"fx1","password":"nope"}"#))
        .unwrap();
    assert_eq!(router.clone().oneshot(bad).await.unwrap().status(), StatusCode::UNAUTHORIZED);

    let ok = Request::post("/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"username":"fx1","password":"password"}"#))
        .unwrap();
    let res = router.clone().oneshot(ok).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let login: LoginResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(login.user.username, "fx1");
    assert_eq!(login.desks.len(), 1);

    let req = Request::get("/deals")
        .header(header::AUTHORIZATION, format!("Bearer {}", login.token))
        .body(Body::empty())
        .unwrap();
    let res = router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let deals: DealsResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(deals.deals.len(), 300);
    assert!(deals.deals.iter().all(|d| d.desk_code == "FX"));

    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE username = 'fx1'").fetch_one(&owner).await.unwrap();
    assert_eq!(audits, 2, "one failed and one successful login audited");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn insert_notifies_deal_events(owner: PgPool) {
    let app = seeded(&owner).await;
    let mut listener = PgListener::connect_with(&app).await.unwrap();
    listener.listen("deal_events").await.unwrap();

    insert_as(&app, "mm1", "MM").await.unwrap();

    let n = tokio::time::timeout(Duration::from_secs(2), listener.recv()).await.expect("notification within 2s").unwrap();
    let payload: serde_json::Value = serde_json::from_str(n.payload()).unwrap();
    assert_eq!(payload["op"], "INSERT");
    assert_eq!(payload["desk_id"], 2);
    assert!(payload["id"].as_str().map(|s| Uuid::parse_str(s).is_ok()).unwrap_or(false));
}
