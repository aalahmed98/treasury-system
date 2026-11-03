//! Treasury server: thin HTTP/SSE layer over Postgres. Entitlement decisions are made by
//! Postgres row-level security; this crate only establishes identity per transaction.

pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod listener;
pub mod routes;
pub mod seed;
pub mod state;

pub use state::AppState;

pub fn app(state: AppState) -> axum::Router {
    routes::router(state)
}
