use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::{Extension, Json};
use chrono::{DateTime, Utc};
use domain::{LoginRequest, LoginResponse, MeResponse};
use uuid::Uuid;

use crate::auth::{AuthUser, load_identity, password, session};
use crate::error::ApiError;
use crate::routes::audit::{self, AuditCtx};
use crate::state::AppState;

const MAX_PASSWORD_LEN: usize = 256;

pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let ctx = AuditCtx::new(Some(peer), &headers);
    let cfg = &state.config;
    let username = req.username.trim().to_lowercase();

    if let Err(retry_after) = state.login_limiter.check(peer.ip()) {
        audit::write(&state.pool, None, &username, "login_rate_limited", &ctx, None).await;
        return Err(ApiError::TooManyRequests { retry_after });
    }
    if username.is_empty() || username.len() > 64 || req.password.is_empty() || req.password.len() > MAX_PASSWORD_LEN {
        audit::write(&state.pool, None, &username, "login_failed", &ctx, None).await;
        return Err(ApiError::Unauthorized);
    }

    // One row always comes back, whether or not the username exists.
    let (user_id, hash, is_active, locked_until): (Option<Uuid>, Option<String>, Option<bool>, Option<DateTime<Utc>>) =
        sqlx::query_as("SELECT user_id, password_hash, is_active, locked_until FROM auth_lookup($1)")
            .bind(&username)
            .fetch_one(&state.pool)
            .await?;

    if let Some(until) = locked_until.filter(|u| *u > Utc::now()) {
        audit::write(&state.pool, user_id, &username, "login_locked", &ctx, None).await;
        return Err(ApiError::TooManyRequests { retry_after: seconds_until(until) });
    }

    // Always run one argon2 verification so a missing or inactive user costs the same time.
    let ok = match (&hash, is_active) {
        (Some(h), Some(true)) => password::verify(&req.password, h),
        (Some(h), _) => {
            password::verify(&req.password, h);
            false
        }
        (None, _) => {
            password::verify(&req.password, password::dummy_hash());
            false
        }
    };

    if !ok {
        let locked: Option<DateTime<Utc>> =
            sqlx::query_scalar("SELECT auth_login_failed($1, $2, make_interval(secs => $3))")
                .bind(&username)
                .bind(cfg.login_max_failures)
                .bind(cfg.login_lock.num_seconds() as f64)
                .fetch_one(&state.pool)
                .await?;
        let detail = locked.map(|l| serde_json::json!({ "locked_until": l }));
        audit::write(&state.pool, user_id, &username, "login_failed", &ctx, detail).await;
        return Err(match locked {
            Some(until) => ApiError::TooManyRequests { retry_after: seconds_until(until) },
            None => ApiError::Unauthorized,
        });
    }
    let user_id = user_id.expect("a verified password implies a user");

    sqlx::query("SELECT auth_login_succeeded($1)").bind(&username).execute(&state.pool).await?;
    let sess = session::create(&state.pool, user_id, cfg.session_ttl).await?;
    let (user, desks) = load_identity(&state.pool, user_id).await?;
    audit::write(&state.pool, Some(user_id), &username, "login_ok", &ctx, None).await;

    Ok(Json(LoginResponse { token: sess.token, expires_at: sess.expires_at, user, desks }))
}

pub async fn logout(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Extension(auth): Extension<AuthUser>,
) -> Result<(), ApiError> {
    session::revoke(&state.pool, &auth.token_hash).await?;
    // Close this session's open event streams immediately.
    let _ = state.revoked.send(auth.token_hash.clone());
    let ctx = AuditCtx::new(Some(peer), &headers);
    audit::write(&state.pool, Some(auth.user.id), &auth.user.username, "logout", &ctx, None).await;
    Ok(())
}

pub async fn me(Extension(auth): Extension<AuthUser>) -> Json<MeResponse> {
    Json(MeResponse { user: auth.user, desks: auth.desks })
}

fn seconds_until(t: DateTime<Utc>) -> u64 {
    (t - Utc::now()).num_seconds().max(1) as u64
}
