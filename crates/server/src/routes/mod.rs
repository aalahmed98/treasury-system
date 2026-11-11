pub mod activity;
pub mod alm;
pub mod alm_ext;
pub mod alm_scenarios;
pub mod audit;
pub mod auth;
pub mod deals;
pub mod events;
pub mod ftp;
pub mod liquidity;
pub mod market;
pub mod pmu;
pub mod pmu_book;
pub mod positions;
pub mod recon;
pub mod risk;

use axum::extract::{DefaultBodyLimit, Request};
use axum::http::header::{self, HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::{Redirect, Response};
use axum::routing::{get, patch, post};
use axum::{Router, middleware};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::auth::require_auth;
use crate::state::AppState;

/// The pages may load scripts, styles, fonts and images from this server only, call this server
/// only, and never sit inside another site's frame.
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'; object-src 'none'";

/// Every response, pages and API alike, carries the browser hardening headers. HSTS goes out only
/// when the server itself speaks TLS or sits behind a terminator that says so.
async fn security_headers(req: Request, next: Next, tls: bool) -> Response {
    let forwarded_https = req.headers().get("x-forwarded-proto").and_then(|v| v.to_str().ok()) == Some("https");
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(HeaderName::from_static("permissions-policy"), HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=(), usb=()"));
    h.insert(HeaderName::from_static("cross-origin-opener-policy"), HeaderValue::from_static("same-origin"));
    h.insert(HeaderName::from_static("cross-origin-resource-policy"), HeaderValue::from_static("same-origin"));
    if tls || forwarded_https {
        h.insert(header::STRICT_TRANSPORT_SECURITY, HeaderValue::from_static("max-age=31536000; includeSubDomains"));
    }
    res
}

pub fn router(state: AppState) -> Router {
    let tls = state.config.tls.is_some();
    let protected = Router::new()
        .route("/auth/logout", post(auth::logout))
        .route("/me", get(auth::me))
        .route("/deals", get(deals::list).post(deals::create))
        .route("/deals/{id}", get(deals::get_one).patch(liquidity::amend))
        .route("/deals/{id}/cancel", post(liquidity::cancel))
        .route("/deals/{id}/history", get(liquidity::history))
        .route("/deals/{id}/provenance", get(recon::provenance))
        .route("/ladder", get(liquidity::ladder))
        .route("/ladder/flows", get(liquidity::flows))
        .route("/ladder/movements", get(liquidity::movements))
        .route("/breaks", get(recon::list_breaks))
        .route("/breaks/{id}", get(recon::get_break))
        .route("/breaks/{id}/ack", post(recon::ack_break))
        .route("/breaks/{id}/resolve", post(recon::resolve_break))
        .route("/feeds", get(recon::feeds))
        .route("/positions", get(positions::list))
        .route("/alm/params", get(alm::params).post(alm::save_params))
        .route("/alm/snapshots", get(alm::snapshots))
        .route("/alm/positions", get(alm::positions))
        .route("/alm/overview", get(alm::overview))
        .route("/alm/trend", get(alm::trend))
        .route("/alm/coverage", post(alm::coverage))
        .route("/alm/lcr", post(alm::lcr))
        .route("/alm/nsfr", post(alm::nsfr))
        .route("/alm/gap", post(alm::gap))
        .route("/alm/nii", post(alm::nii))
        .route("/alm/eve", post(alm::eve))
        .route("/alm/stress", post(alm::stress))
        .route("/alm/attribution", post(alm::attribution))
        .route("/alm/funding", post(alm::funding))
        .route("/alm/liquidity-gap", post(alm_ext::liquidity_gap))
        .route("/alm/ewi", post(alm_ext::ewi))
        .route("/alm/scenarios", get(alm_scenarios::list).post(alm_scenarios::save))
        .route("/alm/scenarios/{id}", axum::routing::delete(alm_scenarios::delete))
        .route("/alm/scenarios/{id}/share", post(alm_scenarios::share))
        .route("/alm/scenarios/{id}/pack", post(alm_scenarios::pack))
        .route("/alm/compare", post(alm_scenarios::compare))
        .route("/alm/pack", get(alm_ext::pack))
        .route("/alm/pack/note", post(alm_ext::pack_note))
        .route("/alm/pack/sign", post(alm_ext::pack_sign))
        .route("/risk/fx", get(risk::fx_risk))
        .route("/risk/fx/deals", get(risk::fx_deals))
        .route("/risk/fx/history", get(risk::fx_history))
        .route("/risk/limits", get(risk::limits).post(risk::save_limits))
        .route("/risk/breaches", get(risk::breaches))
        .route("/risk/breaches/sign", post(risk::sign_breach))
        .route("/market/rates", get(market::rates))
        .route("/ftp", get(ftp::current))
        .route("/ftp/history", get(ftp::history))
        .route("/ftp/template.xlsx", get(ftp::template))
        .route("/ftp/preview", post(ftp::preview).layer(DefaultBodyLimit::max(ftp::MAX_WORKBOOK + 1024)))
        .route("/ftp/publish", post(ftp::publish))
        .route("/ftp/log", get(ftp::log))
        .route("/ftp/upload/{id}/file", get(ftp::file))
        .route("/pmu/overview", get(pmu::overview))
        .route("/pmu/positions", get(pmu::positions))
        .route("/pmu/income", get(pmu::income))
        .route("/pmu/forecast", get(pmu::forecast))
        .route("/pmu/risk", get(pmu::risk))
        .route("/pmu/funding", get(pmu::funding))
        .route("/pmu/bond/{isin}", get(pmu::bond))
        .route("/pmu/bonds", get(pmu_book::search).post(pmu_book::add_bond))
        .route("/pmu/ticket/preview", post(pmu_book::preview))
        .route("/pmu/buy", post(pmu_book::buy))
        .route("/pmu/sell", post(pmu_book::sell))
        .route("/pmu/repo/preview", post(pmu_book::repo_preview))
        .route("/pmu/repo", post(pmu_book::repo_book))
        .route("/pmu/repo/{id}/close", post(pmu_book::repo_close))
        .route("/pmu/repo/{id}/roll", post(pmu_book::repo_roll))
        .route("/pmu/repo/{id}", patch(pmu_book::repo_kondor))
        .route("/pmu/lot/{id}", patch(pmu_book::lot_kondor))
        .route("/pmu/sale/{sale}", patch(pmu_book::sale_kondor))
        .route("/activity", get(activity::activity))
        .route("/attention", get(activity::attention))
        .route("/events", get(events::stream))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    // The web client: built pages under `/app`, with every deep link falling back to the page so
    // the router inside it can take over. Nothing here is behind `require_auth`; the pages carry
    // no data and fetch everything through the API with the user's session.
    let web_dist = state.config.web_dist.clone();
    let web = ServeDir::new(&web_dist).fallback(ServeFile::new(web_dist.join("index.html")));

    // Later `.layer` calls are outermost: request ids are assigned first, traced, then echoed back.
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/", get(|| async { Redirect::temporary("/app/") }))
        .route("/auth/login", post(auth::login))
        .nest_service("/app", web)
        .merge(protected)
        .layer(middleware::from_fn(move |req: Request, next: Next| security_headers(req, next, tls)))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(TraceLayer::new_for_http().make_span_with(|req: &Request| {
            let request_id = req.headers().get("x-request-id").and_then(|v| v.to_str().ok()).unwrap_or("-");
            // Path only: query strings never reach the logs.
            tracing::info_span!("http", method = %req.method(), path = %req.uri().path(), request_id)
        }))
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .with_state(state)
}
