use domain::ServerEvent;
use sqlx::PgPool;
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub events: broadcast::Sender<ServerEvent>,
    pub session_ttl: chrono::Duration,
}
