//! The PMU endpoints over HTTP against a hand-built portfolio: one 5% dollar bond bought at par,
//! half of it swapped, part of it repoed. The figures are worked by hand in the comments.

use std::net::SocketAddr;
use std::str::FromStr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, Response, StatusCode, header};
use chrono::{Duration, NaiveDate, Utc};
use domain::{AttentionItem, AttentionTarget, LoginResponse, PmuFunding, PmuIncome, PmuOverview, PmuPositions, PmuRisk, RowLevel};
use http_body_util::BodyExt;
use server::config::Config;
use server::{AppState, seed};
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

async fn get(router: &axum::Router, uri: &str, tok: &str) -> Response<Body> {
    router.clone().oneshot(req(Method::GET, uri, Some(tok), None)).await.unwrap()
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

const SPOT: f64 = 0.376;

/// 10m of a 5% semi-annual 30/360 dollar bond (15 Jan / 15 Jul, to 2035), bought at par on the
/// 15 Jan 2026 coupon date with FTP locked at 4%. A 5m payer swap hedges it; a 4m repo against 6m
/// face of it matures in five days. The dollar is 0.376 and the dollar curve flat at 4%.
async fn fixture(owner: &PgPool) -> axum::Router {
    let app = pool_for(owner, "APP_DATABASE_URL").await;
    seed::run(owner, &app).await.unwrap();
    let today = Utc::now().date_naive();
    for days in [0, 30, 400] {
        let on = today - Duration::days(days);
        sqlx::query("INSERT INTO fx_spots (as_of, pair, mid) VALUES ($1, 'USD/BHD', $2)").bind(on).bind(SPOT).execute(owner).await.unwrap();
        for (t, td) in [("1M", 30), ("3M", 91), ("1Y", 365), ("5Y", 1825), ("10Y", 3650)] {
            sqlx::query("INSERT INTO market_rates (as_of, ccy, index_code, tenor, tenor_days, rate) VALUES ($1, 'USD', 'SOFR', $2, $3, 4.0)").bind(on).bind(t).bind(td).execute(owner).await.unwrap();
        }
    }
    sqlx::query(
        "INSERT INTO bonds (isin, name, issuer, issuer_type, ccy, coupon, coupon_freq, day_count, issue_date, maturity, rate_type) \
         VALUES ('XSSYN0000011', 'Test sovereign 5% 2035', 'Test', 'SOVEREIGN', 'USD', 5, 2, '30/360', '2025-01-15', '2035-01-15', 'FIXED')",
    )
    .execute(owner)
    .await
    .unwrap();
    let lot = uuid::Uuid::from_u128(1);
    sqlx::query(
        "INSERT INTO bond_lots (id, book, isin, trade_date, settle_date, face, clean_price, kondor_id, eir, ftp_rate) \
         VALUES ($1, 'AFS USD', 'XSSYN0000011', '2026-01-15', '2026-01-15', 10000000, 100, 'KND0000001', 5.0625, 4.0)",
    )
    .bind(lot)
    .execute(owner)
    .await
    .unwrap();
    let swap = uuid::Uuid::from_u128(2);
    sqlx::query(
        "INSERT INTO irs (id, book, ccy, notional, start_date, maturity, pay_fixed, fixed_rate, fixed_freq, fixed_day_count, float_index, float_tenor_months) \
         VALUES ($1, 'AFS USD', 'USD', 5000000, '2026-01-15', '2035-01-15', true, 4.0, 2, '30/360', 'SOFR', 3)",
    )
    .bind(swap)
    .execute(owner)
    .await
    .unwrap();
    sqlx::query("INSERT INTO irs_hedges (irs_id, lot_id, notional) VALUES ($1, $2, 5000000)").bind(swap).bind(lot).execute(owner).await.unwrap();
    let repo = uuid::Uuid::from_u128(3);
    sqlx::query("INSERT INTO repos (id, counterparty, ccy, start_date, end_date, cash, rate, haircut_pct) VALUES ($1, 'Citibank N.A.', 'USD', $2, $3, 4000000, 4.5, 5)")
        .bind(repo)
        .bind(today - Duration::days(25))
        .bind(today + Duration::days(5))
        .execute(owner)
        .await
        .unwrap();
    sqlx::query("INSERT INTO repo_collateral (repo_id, lot_id, face) VALUES ($1, $2, 6000000)").bind(repo).bind(lot).execute(owner).await.unwrap();
    server::app(AppState::new(app, Config::default()))
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn a_hand_built_portfolio_is_accrued_hedged_and_funded(owner: PgPool) {
    let router = fixture(&owner).await;
    let pmu = token(&router, "pmu1").await;

    // Ten days in March: 30/360 accrues the 2.5% half-year coupon by 1/180 a day, so the coupon
    // is 10m x 2.5% x 10/180 = 13,888.89 dollars; FTP is the book value (par, near enough) at 4%
    // over 360: 10m x 4% x 10/360 = 11,111.11. In dinars at 0.376: 5,222.22 and 4,177.78.
    let inc: PmuIncome = json(get(&router, "/pmu/income?from=2026-03-02&to=2026-03-11", &pmu).await).await;
    assert_eq!(inc.daily.len(), 10);
    assert!(close(inc.total.coupon, 13_888.888_9 * SPOT, 0.01), "{:?}", inc.total);
    assert!(close(inc.total.ftp, 11_111.111 * SPOT, 2.0), "{:?}", inc.total);
    // At par the effective rate is the coupon compounded, and amortisation is all but nil.
    assert!(inc.total.amortisation.abs() < 0.02 * inc.total.coupon, "{:?}", inc.total);
    assert!(close(inc.total.carry, inc.total.coupon + inc.total.amortisation - inc.total.ftp, 1e-6));
    // The swap pays 4% fixed on 30/360 and receives the 3M rate (4%) on ACT/360: on 5m that is
    // 5m x 4% x (1/360 - 1/360) a day in March, so nothing, give or take the day counts.
    assert!(inc.total.irs.abs() < 50.0, "{:?}", inc.total);
    // The repo: 4m at 4.5% over 360, but only when the window overlaps it (it does not here).
    assert_eq!(inc.total.repo, 0.0);

    // Overview: 10m of face, all lots with a Kondor id, and the repo due within seven days.
    let o: PmuOverview = json(get(&router, "/pmu/overview", &pmu).await).await;
    assert!(close(o.headline.portfolio_face_bhd, 10_000_000.0 * SPOT, 1e-6));
    assert_eq!(o.headline.lots, 1);
    assert_eq!(o.headline.kondor_pct, 100.0);
    assert!(close(o.repo.due_7d_bhd, 4_000_000.0 * SPOT, 1e-6));
    assert!(close(o.repo.pledged_face_bhd, 6_000_000.0 * SPOT, 1e-6));
    assert!(close(o.repo.available_face_bhd, 4_000_000.0 * SPOT, 1e-6));
    assert_eq!(o.repo.active, 1);
    assert!(close(o.repo.avg_rate, 4.5, 1e-9));
    assert!(o.monthly.len() == 12 && o.monthly.iter().any(|m| m.actual));

    // Positions: book, bond, lot; half the face hedged.
    let p: PmuPositions = json(get(&router, "/pmu/positions", &pmu).await).await;
    assert_eq!(p.rows.iter().map(|r| r.level).collect::<Vec<_>>(), [RowLevel::Book, RowLevel::Bond, RowLevel::Lot]);
    assert!(close(p.rows[2].hedged_face_bhd, 5_000_000.0 * SPOT, 1e-6));
    assert_eq!(p.rows[2].ftp_rate, Some(4.0));
    assert_eq!(p.swaps.len(), 1);

    // Risk: the bond loses when rates rise, the swap gains, half hedged nets about half the risk.
    let r: PmuRisk = json(get(&router, "/pmu/risk", &pmu).await).await;
    assert_eq!(r.hedge_pairs.len(), 1);
    let hp = &r.hedge_pairs[0];
    assert!(hp.bond_dv01 < 0.0 && hp.irs_dv01 > 0.0);
    let share = -hp.irs_dv01 / hp.bond_dv01;
    assert!(share > 0.35 && share < 0.6, "{hp:?}");
    let up100 = r.shocks.iter().find(|s| s.shift_bp == 100.0).unwrap();
    let down100 = r.shocks.iter().find(|s| s.shift_bp == -100.0).unwrap();
    assert!(up100.total < 0.0 && down100.total > 0.0 && down100.total > -up100.total, "convexity: {up100:?} {down100:?}");
    assert!(r.unhedged.is_empty());

    // Funding: the repo in the seven-day bucket; the coupon of 15 Jan in the receipts.
    let f: PmuFunding = json(get(&router, "/pmu/funding", &pmu).await).await;
    assert!(close(f.by_remaining[0].amount, 4_000_000.0 * SPOT, 1e-6));
    let coupons: f64 = f.receipts.iter().map(|m| m.a).sum();
    assert!(close(coupons, 2.0 * 250_000.0 * SPOT, 1e-3), "two coupons of 250k in the next year: {coupons}");

    // The attention list tells the desk about the repo.
    let items: Vec<AttentionItem> = json(get(&router, "/attention", &pmu).await).await;
    assert!(items.iter().any(|i| i.title.starts_with("Repos of") && i.target == AttentionTarget::Pmu { page: "funding".into() }), "{items:?}");

    // Bond detail by ISIN, and a missing one.
    assert_eq!(get(&router, "/pmu/bond/xssyn0000011", &pmu).await.status(), StatusCode::OK);
    assert_eq!(get(&router, "/pmu/bond/XSSYN0000099", &pmu).await.status(), StatusCode::NOT_FOUND);
    // A bad period is refused.
    assert_eq!(get(&router, "/pmu/income?from=2026-03-11&to=2026-03-01", &pmu).await.status(), StatusCode::BAD_REQUEST);
    let _ = d(2026, 1, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn the_dealers_do_not_see_the_portfolio_and_alm_does(owner: PgPool) {
    let router = fixture(&owner).await;
    let fx = token(&router, "fx1").await;
    for path in ["/pmu/overview", "/pmu/positions", "/pmu/risk", "/pmu/funding", "/pmu/forecast"] {
        assert_eq!(get(&router, path, &fx).await.status(), StatusCode::NOT_FOUND, "{path}");
    }
    let alm = token(&router, "alm1").await;
    assert_eq!(get(&router, "/pmu/overview", &alm).await.status(), StatusCode::OK);
    // ALM is reminded that today's FTP curves are not published (on a business day).
    let items: Vec<AttentionItem> = json(get(&router, "/attention", &alm).await).await;
    let business = domain::Calendar::new(Vec::<NaiveDate>::new()).is_business_day(Utc::now().date_naive());
    assert_eq!(items.iter().any(|i| i.target == AttentionTarget::Ftp), business);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn deals_are_booked_on_the_dealing_desks_only(owner: PgPool) {
    let router = fixture(&owner).await;
    let desk = |code: &'static str| {
        let owner = owner.clone();
        async move { sqlx::query_scalar::<_, i16>("SELECT id FROM desks WHERE code = $1").bind(code).fetch_one(&owner).await.unwrap() }
    };
    let today = Utc::now().date_naive();
    let deal = |desk_id: i16, product: &str| {
        serde_json::json!({ "desk_id": desk_id, "product": product, "side": if product.starts_with("FX") { "BUY" } else { "LEND" },
            "ccy1": "USD", "ccy2": if product.starts_with("FX") { serde_json::json!("BHD") } else { serde_json::Value::Null },
            "notional": "1000000", "rate": "0.376", "trade_date": today, "value_date": today + Duration::days(2),
            "maturity_date": if product.starts_with("MM") { serde_json::json!(today + Duration::days(30)) } else { serde_json::Value::Null },
            "counterparty": "Citibank N.A." })
    };
    // pmu1 may book on its desk (for the PMU tickets to come) but not a deal.
    let pmu = token(&router, "pmu1").await;
    let res = router.clone().oneshot(req(Method::POST, "/deals", Some(&pmu), Some(deal(desk("PMU").await, "FX_SPOT").to_string()))).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let body: serde_json::Value = json(res).await;
    assert!(body["error"].as_str().unwrap().contains("FX and money-market desks"), "{body}");
    // The FX desk books FX products only.
    let fx = token(&router, "fx1").await;
    let res = router.clone().oneshot(req(Method::POST, "/deals", Some(&fx), Some(deal(desk("FX").await, "MM_DEPO").to_string()))).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let res = router.clone().oneshot(req(Method::POST, "/deals", Some(&fx), Some(deal(desk("FX").await, "FX_SPOT").to_string()))).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    // And the database refuses it whoever asks: a direct insert on the PMU desk as the owner.
    let e = sqlx::query(
        "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, trade_date, value_date, counterparty, trader_id, status) \
         SELECT gen_random_uuid(), 'PMU-1', id, 'FX_SPOT', 'BUY', 'USD', 'BHD', 1, 0.376, now()::date, now()::date, 'x', $1, 'BOOKED' FROM desks WHERE code = 'PMU'",
    )
    .bind(seed::user_id("pmu1"))
    .execute(&owner)
    .await
    .unwrap_err();
    assert!(e.to_string().contains("not entitled to book deals on the PMU desk"), "{e}");
}
