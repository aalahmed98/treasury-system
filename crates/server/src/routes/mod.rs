pub mod audit;
pub mod auth;
pub mod deals;
pub mod events;
pub mod positions;
pub mod recon;

use axum::extract::Request;
use axum::routing::{get, post};
use axum::{Router, middleware};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;

use crate::auth::require_auth;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/auth/logout", post(auth::logout))
        .route("/me", get(auth::me))
        .route("/deals", get(deals::list).post(deals::create))
        .route("/deals/{id}", get(deals::get_one))
        .route("/deals/{id}/provenance", get(recon::provenance))
        .route("/breaks", get(recon::list_breaks))
        .route("/breaks/{id}", get(recon::get_break))
        .route("/breaks/{id}/ack", post(recon::ack_break))
        .route("/breaks/{id}/resolve", post(recon::resolve_break))
        .route("/feeds", get(recon::feeds))
        .route("/positions", get(positions::list))
        .route("/events", get(events::stream))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    // Later `.layer` calls are outermost: request ids are assigned first, traced, then echoed back.
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/auth/login", post(auth::login))
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
