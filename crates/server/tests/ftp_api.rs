//! The FTP curves over HTTP: ALM drops a workbook, previews it, publishes it; every desk reads the
//! curves in force; publishing again supersedes; nobody else may publish.

use std::net::SocketAddr;
use std::str::FromStr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, Response, StatusCode, header};
use chrono::Utc;
use domain::{FtpChange, FtpCurrent, FtpHistory, FtpPreview, FtpPublication, LoginResponse};
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

fn req(method: Method, uri: &str, token: Option<&str>, content_type: &str, body: Body) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .extension(ConnectInfo("127.0.0.1:40000".parse::<SocketAddr>().unwrap()))
        .header(header::CONTENT_TYPE, content_type);
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    b.body(body).unwrap()
}

async fn bytes(res: Response<Body>) -> Vec<u8> {
    res.into_body().collect().await.unwrap().to_bytes().to_vec()
}

async fn json<T: serde::de::DeserializeOwned>(res: Response<Body>) -> T {
    let b = bytes(res).await;
    serde_json::from_slice(&b).unwrap_or_else(|e| panic!("json body: {e}: {}", String::from_utf8_lossy(&b)))
}

async fn token(router: &axum::Router, user: &str) -> String {
    let body = serde_json::json!({ "username": user, "password": seed::DEV_PASSWORD }).to_string();
    let res = router.clone().oneshot(req(Method::POST, "/auth/login", None, "application/json", Body::from(body))).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    json::<LoginResponse>(res).await.token
}

async fn get(router: &axum::Router, uri: &str, tok: &str) -> Response<Body> {
    router.clone().oneshot(req(Method::GET, uri, Some(tok), "application/json", Body::empty())).await.unwrap()
}

async fn post_json(router: &axum::Router, uri: &str, tok: &str, body: serde_json::Value) -> Response<Body> {
    router.clone().oneshot(req(Method::POST, uri, Some(tok), "application/json", Body::from(body.to_string()))).await.unwrap()
}

async fn post_file(router: &axum::Router, uri: &str, tok: &str, file: Vec<u8>) -> Response<Body> {
    router.clone().oneshot(req(Method::POST, uri, Some(tok), "application/octet-stream", Body::from(file))).await.unwrap()
}

/// A workbook in ALM's wide layout: BHD, USD, AED and SAR at ON, 1Y and 5Y, `bump` added to BHD 5Y.
fn workbook(bump: f64) -> Vec<u8> {
    let mut wb = rust_xlsxwriter::Workbook::new();
    let ws = wb.add_worksheet();
    for (c, h) in ["Tenor", "BHD", "USD", "AED", "SAR"].iter().enumerate() {
        ws.write_string(0, c as u16, *h).unwrap();
    }
    let rows = [("ON", [5.10, 4.35, 4.45, 5.25]), ("1Y", [5.95, 4.40, 4.70, 5.65]), ("5Y", [6.60 + bump, 4.90, 5.20, 6.10])];
    for (r, (t, rates)) in rows.iter().enumerate() {
        ws.write_string(1 + r as u32, 0, *t).unwrap();
        for (c, v) in rates.iter().enumerate() {
            ws.write_number(1 + r as u32, 1 + c as u16, *v).unwrap();
        }
    }
    wb.save_to_buffer().unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn alm_publishes_the_workbook_and_every_desk_reads_it(owner: PgPool) {
    let app = pool_for(&owner, "APP_DATABASE_URL").await;
    seed::run(&owner, &app).await.unwrap();
    let router = server::app(AppState::new(app.clone(), Config::default()));
    let alm = token(&router, "alm1").await;
    let fx = token(&router, "fx1").await;
    let today = Utc::now().date_naive();

    // Nothing published yet: the dealer sees that, not an error.
    let c: FtpCurrent = json(get(&router, "/ftp", &fx).await).await;
    assert!(c.current.is_none() && c.stale && !c.can_publish);

    // The template downloads as a workbook even before anything is published.
    let res = get(&router, "/ftp/template.xlsx", &alm).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers()[header::CONTENT_TYPE].to_str().unwrap().contains("spreadsheetml"));
    assert!(bytes(res).await.starts_with(b"PK"));

    // Preview: four curves, nothing stored, and the dealer may not preview.
    let p: FtpPreview = json(post_file(&router, "/ftp/preview", &alm, workbook(0.0)).await).await;
    assert!(p.errors.is_empty(), "{:?}", p.errors);
    assert_eq!(p.curves.iter().map(|c| c.ccy.as_str()).collect::<Vec<_>>(), ["AED", "BHD", "SAR", "USD"]);
    assert_eq!(p.as_of, today);
    assert!(p.compared_with.is_none() && p.changes.is_empty());
    assert_eq!(post_file(&router, "/ftp/preview", &fx, workbook(0.0)).await.status(), StatusCode::FORBIDDEN);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM ftp_publications").fetch_one(&owner).await.unwrap();
    assert_eq!(n, 0);

    // Publishing needs a note; then it is in force for everyone.
    let body = |note: &str, p: &FtpPreview| serde_json::json!({ "as_of": p.as_of, "curves": p.curves, "file_name": "FTP 05-Nov.xlsx", "file_sha256": p.file_sha256, "note": note });
    assert_eq!(post_json(&router, "/ftp/publish", &alm, body("  ", &p)).await.status(), StatusCode::BAD_REQUEST);
    assert_eq!(post_json(&router, "/ftp/publish", &fx, body("mine", &p)).await.status(), StatusCode::FORBIDDEN);
    let res = post_json(&router, "/ftp/publish", &alm, body("Morning curves", &p)).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let first: FtpPublication = json(res).await;
    assert_eq!(first.published_by.as_deref(), Some("ALM Analyst"));
    let c: FtpCurrent = json(get(&router, "/ftp", &fx).await).await;
    assert!(!c.stale && c.age_days == Some(0));
    let cur = c.current.unwrap();
    assert_eq!(cur.curves.len(), 4);
    let bhd = cur.curves.iter().find(|c| c.ccy == "BHD").unwrap();
    assert_eq!(bhd.points.iter().map(|p| (p.tenor.as_str(), p.tenor_days, p.rate)).collect::<Vec<_>>(), [("ON", 1, 5.10), ("1Y", 365, 5.95), ("5Y", 1825, 6.60)]);

    // A corrected workbook the same day: the preview compares with the eve (none here), the
    // publication supersedes the morning's, and both are kept.
    let p2: FtpPreview = json(post_file(&router, "/ftp/preview", &alm, workbook(0.40)).await).await;
    let res = post_json(&router, "/ftp/publish", &alm, body("Corrected BHD 5Y", &p2)).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let c: FtpCurrent = json(get(&router, "/ftp", &fx).await).await;
    let cur = c.current.unwrap();
    assert_eq!(cur.superseded, 1);
    assert_eq!(cur.note.as_deref(), Some("Corrected BHD 5Y"));
    assert_eq!(c.recent.len(), 2);
    assert_eq!(c.recent.iter().filter(|r| r.is_current).count(), 1);
    let h: FtpHistory = json(get(&router, "/ftp/history?ccy=bhd&tenor=5Y", &fx).await).await;
    assert_eq!(h.points.len(), 1);
    assert!((h.points[0].rate - 7.0).abs() < 1e-9);

    // Tomorrow's preview of the same file measures the moves against today.
    let changes = server::ftp_sheet::changes(&cur.curves, &p.curves);
    assert!(changes.iter().any(|c| matches!(c, FtpChange::Moved { ccy, tenor, .. } if ccy == "BHD" && tenor == "5Y")));

    // The audit trail has both, and the app role cannot write the tables directly.
    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'ftp_publish'").fetch_one(&owner).await.unwrap();
    assert_eq!(audits, 2);
    let mut tx = db::begin_as(&app, seed::user_id("alm1")).await.unwrap();
    let denied = sqlx::query("INSERT INTO ftp_publications (as_of, note) VALUES (now()::date, 'x')").execute(&mut *tx).await;
    assert!(denied.is_err());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn the_function_itself_refuses_a_dealer_and_an_empty_publication(owner: PgPool) {
    let app = pool_for(&owner, "APP_DATABASE_URL").await;
    seed::run(&owner, &app).await.unwrap();
    let pts = serde_json::json!([{ "ccy": "BHD", "tenor": "1Y", "tenor_days": 365, "rate": 5.9 }]);
    let mut tx = db::begin_as(&app, seed::user_id("fx1")).await.unwrap();
    let e = sqlx::query("SELECT ftp_publish(now()::date, $1, NULL, NULL, 'x')").bind(&pts).execute(&mut *tx).await.unwrap_err();
    assert!(e.to_string().contains("only ALM"), "{e}");
    drop(tx);
    let mut tx = db::begin_as(&app, seed::user_id("alm1")).await.unwrap();
    let e = sqlx::query("SELECT ftp_publish(now()::date, '[]'::jsonb, NULL, NULL, 'x')").execute(&mut *tx).await.unwrap_err();
    assert!(e.to_string().contains("no curve points"), "{e}");
    drop(tx);
    // The PMU desk reads the curves and may not publish them.
    let mut tx = db::begin_as(&app, seed::user_id("pmu1")).await.unwrap();
    let sees: bool = sqlx::query_scalar("SELECT can_see_pmu()").fetch_one(&mut *tx).await.unwrap();
    assert!(sees);
    let e = sqlx::query("SELECT ftp_publish(now()::date, $1, NULL, NULL, 'x')").bind(&pts).execute(&mut *tx).await.unwrap_err();
    assert!(e.to_string().contains("only ALM"), "{e}");
}
