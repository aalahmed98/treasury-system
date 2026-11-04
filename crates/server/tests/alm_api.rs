//! The ALM endpoints over HTTP: scoping, the saved parameters, snapshots, and the LCR against the
//! pure crate's hand-worked case.

use std::net::SocketAddr;
use std::str::FromStr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, Response, StatusCode, header};
use chrono::Utc;
use domain::{AlmParams, AlmParamsResponse, AlmSnapshot, LcrResult, LoginResponse};
use http_body_util::BodyExt;
use server::config::Config;
use server::{AppState, seed};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tower::ServiceExt;

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

/// Today's snapshot: the hand-worked LCR case (stock 1000; outflows 250 + 1000; inflows 800 under
/// the 75% cap; net 450), plus an incomplete snapshot that must stay invisible.
async fn fixture(owner: &PgPool) -> axum::Router {
    let app = pool_for(owner, "APP_DATABASE_URL").await;
    seed::run(owner, &app).await.unwrap();
    let today = Utc::now().date_naive();
    sqlx::query("INSERT INTO bs_snapshots (as_of, business_date, generated_at, seed, complete) VALUES ($1, $1, now(), 1, true), ($1 + 1, $1 + 1, now(), 1, false)")
        .bind(today)
        .execute(owner)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO bs_positions (as_of, position_id, side, book, segment, product, ccy, balance, rate, rate_type, hqla_level, lcr_class, maturity) VALUES \
         ($1, 'CASH', 'ASSET', 'CASH', 'CENTRAL_BANK', 'CASH', 'BHD', 1000, 0, 'NONE', 'L1', NULL, NULL), \
         ($1, 'RETAIL', 'LIABILITY', 'DEPOSITS', 'RETAIL', 'CURRENT_ACCOUNT', 'BHD', 5000, 0, 'ADMIN', NULL, 'RET_STABLE', NULL), \
         ($1, 'FI', 'LIABILITY', 'DEPOSITS', 'FI', 'TERM_DEPOSIT', 'BHD', 1000, 4, 'FIXED', NULL, 'FI_WHOLESALE', $1 + 10), \
         ($1, 'PLACE', 'ASSET', 'INTERBANK', 'FI', 'PLACEMENT', 'BHD', 800, 0, 'FIXED', NULL, 'INFLOW_FI', $1 + 5)",
    )
    .bind(today)
    .execute(owner)
    .await
    .unwrap();
    server::app(AppState::new(app, Config::default()))
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn parameters_and_snapshots_are_scoped(owner: PgPool) {
    let router = fixture(&owner).await;
    let fx1 = token(&router, "fx1").await;
    let alm1 = token(&router, "alm1").await;
    for uri in ["/alm/params", "/alm/snapshots"] {
        let res = router.clone().oneshot(req(Method::GET, uri, Some(&fx1), None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{uri} for fx1");
    }
    let res = router.clone().oneshot(req(Method::GET, "/alm/params", Some(&alm1), None)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let p: AlmParamsResponse = json(res).await;
    assert_eq!(p.active.version, 1);
    assert!(p.active.is_active);
    assert_eq!(p.params, AlmParams::default(), "version 1 is the application's defaults");
    assert_eq!(p.history.len(), 1);

    let res = router.clone().oneshot(req(Method::GET, "/alm/snapshots", Some(&alm1), None)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let s: Vec<AlmSnapshot> = json(res).await;
    assert_eq!(s.len(), 1, "the incomplete snapshot is not listed");
    assert_eq!(s[0].positions, 4);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn lcr_matches_the_hand_worked_case_and_accepts_what_ifs(owner: PgPool) {
    let router = fixture(&owner).await;
    let fx1 = token(&router, "fx1").await;
    let head1 = token(&router, "head1").await;
    let res = router.clone().oneshot(req(Method::POST, "/alm/lcr", Some(&fx1), Some("{}".into()))).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = router.clone().oneshot(req(Method::POST, "/alm/lcr", Some(&head1), Some("{}".into()))).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let r: LcrResult = json(res).await;
    assert_eq!(r.params_version, Some(1));
    assert!((r.total.stock - 1000.0).abs() < 1e-9);
    assert!((r.total.total_outflows - 1250.0).abs() < 1e-9);
    assert!((r.total.capped_inflows - 800.0).abs() < 1e-9);
    assert!((r.total.net_outflows - 450.0).abs() < 1e-9);
    assert!((r.total.ratio.unwrap() - 222.222222).abs() < 1e-5);
    assert_eq!(r.by_ccy.len(), 1);

    // The same call again is served from the cache and is identical.
    let res = router.clone().oneshot(req(Method::POST, "/alm/lcr", Some(&head1), Some("{}".into()))).await.unwrap();
    let again: LcrResult = json(res).await;
    assert_eq!(again, r);

    // An unsaved what-if: doubling the stable retail run-off.
    let mut p = AlmParams::default();
    p.lcr.outflow_rates.insert("RET_STABLE".into(), 10.0);
    let body = serde_json::json!({ "params": p }).to_string();
    let res = router.clone().oneshot(req(Method::POST, "/alm/lcr", Some(&head1), Some(body))).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let w: LcrResult = json(res).await;
    assert_eq!(w.params_version, None);
    assert!((w.total.net_outflows - 700.0).abs() < 1e-9);
    assert!((w.total.ratio.unwrap() - 142.857142).abs() < 1e-5);

    // An unknown or incomplete snapshot date is refused.
    let res = router.clone().oneshot(req(Method::POST, "/alm/lcr", Some(&head1), Some(r#"{"as_of":"2030-01-01"}"#.into()))).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}
