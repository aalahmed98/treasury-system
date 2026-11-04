pub mod activity;
pub mod alm;
pub mod audit;
pub mod auth;
pub mod deals;
pub mod events;
pub mod liquidity;
pub mod market;
pub mod positions;
pub mod recon;

use axum::extract::Request;
use axum::response::Redirect;
use axum::routing::{get, post};
use axum::{Router, middleware};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::auth::require_auth;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
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
        .route("/alm/lcr", post(alm::lcr))
        .route("/alm/nsfr", post(alm::nsfr))
        .route("/alm/gap", post(alm::gap))
        .route("/alm/nii", post(alm::nii))
        .route("/alm/eve", post(alm::eve))
        .route("/alm/stress", post(alm::stress))
        .route("/alm/attribution", post(alm::attribution))
        .route("/alm/funding", post(alm::funding))
        .route("/market/rates", get(market::rates))
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
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(TraceLayer::new_for_http().make_span_with(|req: &Request| {
            let request_id = req.headers().get("x-request-id").and_then(|v| v.to_str().ok()).unwrap_or("-");
            // Path only: query strings never reach the logs.
            tracing::info_span!("http", method = %req.method(), path = %req.uri().path(), request_id)
        }))
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .with_state(state)
}
