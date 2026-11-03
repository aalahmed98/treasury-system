use axum::extract::State;
use axum::{Extension, Json};
use domain::{LoginRequest, LoginResponse, MeResponse};
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::{AuthUser, load_identity, password, session};
use crate::error::ApiError;
use crate::state::AppState;

pub async fn login(State(state): State<AppState>, Json(req): Json<LoginRequest>) -> Result<Json<LoginResponse>, ApiError> {
    let username = req.username.trim().to_lowercase();
    let row: Option<(Uuid, String, bool)> =
        sqlx::query_as("SELECT id, password_hash, is_active FROM users WHERE username = $1")
            .bind(&username)
            .fetch_optional(&state.pool)
            .await?;

    let (user_id, ok) = match &row {
        Some((id, hash, active)) => (Some(*id), password::verify(&req.password, hash) && *active),
        None => {
            password::verify(&req.password, password::dummy_hash());
            (None, false)
        }
    };

    if !ok {
        audit(&state.pool, user_id, &username, "login_failed").await;
        return Err(ApiError::Unauthorized);
    }
    let user_id = user_id.expect("checked above");

    let sess = session::create(&state.pool, user_id, state.session_ttl).await?;
    let (user, desks) = load_identity(&state.pool, user_id).await?;
    audit(&state.pool, Some(user_id), &username, "login_ok").await;

    Ok(Json(LoginResponse { token: sess.token, expires_at: sess.expires_at, user, desks }))
}

pub async fn logout(State(state): State<AppState>, Extension(auth): Extension<AuthUser>) -> Result<(), ApiError> {
    session::revoke(&state.pool, &auth.token).await?;
    audit(&state.pool, Some(auth.user.id), &auth.user.username, "logout").await;
    Ok(())
}

pub async fn me(Extension(auth): Extension<AuthUser>) -> Json<MeResponse> {
    Json(MeResponse { user: auth.user, desks: auth.desks })
}

/// Best-effort audit write. A failure here is logged, never surfaced to the caller.
async fn audit(pool: &PgPool, user_id: Option<Uuid>, username: &str, action: &str) {
    let res = sqlx::query("INSERT INTO audit_log (user_id, username, action) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(username)
        .bind(action)
        .execute(pool)
        .await;
    if let Err(e) = res {
        tracing::warn!(error = %e, action, "audit write failed");
    }
}
