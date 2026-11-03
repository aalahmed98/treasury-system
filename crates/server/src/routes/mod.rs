pub mod auth;
pub mod deals;
pub mod events;
pub mod positions;

use axum::routing::{get, post};
use axum::{Router, middleware};
use tower_http::trace::TraceLayer;

use crate::auth::require_auth;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/auth/logout", post(auth::logout))
        .route("/me", get(auth::me))
        .route("/deals", get(deals::list).post(deals::create))
        .route("/deals/{id}", get(deals::get_one))
        .route("/positions", get(positions::list))
        .route("/events", get(events::stream))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/auth/login", post(auth::login))
        .merge(protected)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
