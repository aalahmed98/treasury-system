use std::collections::HashSet;

use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::Response;
use domain::{DeskAccess, UserInfo};
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::session;
use crate::db;
use crate::error::ApiError;
use crate::state::AppState;

/// Requests carrying this header are machine-initiated (event-driven refetches) and do not
/// count as user activity for the idle timeout.
pub const PASSIVE_HEADER: &str = "x-treasury-passive";

/// The authenticated caller, attached to every protected request. Holds the token's hash only;
/// the raw token is not kept after authentication.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user: UserInfo,
    pub desks: Vec<DeskAccess>,
    pub token_hash: Vec<u8>,
}

impl AuthUser {
    pub fn desk_ids(&self) -> HashSet<i16> {
        self.desks.iter().map(|d| d.desk_id).collect()
    }

    pub fn desk(&self, desk_id: i16) -> Option<&DeskAccess> {
        self.desks.iter().find(|d| d.desk_id == desk_id)
    }
}

pub async fn require_auth(State(state): State<AppState>, mut req: Request, next: Next) -> Result<Response, ApiError> {
    let raw = bearer_token(&req).ok_or(ApiError::Unauthorized)?.to_owned();
    let passive = req.headers().get(PASSIVE_HEADER).is_some();
    let (user_id, token_hash) = session::authenticate(&state.pool, &raw, state.config.session_idle, !passive)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    let (user, desks) = load_identity(&state.pool, user_id).await?;
    req.extensions_mut().insert(AuthUser { user, desks, token_hash });
    Ok(next.run(req).await)
}

fn bearer_token(req: &Request) -> Option<&str> {
    req.headers()
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// Load the user's profile and entitlement rows under their own identity
/// (user_desk_access is itself RLS-protected).
pub async fn load_identity(pool: &PgPool, user_id: Uuid) -> Result<(UserInfo, Vec<DeskAccess>), ApiError> {
    let mut tx = db::begin_as(pool, user_id).await?;

    let user: Option<(Uuid, String, String, i16)> = sqlx::query_as(
        "SELECT id, username, display_name, home_desk_id FROM users WHERE id = $1 AND is_active",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (id, username, display_name, home_desk_id) = user.ok_or(ApiError::Unauthorized)?;

    let desks: Vec<(i16, String, String, bool, bool)> = sqlx::query_as(
        "SELECT a.desk_id, k.code, k.name, a.can_book, a.can_resolve \
         FROM user_desk_access a JOIN desks k ON k.id = a.desk_id \
         WHERE a.user_id = $1 ORDER BY k.id",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    let desks = desks
        .into_iter()
        .map(|(desk_id, code, name, can_book, can_resolve)| DeskAccess { desk_id, code, name, can_book, can_resolve })
        .collect();
    Ok((UserInfo { id, username, display_name, home_desk_id }, desks))
}
