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

async fn post(router: &axum::Router, uri: &str, tok: &str, body: serde_json::Value) -> Response<Body> {
    router.clone().oneshot(req(Method::POST, uri, Some(tok), Some(body.to_string()))).await.unwrap()
}

async fn patch(router: &axum::Router, uri: &str, tok: &str, body: serde_json::Value) -> Response<Body> {
    router.clone().oneshot(req(Method::PATCH, uri, Some(tok), Some(body.to_string()))).await.unwrap()
}

async fn error_of(res: Response<Body>) -> String {
    let v: serde_json::Value = json(res).await;
    v["error"].as_str().unwrap_or_default().to_string()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn pmu_buys_sells_and_repos_and_nobody_else_may(owner: PgPool) {
    use domain::{BondSearchHit, Booked, RepoPreview, TicketPreview};
    let router = fixture(&owner).await;
    let pmu = token(&router, "pmu1").await;
    let today = Utc::now().date_naive();
    let isin = "XSSYN0000011";

    // Search by ISIN: the bond, held 10m in AFS USD, 6m of it pledged.
    let hits: Vec<BondSearchHit> = json(get(&router, "/pmu/bonds?q=syn00000", &pmu).await).await;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].holdings, vec![("AFS USD".to_string(), 10_000_000.0, 6_000_000.0, 4_000_000.0)]);

    // Buy 2m at 99.5 settling today: the preview prices it and warns there is no FTP curve.
    let buy = serde_json::json!({ "side": "BUY", "isin": isin, "book": "AFS USD", "face": 2_000_000.0, "clean_price": 99.5, "trade_date": today, "settle_date": today });
    let pv: TicketPreview = json(post(&router, "/pmu/ticket/preview", &pmu, buy.clone()).await).await;
    assert!(pv.blocking.is_empty(), "{:?}", pv.blocking);
    assert!(pv.quote.ytm > 5.0 && pv.quote.accrued > 0.0 && pv.quote.dv01_bhd < 0.0, "{:?}", pv.quote);
    assert!(close(pv.quote.consideration, (99.5 * 20_000.0) + pv.quote.accrued, 1e-6));
    assert!(pv.warnings.iter().any(|w| w.contains("no FTP curve")), "{:?}", pv.warnings);
    let res = post(&router, "/pmu/buy", &pmu, buy).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let lot: Booked = json(res).await;
    let p: PmuPositions = json(get(&router, "/pmu/positions", &pmu).await).await;
    let new_lot = p.rows.iter().find(|r| r.lot_id.map(|x| x.to_string()) == Some(lot.id.clone())).expect("the new lot");
    assert!(new_lot.provisional);
    assert!(p.rows[0].provisional, "the book shows it holds a provisional lot");
    // Attaching its Kondor id makes it final.
    assert_eq!(patch(&router, &format!("/pmu/lot/{}", lot.id), &pmu, serde_json::json!({ "kondor_id": "KND0001234" })).await.status(), StatusCode::OK);
    let p: PmuPositions = json(get(&router, "/pmu/positions", &pmu).await).await;
    assert!(!p.rows.iter().any(|r| r.provisional));

    // Sell 5m settling in two days: oldest first, 4m from the first lot (6m pledged) and 1m from the new.
    let settle = today + Duration::days(2);
    let sell = serde_json::json!({ "side": "SELL", "isin": isin, "face": 5_000_000.0, "clean_price": 100.25, "trade_date": today, "settle_date": settle });
    let pv: TicketPreview = json(post(&router, "/pmu/ticket/preview", &pmu, sell.clone()).await).await;
    assert!(pv.blocking.is_empty(), "{:?}", pv.blocking);
    assert_eq!(pv.legs.iter().map(|l| l.face).collect::<Vec<_>>(), vec![4_000_000.0, 1_000_000.0]);
    assert!(pv.legs[0].realised > 0.0, "sold above book value: {:?}", pv.legs[0]);
    assert_eq!(post(&router, "/pmu/sell", &pmu, sell).await.status(), StatusCode::CREATED);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM lot_sales").fetch_one(&owner).await.unwrap();
    assert_eq!(n, 2);
    // Nothing of the first lot is free any more until the repo ends.
    let first = uuid::Uuid::from_u128(1);
    let more = serde_json::json!({ "side": "SELL", "isin": isin, "face": 1_000_000.0, "clean_price": 100.25, "trade_date": today, "settle_date": settle, "lots": [{ "lot_id": first, "face": 1_000_000.0 }] });
    let res = post(&router, "/pmu/sell", &pmu, more).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert!(error_of(res).await.contains("free"));

    // A repo on what is left of the new lot (1m), then closed early.
    let repo = serde_json::json!({ "direction": "REPO", "counterparty": "JPMorgan Chase", "ccy": "USD", "start": settle, "end": settle + Duration::days(30), "rate": 4.6, "haircut_pct": 5.0,
        "collateral": [{ "lot_id": lot.id, "face": 1_000_000.0 }] });
    let pv: RepoPreview = json(post(&router, "/pmu/repo/preview", &pmu, repo.clone()).await).await;
    assert!(pv.blocking.is_empty(), "{:?}", pv.blocking);
    assert!(close(pv.cash, pv.collateral_value * 0.95, 1e-6) && pv.cash > 900_000.0, "{pv:?}");
    let res = post(&router, "/pmu/repo", &pmu, repo.clone()).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let r: Booked = json(res).await;
    // The same face cannot be pledged twice.
    let res = post(&router, "/pmu/repo", &pmu, repo).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let res = post(&router, &format!("/pmu/repo/{}/close", r.id), &pmu, serde_json::json!({ "on": settle + Duration::days(10) })).await;
    assert_eq!(res.status(), StatusCode::OK);
    // The fixture's repo rolls at its end for another month.
    let old = uuid::Uuid::from_u128(3);
    let res = post(&router, &format!("/pmu/repo/{old}/roll"), &pmu, serde_json::json!({ "on": today + Duration::days(5), "end": today + Duration::days(35), "rate": 4.7 })).await;
    assert_eq!(res.status(), StatusCode::CREATED, "{}", error_of(res).await);
    let rolled: (i64, Option<uuid::Uuid>) = sqlx::query_as("SELECT count(*), max(rolled_from::text)::uuid FROM repos WHERE rolled_from IS NOT NULL").fetch_one(&owner).await.unwrap();
    assert_eq!(rolled, (1, Some(old)));
    // A reverse repo: 3m lent against 3.2m of the counterparty's bond.
    let rev = serde_json::json!({ "direction": "REVERSE", "counterparty": "Citibank N.A.", "ccy": "USD", "start": today, "end": today + Duration::days(14), "rate": 4.4, "cash": 3_000_000.0,
        "collateral_isin": "US0378331005", "collateral_face": 3_200_000.0, "collateral_note": "their collateral" });
    assert_eq!(post(&router, "/pmu/repo", &pmu, rev).await.status(), StatusCode::CREATED);
    let f: PmuFunding = json(get(&router, "/pmu/funding", &pmu).await).await;
    assert!(close(f.summary.reverse_bhd, 3_000_000.0 * SPOT, 1e-6) && f.summary.reverse_active == 1, "{:?}", f.summary);
    assert!(f.repos.iter().any(|r| r.direction == "REVERSE" && r.provisional));

    // Off the market: with a price of 100 on file, buying at 95 needs a confirmation.
    sqlx::query("INSERT INTO bond_prices (as_of, isin, clean_price, ytm) VALUES ($1, $2, 100, 5.0)").bind(today).bind(isin).execute(&owner).await.unwrap();
    let cheap = serde_json::json!({ "side": "BUY", "isin": isin, "book": "AFS USD", "face": 1_000_000.0, "clean_price": 95.0 });
    let res = post(&router, "/pmu/buy", &pmu, cheap.clone()).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert!(error_of(res).await.contains("off the market"));
    let mut ok = cheap.clone();
    ok["confirm_off_market"] = serde_json::json!(true);
    assert_eq!(post(&router, "/pmu/buy", &pmu, ok).await.status(), StatusCode::CREATED);

    // A new bond: the ISIN must check out, and it can then be bought.
    let nb = |isin: &str| serde_json::json!({ "isin": isin, "name": "Test corporate 5.25% 2031", "issuer": "Test corporate", "issuer_type": "CORPORATE", "ccy": "USD",
        "coupon": 5.25, "coupon_freq": 2, "day_count": "30/360", "issue_date": "2024-03-01", "maturity": "2031-03-01", "rate_type": "FIXED" });
    let res = post(&router, "/pmu/bonds", &pmu, nb("US0378331006")).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert!(error_of(res).await.contains("not a valid ISIN"));
    assert_eq!(post(&router, "/pmu/bonds", &pmu, nb("US0378331005")).await.status(), StatusCode::CREATED);
    assert_eq!(post(&router, "/pmu/bonds", &pmu, nb("US0378331005")).await.status(), StatusCode::BAD_REQUEST);
    let res = post(&router, "/pmu/buy", &pmu, serde_json::json!({ "side": "BUY", "isin": "US0378331005", "book": "AFS USD", "face": 1_000_000.0, "ytm": 5.4 })).await;
    assert_eq!(res.status(), StatusCode::CREATED, "{}", error_of(res).await);

    // ALM and the head read the portfolio but may not book.
    for user in ["alm1", "head1"] {
        let t = token(&router, user).await;
        let res = post(&router, "/pmu/buy", &t, serde_json::json!({ "side": "BUY", "isin": isin, "book": "AFS USD", "face": 1_000_000.0, "clean_price": 100.0 })).await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{user}");
    }
    // Every booking was audited.
    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action LIKE 'pmu_%'").fetch_one(&owner).await.unwrap();
    assert!(audits >= 9, "{audits}");
}
