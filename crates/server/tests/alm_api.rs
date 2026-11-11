//! The ALM endpoints over HTTP: scoping, the saved parameters, snapshots, and the LCR against the
//! pure crate's hand-worked case.

use std::net::SocketAddr;
use std::str::FromStr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, Response, StatusCode, header};
use chrono::Utc;
use domain::{ActivityRow, AlmOverview, AlmParams, AlmParamsResponse, AlmSnapshot, AttentionItem, AttentionSeverity, AttentionTarget, AttributionResult, BalanceSheetMode, EveResult, GapResult, LcrResult, LoginResponse, MarketResponse, NiiResult, PositionsSummaryRow, StressResult, TrendResponse};
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
    sqlx::query("INSERT INTO market_rates (as_of, ccy, index_code, tenor, tenor_days, rate) VALUES ($1, 'BHD', 'BHIBOR', 'ON', 1, 5.0), ($1, 'BHD', 'BHIBOR', '1Y', 365, 5.5)")
        .bind(today)
        .execute(owner)
        .await
        .unwrap();
    sqlx::query("INSERT INTO fx_spots (as_of, pair, mid) VALUES ($1, 'USD/BHD', 0.376)").bind(today).execute(owner).await.unwrap();
    server::app(AppState::new(app, Config::default()))
}

async fn post<T: serde::de::DeserializeOwned>(router: &axum::Router, tok: &str, uri: &str, body: &str) -> (StatusCode, Option<T>) {
    let res = router.clone().oneshot(req(Method::POST, uri, Some(tok), Some(body.into()))).await.unwrap();
    let status = res.status();
    if status.is_success() { (status, Some(json(res).await)) } else { (status, None) }
}

async fn get<T: serde::de::DeserializeOwned>(router: &axum::Router, tok: &str, uri: &str) -> (StatusCode, Option<T>) {
    let res = router.clone().oneshot(req(Method::GET, uri, Some(tok), None)).await.unwrap();
    let status = res.status();
    if status.is_success() { (status, Some(json(res).await)) } else { (status, None) }
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

#[sqlx::test(migrations = "../../db/migrations")]
async fn every_tool_answers_for_alm_users_only(owner: PgPool) {
    let router = fixture(&owner).await;
    let fx1 = token(&router, "fx1").await;
    let alm1 = token(&router, "alm1").await;
    for uri in ["/alm/nsfr", "/alm/gap", "/alm/nii", "/alm/eve", "/alm/stress", "/alm/funding"] {
        let (status, _) = post::<serde_json::Value>(&router, &fx1, uri, "{}").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri} for fx1");
        let (status, body) = post::<serde_json::Value>(&router, &alm1, uri, "{}").await;
        assert_eq!(status, StatusCode::OK, "{uri} for alm1");
        assert!(body.is_some());
    }
    let (status, nii) = post::<NiiResult>(&router, &alm1, "/alm/nii", r#"{"scenario":"PARALLEL_UP"}"#).await;
    assert_eq!(status, StatusCode::OK);
    let nii = nii.unwrap();
    assert!(nii.base_nii.is_some(), "a shocked run carries the base");
    assert_eq!(nii.months.len(), 12);
    let (_, stress) = post::<StressResult>(&router, &alm1, "/alm/stress", r#"{"custom_shock_bp": 50}"#).await;
    assert_eq!(stress.unwrap().rate_scenarios.len(), 7);

    // One rate shock, sent the same way to every rates tool: a size and the buckets it moves.
    let aimed = r#"{"scenario":"CUSTOM","custom_shock_bp":100,"custom_shock_buckets":["3M","6M","1Y"]}"#;
    let (_, gap) = post::<GapResult>(&router, &alm1, "/alm/gap", aimed).await;
    let gap = gap.unwrap();
    assert_eq!(gap.shock_label.as_deref(), Some("Custom +100 bp at 3M to 1Y"));
    assert!(gap.nii_effect_12m.is_some());
    let bucket = |l: &str| gap.buckets.iter().find(|b| b.label == l).unwrap();
    assert!((bucket("3M").shock_bp.unwrap() - 100.0).abs() < 1e-6 && bucket("1M").shock_bp.unwrap().abs() < 1e-6, "only the named buckets move");
    let (_, plain) = post::<GapResult>(&router, &alm1, "/alm/gap", "{}").await;
    let plain = plain.unwrap();
    assert_eq!((plain.shock_label.clone(), plain.nii_effect_12m), (None, None), "no shock, nothing estimated");
    assert_eq!(plain.buckets.iter().map(|b| b.gap).collect::<Vec<_>>(), gap.buckets.iter().map(|b| b.gap).collect::<Vec<_>>(), "a shock does not move the balances");
    let (_, eve) = post::<EveResult>(&router, &alm1, "/alm/eve", aimed).await;
    let eve = eve.unwrap();
    assert_eq!(eve.shock_label.as_deref(), Some("Custom +100 bp at 3M to 1Y"));
    assert_eq!(eve.duration.estimate, None, "the duration formula needs a parallel move");
    assert_eq!(eve.by_bucket.len(), 9);
    let (_, par) = post::<EveResult>(&router, &alm1, "/alm/eve", r#"{"scenario":"CUSTOM","custom_shock_bp":100}"#).await;
    assert!(par.unwrap().duration.estimate.is_some());
    let (_, nii) = post::<NiiResult>(&router, &alm1, "/alm/nii", aimed).await;
    let nii = nii.unwrap();
    assert_eq!(nii.shock_label.as_deref(), Some("Custom +100 bp at 3M to 1Y"));
    assert_eq!(nii.modes.iter().map(|m| m.mode).collect::<Vec<_>>(), [BalanceSheetMode::Constant, BalanceSheetMode::RunOff, BalanceSheetMode::Dynamic]);
    assert_eq!(nii.mode, BalanceSheetMode::Constant);
    let (_, stress) = post::<StressResult>(&router, &alm1, "/alm/stress", aimed).await;
    assert_eq!(stress.unwrap().rate_scenarios.last().unwrap().label, "Custom +100 bp at 3M to 1Y");

    // Trial hedges: added for this request only, to the rates tools only, and never cached.
    let hedge = r#"{"scenario":"CUSTOM","custom_shock_bp":100,"hedges":[{"kind":"PAY_FIXED_SWAP","ccy":"BHD","notional":50000000,"tenor_months":60}]}"#;
    let (status, hedged) = post::<EveResult>(&router, &alm1, "/alm/eve", hedge).await;
    assert_eq!(status, StatusCode::OK);
    let (_, plain) = post::<EveResult>(&router, &alm1, "/alm/eve", r#"{"scenario":"CUSTOM","custom_shock_bp":100}"#).await;
    let (hedged, plain) = (hedged.unwrap(), plain.unwrap());
    assert!(hedged.delta > plain.delta, "paying fixed gains when rates rise: {} against {}", hedged.delta, plain.delta);
    let (_, again) = post::<EveResult>(&router, &alm1, "/alm/eve", r#"{"scenario":"CUSTOM","custom_shock_bp":100}"#).await;
    assert_eq!(again.unwrap().delta, plain.delta, "the hedged run left nothing in the cache");
    let (_, g) = post::<GapResult>(&router, &alm1, "/alm/gap", hedge).await;
    let (_, g0) = post::<GapResult>(&router, &alm1, "/alm/gap", r#"{"scenario":"CUSTOM","custom_shock_bp":100}"#).await;
    let (g, g0) = (g.unwrap(), g0.unwrap());
    assert_eq!(g.total_assets, g0.total_assets, "notionals are not assets");
    assert_ne!(g.nii_effect_12m, g0.nii_effect_12m);
    let (_, l) = post::<LcrResult>(&router, &alm1, "/alm/lcr", hedge).await;
    let (_, l0) = post::<LcrResult>(&router, &alm1, "/alm/lcr", "{}").await;
    assert_eq!(l.unwrap().total.ratio, l0.unwrap().total.ratio, "liquidity tools do not see hedges");
    for (uri, body) in [
        ("/alm/nii", r#"{"hedges":[{"kind":"PAY_FIXED_SWAP","ccy":"XXX","notional":1000000,"tenor_months":12}]}"#),
        ("/alm/gap", r#"{"hedges":[{"kind":"FIXED_FUNDING","ccy":"BHD","notional":0,"tenor_months":12}]}"#),
        ("/alm/stress", r#"{"hedges":[{"kind":"FIXED_INVESTMENT","ccy":"BHD","notional":1000000,"tenor_months":0}]}"#),
    ] {
        let (status, _) = post::<serde_json::Value>(&router, &alm1, uri, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri} {body}");
    }
    for bad in [r#"{"scenario":"CUSTOM","custom_shock_bp":100,"custom_shock_buckets":["4M"]}"#, r#"{"scenario":"CUSTOM","custom_shock_bp":5000}"#] {
        let (status, _) = post::<serde_json::Value>(&router, &alm1, "/alm/gap", bad).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }

    let (status, o) = get::<AlmOverview>(&router, &alm1, "/alm/overview").await;
    assert_eq!(status, StatusCode::OK);
    let o = o.unwrap();
    assert!((o.lcr.unwrap() - 222.222222).abs() < 1e-5);
    assert_eq!(o.params_version, 1);
    assert!(o.limits.iter().any(|l| l.metric == "LCR" && !l.breach));
    assert_eq!(o.lcr_trend.len(), 1);
    let (status, _) = get::<AlmOverview>(&router, &fx1, "/alm/overview").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, t) = get::<TrendResponse>(&router, &alm1, "/alm/trend?metric=nsfr&days=30").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(t.unwrap().points.len(), 1);
    let (status, _) = get::<TrendResponse>(&router, &alm1, "/alm/trend?metric=bogus").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, p) = get::<Vec<PositionsSummaryRow>>(&router, &alm1, "/alm/positions").await;
    assert_eq!(status, StatusCode::OK);
    assert!(p.unwrap().len() >= 3);

    // Market data is for every desk.
    let (status, m) = get::<MarketResponse>(&router, &fx1, "/market/rates").await;
    assert_eq!(status, StatusCode::OK);
    let m = m.unwrap();
    assert_eq!(m.curves.len(), 1);
    assert_eq!(m.curves[0].points.len(), 2);
    assert_eq!(m.spots.len(), 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn attribution_compares_months_and_budget(owner: PgPool) {
    let router = fixture(&owner).await;
    let alm1 = token(&router, "alm1").await;
    sqlx::query(
        "INSERT INTO alm_period (period, book, side, ccy, kind, nii, avg_balance) VALUES \
         ('2026-09-01', 'LOANS', 'ASSET', 'BHD', 'ACTUAL', 4.1667, 1000), ('2026-10-01', 'LOANS', 'ASSET', 'BHD', 'ACTUAL', 6.2, 1200), \
         ('2026-09-01', 'DEPOSITS', 'LIABILITY', 'BHD', 'ACTUAL', -1.3333, 800), ('2026-10-01', 'DEPOSITS', 'LIABILITY', 'BHD', 'ACTUAL', -1.7222, 800), \
         ('2026-10-01', 'LOANS', 'ASSET', 'BHD', 'BUDGET', 5.0, 1100), ('2026-10-01', 'DEPOSITS', 'LIABILITY', 'BHD', 'BUDGET', -1.5, 800)",
    )
    .execute(&owner)
    .await
    .unwrap();
    let (status, r) = post::<AttributionResult>(&router, &alm1, "/alm/attribution", r#"{"period":"2026-10-01"}"#).await;
    assert_eq!(status, StatusCode::OK);
    let r = r.unwrap();
    assert_eq!(r.period_a.to_string(), "2026-09-01");
    assert!((r.delta - ((6.2 - 1.72) - (4.17 - 1.33))).abs() < 1e-9, "delta {}", r.delta);
    let sum = r.days_effect + r.pure_volume_effect + r.mix_effect + r.rate_effect + r.residual;
    assert!((sum - r.delta).abs() < 1e-9);
    let (status, b) = post::<AttributionResult>(&router, &alm1, "/alm/attribution", r#"{"period":"2026-10-01","compare_to":"BUDGET"}"#).await;
    assert_eq!(status, StatusCode::OK);
    let b = b.unwrap();
    assert!((b.nii_a - 3.5).abs() < 1e-9, "budget is the starting point");
    assert!((b.nii_b - (6.2 - 1.72)).abs() < 1e-9);
    let (status, _) = post::<AttributionResult>(&router, &alm1, "/alm/attribution", r#"{"period":"2020-01-01"}"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn saving_parameters_shows_in_attention_and_activity(owner: PgPool) {
    let router = fixture(&owner).await;
    let fx1 = token(&router, "fx1").await;
    let alm1 = token(&router, "alm1").await;
    let mut p = AlmParams::default();
    p.lcr.internal_minimum = 500.0;
    let body = serde_json::json!({ "params": p, "note": "stress the floor" }).to_string();
    let (status, _) = post::<serde_json::Value>(&router, &fx1, "/alm/params", &body).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "fx1 cannot save");
    let bad = serde_json::json!({ "params": AlmParams::default(), "note": "" }).to_string();
    let (status, _) = post::<serde_json::Value>(&router, &alm1, "/alm/params", &bad).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a note is required");
    let mut broken = AlmParams::default();
    broken.lcr.outflow_rates.insert("RET_STABLE".into(), 150.0);
    let bad = serde_json::json!({ "params": broken, "note": "x" }).to_string();
    let (status, _) = post::<serde_json::Value>(&router, &alm1, "/alm/params", &bad).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "percentages are checked before the database");
    let (status, v) = post::<domain::AlmParamsVersion>(&router, &alm1, "/alm/params", &body).await;
    assert_eq!(status, StatusCode::CREATED);
    let v = v.unwrap();
    assert_eq!(v.version, 2);
    assert_eq!(v.created_by.as_deref(), Some("ALM Analyst"));

    let (_, p2) = get::<AlmParamsResponse>(&router, &alm1, "/alm/params").await;
    let p2 = p2.unwrap();
    assert_eq!(p2.active.version, 2);
    assert_eq!(p2.params.lcr.internal_minimum, 500.0);
    assert_eq!(p2.history.len(), 2);

    let (status, items) = get::<Vec<AttentionItem>>(&router, &alm1, "/attention").await;
    assert_eq!(status, StatusCode::OK);
    let items = items.unwrap();
    let lcr = items.iter().find(|i| i.title.starts_with("LCR")).expect("an LCR item: {items:?}");
    assert_eq!(lcr.severity, AttentionSeverity::Critical);
    assert_eq!(lcr.target, AttentionTarget::Alm { tool: "lcr".into() });
    assert!(items.iter().any(|i| i.title.contains("parameters changed to v2")));
    assert_eq!(items.first().map(|i| i.severity), Some(AttentionSeverity::Critical), "sorted by severity");
    let (status, fx_items) = get::<Vec<AttentionItem>>(&router, &fx1, "/attention").await;
    assert_eq!(status, StatusCode::OK);
    assert!(fx_items.unwrap().iter().all(|i| !matches!(i.target, AttentionTarget::Alm { .. })), "no ALM items for a trader");

    let (status, acts) = get::<Vec<ActivityRow>>(&router, &alm1, "/activity?limit=10").await;
    assert_eq!(status, StatusCode::OK);
    let acts = acts.unwrap();
    assert!(acts.iter().any(|a| a.kind == "parameters" && a.summary.contains("v2")), "{acts:?}");
    let (status, acts) = get::<Vec<ActivityRow>>(&router, &fx1, "/activity").await;
    assert_eq!(status, StatusCode::OK);
    assert!(acts.unwrap().iter().all(|a| a.kind != "parameters"));

    let (audited,): (i64,) = sqlx::query_as("SELECT count(*) FROM audit_log WHERE action = 'alm_params_save'").fetch_one(&owner).await.unwrap();
    assert_eq!(audited, 1);
}
