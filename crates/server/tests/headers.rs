//! The browser hardening headers travel on every response: the pages, the API and the health check.

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use server::config::Config;
use server::{AppState, seed};
use sqlx::PgPool;
use tower::ServiceExt;

fn req(method: Method, uri: &str) -> Request<Body> {
    Request::builder().method(method).uri(uri).extension(ConnectInfo("127.0.0.1:40000".parse::<SocketAddr>().unwrap())).body(Body::empty()).unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn every_response_carries_the_hardening_headers(owner: PgPool) {
    seed::run(&owner, &owner).await.unwrap();
    let router = server::app(AppState::new(owner.clone(), Config::default()));

    for uri in ["/healthz", "/deals", "/app/", "/app/alm/lcr"] {
        let res = router.clone().oneshot(req(Method::GET, uri)).await.unwrap();
        assert!(res.status() == StatusCode::OK || res.status() == StatusCode::UNAUTHORIZED || res.status() == StatusCode::NOT_FOUND, "{uri}: {}", res.status());
        let h = res.headers();
        let csp = h.get(header::CONTENT_SECURITY_POLICY).and_then(|v| v.to_str().ok()).unwrap_or_default();
        assert!(csp.contains("default-src 'self'") && csp.contains("frame-ancestors 'none'"), "{uri}: csp {csp}");
        assert_eq!(h.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(), "nosniff", "{uri}");
        assert_eq!(h.get(header::X_FRAME_OPTIONS).unwrap(), "DENY", "{uri}");
        assert_eq!(h.get(header::REFERRER_POLICY).unwrap(), "no-referrer", "{uri}");
        assert!(h.contains_key("permissions-policy"), "{uri}");
        // Plain HTTP on a loopback dev server: no HSTS, which would otherwise pin the browser.
        assert!(!h.contains_key(header::STRICT_TRANSPORT_SECURITY), "{uri}");
    }

    // Behind a TLS terminator that says so, HSTS is added.
    let res = router
        .clone()
        .oneshot(Request::builder().uri("/healthz").extension(ConnectInfo("127.0.0.1:40000".parse::<SocketAddr>().unwrap())).header("x-forwarded-proto", "https").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert!(res.headers().contains_key(header::STRICT_TRANSPORT_SECURITY));
}
