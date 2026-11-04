//! The FX risk endpoints over HTTP against a hand-worked book, the limits' versioning and who may
//! change them, breach sign-off, and the ALCO pack's commentary and sign-off.

use std::net::SocketAddr;
use std::str::FromStr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, Response, StatusCode, header};
use chrono::{Duration, Utc};
use domain::{AlcoPack, Breach, FxRiskResult, LimitStatus, LoginResponse, RiskLimitsResponse};
use http_body_util::BodyExt;
use server::config::Config;
use server::{AppState, db, seed};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tower::ServiceExt;

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
        .extension(ConnectInfo("127.0.0.1:40000".parse::<SocketAddr>().unwrap()))
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

async fn call(router: &axum::Router, method: Method, uri: &str, tok: &str, body: Option<serde_json::Value>) -> Response<Body> {
    router.clone().oneshot(req(method, uri, Some(tok), body.map(|b| b.to_string()))).await.unwrap()
}

/// The euro at 0.400 BHD until yesterday and 0.402 today; the dollar pegged at 0.376. No curves,
/// so nothing is discounted. fx1 bought 1m EUR yesterday at 0.400 (value tomorrow), and today sold
/// 1m USD at 0.3765 (13 bp off the mid, within tolerance) and bought 100k EUR at 0.410 (199 bp off).
async fn fixture(owner: &PgPool) -> axum::Router {
    let app = pool_for(owner, "APP_DATABASE_URL").await;
    seed::run(owner, &app).await.unwrap();
    let today = Utc::now().date_naive();
    for i in 0..5 {
        let d = today - Duration::days(i);
        let eur = if i == 0 { 0.402 } else { 0.400 };
        sqlx::query("INSERT INTO fx_spots (as_of, pair, mid) VALUES ($1, 'EUR/BHD', $2), ($1, 'USD/BHD', 0.376)").bind(d).bind(eur).execute(owner).await.unwrap();
    }
    let mut tx = db::begin_as(&app, seed::user_id("fx1")).await.unwrap();
    for (n, side, ccy1, notional, rate, trade) in [
        (1, "BUY", "EUR", 1_000_000, "0.400", today - Duration::days(1)),
        (2, "SELL", "USD", 1_000_000, "0.3765", today),
        (3, "BUY", "EUR", 100_000, "0.410", today),
    ] {
        sqlx::query(
            "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, trade_date, value_date, counterparty, trader_id, status) \
             SELECT gen_random_uuid(), $1, id, 'FX_SPOT', $2, $3, 'BHD', $4, $5::numeric, $6, $7, 'Citibank N.A.', $8, 'BOOKED' FROM desks WHERE code = 'FX'",
        )
        .bind(format!("FX-{n:06}"))
        .bind(side)
        .bind(ccy1)
        .bind(notional)
        .bind(rate)
        .bind(trade)
        .bind(today + Duration::days(1))
        .bind(seed::user_id("fx1"))
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    server::app(AppState::new(app, Config::default()))
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn the_book_is_valued_positioned_and_checked_against_the_market(owner: PgPool) {
    let router = fixture(&owner).await;
    let fx = token(&router, "fx1").await;
    let r: FxRiskResult = json(call(&router, Method::GET, "/risk/fx", &fx, None).await).await;
    // Yesterday's euros: 1m x (0.402 - 0.400) = 2,000. Today's dollars: +500 on day one. Today's
    // off-market euros: 100k x (0.402 - 0.410) = -800.
    assert!((r.pnl.today - 1_700.0).abs() < 1e-6, "{:?}", r.pnl);
    assert!((r.pnl.new_deals - -300.0).abs() < 1e-6, "{:?}", r.pnl);
    assert!((r.pnl.market_move - 2_000.0).abs() < 1e-6, "{:?}", r.pnl);
    // Long 1.1m EUR (442,200 BHD), short 1m USD (376,000 BHD): the shorthand takes the longs.
    assert!((r.nop_total - 442_200.0).abs() < 1e-6, "{}", r.nop_total);
    assert!((r.nop_floating - 442_200.0).abs() < 1e-6 && (r.nop_pegged - 376_000.0).abs() < 1e-6);
    let eur = r.positions.iter().find(|p| p.ccy == "EUR").unwrap();
    assert!((eur.amount - 1_100_000.0).abs() < 1e-6);
    assert_eq!(r.off_market.len(), 1);
    assert_eq!(r.off_market[0].deal_ref, "FX-000003");
    assert!((r.off_market[0].off_market_bp.unwrap() - 199.0).abs() < 0.1);
    assert_eq!(r.counterparties.len(), 1);
    assert_eq!(r.counterparties[0].name, "CITIBANK N.A.");

    // The money-market desk sees none of the FX desk's deals.
    let mm = token(&router, "mm1").await;
    let r: FxRiskResult = json(call(&router, Method::GET, "/risk/fx", &mm, None).await).await;
    assert!(r.positions.is_empty() && r.nop_total == 0.0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn only_the_head_sets_limits_and_signs_off_breaches(owner: PgPool) {
    let router = fixture(&owner).await;
    let fx = token(&router, "fx1").await;
    let head = token(&router, "head1").await;

    let l: RiskLimitsResponse = json(call(&router, Method::GET, "/risk/limits", &fx, None).await).await;
    assert_eq!(l.active.version, 1);
    assert!(!l.can_edit);
    let mut limits = l.limits.clone();
    limits.nop_overnight_total = 400_000.0;
    let body = serde_json::json!({ "limits": limits, "note": "test: tighter overnight limit" });
    assert_eq!(call(&router, Method::POST, "/risk/limits", &fx, Some(body.clone())).await.status(), StatusCode::FORBIDDEN);
    assert_eq!(call(&router, Method::POST, "/risk/limits", &head, Some(body)).await.status(), StatusCode::CREATED);
    let mut bad = limits.clone();
    bad.nop_intraday_total = 1.0;
    let res = call(&router, Method::POST, "/risk/limits", &head, Some(serde_json::json!({ "limits": bad, "note": "bad" }))).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 442,200 against 400,000: breached today, for the dealer too.
    let r: FxRiskResult = json(call(&router, Method::GET, "/risk/fx", &fx, None).await).await;
    assert_eq!(r.limits_version, Some(2));
    assert_eq!(r.limits.iter().find(|u| u.key == "nop.total").unwrap().status, LimitStatus::Breach);

    let today = Utc::now().date_naive();
    let b: Vec<Breach> = json(call(&router, Method::GET, "/risk/breaches?days=1", &head, None).await).await;
    assert!(b.iter().any(|x| x.key == "nop.total" && x.date == today && x.signed_by.is_none()));
    let sign = serde_json::json!({ "date": today, "key": "nop.total", "note": "client flow, squared at 09:00" });
    assert_eq!(call(&router, Method::POST, "/risk/breaches/sign", &fx, Some(sign.clone())).await.status(), StatusCode::FORBIDDEN);
    assert_eq!(call(&router, Method::POST, "/risk/breaches/sign", &head, Some(sign)).await.status(), StatusCode::NO_CONTENT);
    let b: Vec<Breach> = json(call(&router, Method::GET, "/risk/breaches?days=1", &fx, None).await).await;
    let x = b.iter().find(|x| x.key == "nop.total").unwrap();
    assert_eq!(x.note.as_deref(), Some("client flow, squared at 09:00"));
    assert!(x.signed_by.is_some());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn the_alco_pack_takes_commentary_until_the_head_signs_it(owner: PgPool) {
    let router = fixture(&owner).await;
    let alm = token(&router, "alm1").await;
    let head = token(&router, "head1").await;
    let fx = token(&router, "fx1").await;
    let period = Utc::now().date_naive();

    // The pack and the new ALM tools are for the ALM desk and the head.
    assert_eq!(call(&router, Method::GET, "/alm/pack", &fx, None).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(call(&router, Method::POST, "/alm/liquidity-gap", &fx, Some(serde_json::json!({}))).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(call(&router, Method::POST, "/alm/ewi", &fx, Some(serde_json::json!({}))).await.status(), StatusCode::NOT_FOUND);

    let note = serde_json::json!({ "period": period, "section": "liquidity", "body": "LCR comfortable; wholesale maturities bunch in the second week." });
    assert_eq!(call(&router, Method::POST, "/alm/pack/note", &alm, Some(note.clone())).await.status(), StatusCode::NO_CONTENT);
    let p: AlcoPack = json(call(&router, Method::GET, "/alm/pack", &alm, None).await).await;
    assert!(p.notes.get("liquidity").is_some_and(|n| n.body.starts_with("LCR comfortable")));
    assert!(!p.can_sign && p.signoff.is_none());

    let sign = serde_json::json!({ "period": period, "note": "Approved at ALCO" });
    assert_eq!(call(&router, Method::POST, "/alm/pack/sign", &alm, Some(sign.clone())).await.status(), StatusCode::FORBIDDEN);
    assert_eq!(call(&router, Method::POST, "/alm/pack/sign", &head, Some(sign)).await.status(), StatusCode::NO_CONTENT);
    let p: AlcoPack = json(call(&router, Method::GET, "/alm/pack", &head, None).await).await;
    assert!(p.can_sign && p.signoff.is_some());
    // Signed: the commentary is final.
    assert_eq!(call(&router, Method::POST, "/alm/pack/note", &alm, Some(note)).await.status(), StatusCode::BAD_REQUEST);
}
