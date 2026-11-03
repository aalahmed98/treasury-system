//! Row-level security, privilege boundaries and auth behaviour, each against a fresh database.
//! `DATABASE_URL` must be the owner role (sqlx::test creates databases with it) and
//! `APP_DATABASE_URL` supplies the application role's credentials.

use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, Response, StatusCode, header};
use domain::{DealsResponse, LoginResponse};
use http_body_util::BodyExt;
use server::auth::session;
use server::config::Config;
use server::{AppState, db, seed};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgListener, PgPoolOptions};
use tower::ServiceExt;
use uuid::Uuid;

const PEER: &str = "127.0.0.1:40000";

/// A pool connected as the application role (no RLS bypass) to the same test database.
async fn app_pool(owner: &PgPool) -> PgPool {
    let url = std::env::var("APP_DATABASE_URL").expect("APP_DATABASE_URL (app role) must be set");
    let creds = PgConnectOptions::from_str(&url).expect("APP_DATABASE_URL parses");
    let test_db = owner.connect_options().get_database().expect("test database has a name").to_string();
    PgPoolOptions::new().max_connections(4).connect_with(creds.database(&test_db)).await.expect("app role connects")
}

/// Users and entitlements from the seed, plus a deterministic book of internal deals:
/// 150 FX by fx1, 150 FX by fx2, 200 MM by mm1 (every tenth one settled).
async fn seeded(owner: &PgPool) -> PgPool {
    let app = app_pool(owner).await;
    seed::run(owner, &app).await.expect("seed runs");
    book_many(&app, "fx1", "FX", 150).await;
    book_many(&app, "fx2", "FX", 150).await;
    book_many(&app, "mm1", "MM", 200).await;
    app
}

async fn book_many(app: &PgPool, user: &str, desk: &str, n: i32) {
    let fx = desk == "FX";
    let mut tx = db::begin_as(app, seed::user_id(user)).await.unwrap();
    sqlx::query(
        "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, \
                            trade_date, value_date, maturity_date, counterparty, trader_id, status) \
         SELECT gen_random_uuid(), $1 || '-' || lpad(nextval('deal_ref_seq')::text, 6, '0'), k.id, \
                $2, $3, $4, $5, 1000000 * (1 + g % 5), $6, \
                current_date - (g % 20), current_date - (g % 20) + 2, \
                CASE WHEN $7 THEN NULL ELSE current_date - (g % 20) + 32 END, \
                'Test Bank ' || (g % 4), $8, CASE WHEN g % 10 = 0 THEN 'SETTLED' ELSE 'BOOKED' END \
         FROM desks k, generate_series(1, $9) g WHERE k.code = $1",
    )
    .bind(desk)
    .bind(if fx { "FX_SPOT" } else { "MM_DEPO" })
    .bind(if fx { "BUY" } else { "LEND" })
    .bind(if fx { "USD" } else { "BHD" })
    .bind(if fx { Some("BHD") } else { None })
    .bind(if fx { rust_decimal::Decimal::new(376, 3) } else { rust_decimal::Decimal::new(525, 2) })
    .bind(fx)
    .bind(seed::user_id(user))
    .bind(n)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

fn state(app: PgPool) -> AppState {
    state_with(app, Config::default())
}

fn state_with(app: PgPool, config: Config) -> AppState {
    AppState::new(app, config)
}

fn req(method: Method, uri: &str, token: Option<&str>, body: Option<&str>, peer: &str) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .extension(ConnectInfo(peer.parse::<SocketAddr>().unwrap()))
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    b.body(body.map(|s| Body::from(s.to_string())).unwrap_or_else(Body::empty)).unwrap()
}

async fn json<T: serde::de::DeserializeOwned>(res: Response<Body>) -> T {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).expect("json body")
}

async fn login(router: &axum::Router, username: &str, password: &str, peer: &str) -> Response<Body> {
    let body = serde_json::json!({ "username": username, "password": password }).to_string();
    router.clone().oneshot(req(Method::POST, "/auth/login", None, Some(&body), peer)).await.unwrap()
}

async fn login_ok(router: &axum::Router, username: &str) -> LoginResponse {
    let res = login(router, username, seed::DEV_PASSWORD, PEER).await;
    assert_eq!(res.status(), StatusCode::OK);
    json(res).await
}

async fn count_as(app: &PgPool, user: &str) -> i64 {
    let mut tx = db::begin_as(app, seed::user_id(user)).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM deals").fetch_one(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    n
}

async fn desks_seen(app: &PgPool, user: &str) -> Vec<String> {
    let mut tx = db::begin_as(app, seed::user_id(user)).await.unwrap();
    let v: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT k.code FROM deals d JOIN desks k ON k.id = d.desk_id ORDER BY 1")
            .fetch_all(&mut *tx)
            .await
            .unwrap();
    tx.commit().await.unwrap();
    v
}

/// Book a money-market deal on `desk_code` as `user`, attributed to `trader`.
async fn insert_as(app: &PgPool, user: &str, desk_code: &str, trader: &str) -> sqlx::Result<()> {
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
    .bind(seed::user_id(trader))
    .bind(desk_code)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

fn sqlstate(e: &sqlx::Error) -> Option<String> {
    match e {
        sqlx::Error::Database(d) => d.code().map(|c| c.to_string()),
        _ => None,
    }
}

async fn grant_read(owner: &PgPool, user: &str, desk_code: &str) {
    sqlx::query(
        "INSERT INTO user_desk_access (user_id, desk_id, can_book) \
         SELECT $1, id, false FROM desks WHERE code = $2",
    )
    .bind(seed::user_id(user))
    .bind(desk_code)
    .execute(owner)
    .await
    .unwrap();
}

/// Read SSE frames until one carries a `deal` event or the timeout passes.
async fn next_deal_event(body: &mut Body, timeout: Duration) -> Option<String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, body.frame()).await {
            Ok(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    let text = String::from_utf8_lossy(data).to_string();
                    if text.contains("event: deal") {
                        return Some(text);
                    }
                }
            }
            _ => return None,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Entitlement matrix
// ---------------------------------------------------------------------------------------------

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
    // The owner is not a superuser; it sees everything through an explicit admin policy.
    let all: i64 = sqlx::query_scalar("SELECT count(*) FROM deals").fetch_one(&owner).await.unwrap();
    assert_eq!(all, 500);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn booking_requires_can_book_and_self_attribution(owner: PgPool) {
    let app = seeded(&owner).await;
    let err = insert_as(&app, "mm1", "FX", "mm1").await.expect_err("MM trader cannot book FX");
    assert_eq!(sqlstate(&err).as_deref(), Some("42501"), "expected RLS violation, got {err}");

    insert_as(&app, "alm1", "MM", "alm1").await.expect_err("ALM has read-only entitlement");

    let err = insert_as(&app, "fx1", "FX", "fx2").await.expect_err("a deal cannot be attributed to another trader");
    assert_eq!(sqlstate(&err).as_deref(), Some("42501"));

    insert_as(&app, "mm1", "MM", "mm1").await.expect("MM trader books MM");
    assert_eq!(count_as(&app, "mm1").await, 201);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn amendment_requires_booking_rights_on_the_deal_desk(owner: PgPool) {
    let app = seeded(&owner).await;
    // mm1 has no FX access yet
    // Hypothetical future user: reads FX, books MM.
    grant_read(&owner, "mm1", "FX").await;
    assert_eq!(desks_seen(&app, "mm1").await, vec!["FX", "MM"], "mm1 now reads both desks");

    let mut tx = db::begin_as(&app, seed::user_id("mm1")).await.unwrap();
    let touched = sqlx::query("UPDATE deals SET counterparty = 'X' WHERE desk_id = (SELECT id FROM desks WHERE code = 'FX')")
        .execute(&mut *tx)
        .await
        .unwrap()
        .rows_affected();
    assert_eq!(touched, 0, "read-only entitlement must not allow amendments");
    tx.commit().await.unwrap();
    // The API role has no UPDATE privilege on desk_id at all, so a desk move is refused outright.
    let mut tx = db::begin_as(&app, seed::user_id("mm1")).await.unwrap();
    let err = sqlx::query(
        "UPDATE deals SET desk_id = (SELECT id FROM desks WHERE code = 'MM') \
         WHERE desk_id = (SELECT id FROM desks WHERE code = 'FX')",
    )
    .execute(&mut *tx)
    .await
    .expect_err("deals cannot be pulled from a read-only desk into a bookable one");
    assert_eq!(sqlstate(&err).as_deref(), Some("42501"));
    drop(tx);

    // A legitimate amendment on the bookable desk works, bumps the version and is journaled.
    let mut tx = db::begin_as(&app, seed::user_id("mm1")).await.unwrap();
    let id: Uuid = sqlx::query_scalar(
        "SELECT id FROM deals WHERE desk_id = (SELECT id FROM desks WHERE code = 'MM') \
         AND status IN ('EXECUTED', 'BOOKED', 'CONFIRMED') LIMIT 1",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let version: i32 = sqlx::query_scalar("UPDATE deals SET counterparty = 'Amended Bank' WHERE id = $1 RETURNING version")
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(version, 2);
    tx.commit().await.unwrap();

    let mut tx = db::begin_as(&app, seed::user_id("mm1")).await.unwrap();
    let err = sqlx::query("UPDATE deals SET desk_id = (SELECT id FROM desks WHERE code = 'FX') WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .expect_err("desk is immutable");
    assert!(sqlstate(&err).as_deref() == Some("P0001") || sqlstate(&err).as_deref() == Some("42501"), "{err}");
    drop(tx);

    let history: Vec<(i32, String, Option<Uuid>)> =
        sqlx::query_as("SELECT version, op, changed_by FROM deal_history WHERE deal_id = $1 ORDER BY version")
            .bind(id)
            .fetch_all(&owner)
            .await
            .unwrap();
    assert_eq!(history.len(), 2, "one row per version");
    assert_eq!(history[1].0, 2);
    assert_eq!(history[1].1, "UPDATE");
    assert_eq!(history[1].2, Some(seed::user_id("mm1")), "the amending user is recorded");

    // Terminal deals are frozen.
    let mut tx = db::begin_as(&app, seed::user_id("mm1")).await.unwrap();
    let err = sqlx::query(
        "UPDATE deals SET counterparty = 'X' WHERE id = (SELECT id FROM deals WHERE status = 'SETTLED' \
         AND desk_id = (SELECT id FROM desks WHERE code = 'MM') LIMIT 1)",
    )
    .execute(&mut *tx)
    .await
    .expect_err("settled deals cannot be amended");
    assert_eq!(sqlstate(&err).as_deref(), Some("P0001"));
}

// ---------------------------------------------------------------------------------------------
// Privilege boundary of the application role
// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrations = "../../db/migrations")]
async fn app_role_cannot_escalate(owner: PgPool) {
    let app = seeded(&owner).await;
    let denied = |sql: &'static str| {
        let app = app.clone();
        async move {
            let err = sqlx::query(sql).execute(&app).await.expect_err(sql);
            assert_eq!(sqlstate(&err).as_deref(), Some("42501"), "{sql}: {err}");
        }
    };
    denied("INSERT INTO user_desk_access (user_id, desk_id, can_book) SELECT u.id, k.id, true FROM users u, desks k").await;
    denied("UPDATE user_desk_access SET can_book = true").await;
    denied("INSERT INTO users (id, username, display_name, home_desk_id) VALUES (gen_random_uuid(), 'backdoor', 'x', 1)").await;
    denied("UPDATE users SET is_active = false").await;
    denied("SELECT password_hash FROM user_credentials").await;
    denied("UPDATE user_credentials SET password_hash = 'x'").await;
    denied("SELECT * FROM login_throttle").await;
    denied("SELECT * FROM deal_history").await;
    denied("DELETE FROM deal_history").await;
    denied("SELECT * FROM audit_log").await;
    denied("DELETE FROM audit_log").await;
    denied("UPDATE audit_log SET action = 'x'").await;
    denied("DELETE FROM deals").await;

    // The narrow questions it may ask still work.
    let (uid, hash): (Option<Uuid>, Option<String>) =
        sqlx::query_as("SELECT user_id, password_hash FROM auth_lookup('fx1')").fetch_one(&app).await.unwrap();
    assert_eq!(uid, Some(seed::user_id("fx1")));
    assert!(hash.unwrap().starts_with("$argon2id$"));
    let (uid, hash): (Option<Uuid>, Option<String>) =
        sqlx::query_as("SELECT user_id, password_hash FROM auth_lookup('nobody')").fetch_one(&app).await.unwrap();
    assert!(uid.is_none() && hash.is_none(), "unknown users still yield exactly one row");
}

// ---------------------------------------------------------------------------------------------
// Sessions and login hardening
// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrations = "../../db/migrations")]
async fn sessions_expire_absolutely_and_on_idle(owner: PgPool) {
    let app = seeded(&owner).await;
    let router = server::app(state(app.clone()));
    let fx1 = seed::user_id("fx1");

    let live = session::create(&app, fx1, chrono::Duration::hours(1)).await.unwrap();
    let dead = session::create(&app, fx1, chrono::Duration::hours(-1)).await.unwrap();
    let idle = session::create(&app, fx1, chrono::Duration::hours(1)).await.unwrap();
    sqlx::query("UPDATE sessions SET last_seen_at = now() - interval '31 minutes' WHERE token_hash = $1")
        .bind(session::hash_token(&idle.token))
        .execute(&owner)
        .await
        .unwrap();

    let me = |tok: &str| req(Method::GET, "/me", Some(tok), None, PEER);
    assert_eq!(router.clone().oneshot(me(&live.token)).await.unwrap().status(), StatusCode::OK);
    assert_eq!(router.clone().oneshot(me(&dead.token)).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(router.clone().oneshot(me(&idle.token)).await.unwrap().status(), StatusCode::UNAUTHORIZED, "idle for longer than SESSION_IDLE_MINUTES");
    assert_eq!(router.clone().oneshot(me("garbage")).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(router.clone().oneshot(req(Method::GET, "/me", None, None, PEER)).await.unwrap().status(), StatusCode::UNAUTHORIZED);

    // Only hashes are stored.
    let stored: Vec<Vec<u8>> = sqlx::query_scalar("SELECT token_hash FROM sessions").fetch_all(&owner).await.unwrap();
    assert!(stored.iter().all(|h| h.len() == 32 && h != live.token.as_bytes()));

    // A passive (event-driven) request proves liveness without counting as activity.
    let before: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("SELECT last_seen_at FROM sessions WHERE token_hash = $1")
        .bind(session::hash_token(&live.token))
        .fetch_one(&owner)
        .await
        .unwrap();
    let mut passive = me(&live.token);
    passive.headers_mut().insert("x-treasury-passive", "1".parse().unwrap());
    assert_eq!(router.clone().oneshot(passive).await.unwrap().status(), StatusCode::OK);
    let after: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("SELECT last_seen_at FROM sessions WHERE token_hash = $1")
        .bind(session::hash_token(&live.token))
        .fetch_one(&owner)
        .await
        .unwrap();
    assert_eq!(before, after, "passive requests do not extend the idle window");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn login_then_list_deals_over_http(owner: PgPool) {
    let app = seeded(&owner).await;
    let router = server::app(state(app.clone()));

    assert_eq!(login(&router, "fx1", "nope", PEER).await.status(), StatusCode::UNAUTHORIZED);
    let login = login_ok(&router, "fx1").await;
    assert_eq!(login.user.username, "fx1");
    assert_eq!(login.desks.len(), 1);

    let res = router.clone().oneshot(req(Method::GET, "/deals", Some(&login.token), None, PEER)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers().get("x-request-id").is_some(), "responses carry a request id");
    let deals: DealsResponse = json(res).await;
    assert_eq!(deals.deals.len(), 300);
    assert!(deals.deals.iter().all(|d| d.desk_code == "FX"));

    let audits: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT action, host(client_ip), request_id FROM audit_log WHERE username = 'fx1' ORDER BY id",
    )
    .fetch_all(&owner)
    .await
    .unwrap();
    assert_eq!(audits.len(), 2, "one failed and one successful login audited");
    assert_eq!(audits[0].0, "login_failed");
    assert_eq!(audits[1].0, "login_ok");
    assert_eq!(audits[1].1.as_deref(), Some("127.0.0.1"), "client ip recorded");
    assert!(audits[1].2.is_some(), "request id recorded");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn login_locks_after_repeated_failures(owner: PgPool) {
    let app = seeded(&owner).await;
    let router = server::app(state(app.clone()));

    for attempt in 1..=4 {
        assert_eq!(login(&router, "fx1", "wrong", PEER).await.status(), StatusCode::UNAUTHORIZED, "attempt {attempt}");
    }
    let res = login(&router, "fx1", "wrong", PEER).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS, "5th failure locks the username");
    assert!(res.headers().get(header::RETRY_AFTER).is_some());

    // Locked means locked, even with the right password.
    assert_eq!(login(&router, "fx1", seed::DEV_PASSWORD, PEER).await.status(), StatusCode::TOO_MANY_REQUESTS);

    // Unknown usernames behave identically, so lockout responses do not reveal which users exist.
    for _ in 1..=4 {
        assert_eq!(login(&router, "ghost", "wrong", PEER).await.status(), StatusCode::UNAUTHORIZED);
    }
    assert_eq!(login(&router, "ghost", "wrong", PEER).await.status(), StatusCode::TOO_MANY_REQUESTS);

    let locked: Vec<(String, bool)> = sqlx::query_as("SELECT action, coalesce(detail ? 'locked_until', false) FROM audit_log WHERE username = 'fx1' ORDER BY id DESC LIMIT 2")
        .fetch_all(&owner)
        .await
        .unwrap();
    assert!(locked.iter().any(|(a, _)| a == "login_locked"), "lock hit is audited: {locked:?}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn login_is_rate_limited_per_ip(owner: PgPool) {
    let app = seeded(&owner).await;
    let router = server::app(state_with(app.clone(), Config { login_ip_limit: 3, ..Config::default() }));

    for _ in 0..3 {
        assert_eq!(login(&router, "fx1", seed::DEV_PASSWORD, "10.1.1.1:5").await.status(), StatusCode::OK);
    }
    assert_eq!(login(&router, "fx1", seed::DEV_PASSWORD, "10.1.1.1:5").await.status(), StatusCode::TOO_MANY_REQUESTS, "4th attempt from the same ip");
    assert_eq!(login(&router, "fx1", seed::DEV_PASSWORD, "10.1.1.2:5").await.status(), StatusCode::OK, "other ips unaffected");
}

// ---------------------------------------------------------------------------------------------
// Live events
// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrations = "../../db/migrations")]
async fn insert_notifies_deal_events(owner: PgPool) {
    let app = seeded(&owner).await;
    let mut listener = PgListener::connect_with(&app).await.unwrap();
    listener.listen("deal_events").await.unwrap();

    insert_as(&app, "mm1", "MM", "mm1").await.unwrap();

    let n = tokio::time::timeout(Duration::from_secs(2), listener.recv()).await.expect("notification within 2s").unwrap();
    let payload: serde_json::Value = serde_json::from_str(n.payload()).unwrap();
    assert_eq!(payload["op"], "INSERT");
    assert_eq!(payload["desk_id"], 2);
    assert!(payload["id"].as_str().map(|s| Uuid::parse_str(s).is_ok()).unwrap_or(false));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn event_stream_closes_on_logout(owner: PgPool) {
    let app = seeded(&owner).await;
    let router = server::app(state(app.clone()));
    let login = login_ok(&router, "fx1").await;

    let res = router.clone().oneshot(req(Method::GET, "/events", Some(&login.token), None, PEER)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body();
    let drained = tokio::spawn(async move { body.collect().await.map(|c| c.to_bytes().len()) });

    let res = router.clone().oneshot(req(Method::POST, "/auth/logout", Some(&login.token), None, PEER)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    tokio::time::timeout(Duration::from_secs(5), drained)
        .await
        .expect("stream ends promptly after logout")
        .unwrap()
        .unwrap();
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn event_stream_closes_when_session_dies(owner: PgPool) {
    let app = seeded(&owner).await;
    let cfg = Config { stream_check: Duration::from_millis(200), ..Config::default() };
    let router = server::app(state_with(app.clone(), cfg));
    let login = login_ok(&router, "fx1").await;

    let res = router.clone().oneshot(req(Method::GET, "/events", Some(&login.token), None, PEER)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body();
    let drained = tokio::spawn(async move { body.collect().await.map(|_| ()) });

    // Simulate the idle timeout passing.
    sqlx::query("UPDATE sessions SET last_seen_at = now() - interval '2 hours'").execute(&owner).await.unwrap();

    tokio::time::timeout(Duration::from_secs(5), drained)
        .await
        .expect("stream ends within a check interval of the session dying")
        .unwrap()
        .unwrap();
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn event_stream_honours_entitlement_changes(owner: PgPool) {
    let app = seeded(&owner).await;
    let cfg = Config { stream_check: Duration::from_millis(200), ..Config::default() };
    let state = state_with(app.clone(), cfg);
    tokio::spawn(server::listener::run(app.clone(), state.events.clone()));
    let router = server::app(state);
    let login = login_ok(&router, "alm1").await; // reads FX, MM, ALM
    tokio::time::sleep(Duration::from_millis(300)).await; // let the listener subscribe

    let res = router.clone().oneshot(req(Method::GET, "/events", Some(&login.token), None, PEER)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let mut body = res.into_body();

    insert_as(&app, "fx1", "FX", "fx1").await.unwrap();
    assert!(next_deal_event(&mut body, Duration::from_secs(3)).await.is_some(), "FX deal reaches alm1 while entitled");

    // Revoke FX access; the stream must stop forwarding FX within a check interval.
    sqlx::query("DELETE FROM user_desk_access WHERE user_id = $1 AND desk_id = (SELECT id FROM desks WHERE code = 'FX')")
        .bind(seed::user_id("alm1"))
        .execute(&owner)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;

    insert_as(&app, "fx1", "FX", "fx1").await.unwrap();
    assert!(next_deal_event(&mut body, Duration::from_secs(2)).await.is_none(), "FX deal must not reach alm1 after revocation");

    insert_as(&app, "mm1", "MM", "mm1").await.unwrap();
    assert!(next_deal_event(&mut body, Duration::from_secs(3)).await.is_some(), "MM deals still flow");
}
