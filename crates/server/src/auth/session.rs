//! Opaque bearer tokens. The database stores only the SHA-256 of the token, and a session dies
//! at its absolute expiry or after `idle` without an authenticated request, whichever is first.

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

pub struct NewSession {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

pub fn hash_token(raw: &str) -> Vec<u8> {
    Sha256::digest(raw.as_bytes()).to_vec()
}

pub async fn create(pool: &PgPool, user_id: Uuid, ttl: chrono::Duration) -> sqlx::Result<NewSession> {
    let bytes: [u8; 32] = rand::random();
    let token = hex::encode(bytes);
    let expires_at = Utc::now() + ttl;
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, $3)")
        .bind(hash_token(&token))
        .bind(user_id)
        .bind(expires_at)
        .execute(pool)
        .await?;
    Ok(NewSession { token, expires_at })
}

/// Returns the user id for a live token. With `touch`, the access counts as user activity.
pub async fn authenticate(pool: &PgPool, raw: &str, idle: chrono::Duration, touch: bool) -> sqlx::Result<Option<(Uuid, Vec<u8>)>> {
    let hash = hash_token(raw);
    let sql = if touch {
        "UPDATE sessions SET last_seen_at = now() \
         WHERE token_hash = $1 AND expires_at > now() AND last_seen_at > now() - make_interval(secs => $2) \
         RETURNING user_id"
    } else {
        "SELECT user_id FROM sessions \
         WHERE token_hash = $1 AND expires_at > now() AND last_seen_at > now() - make_interval(secs => $2)"
    };
    let user: Option<Uuid> = sqlx::query_scalar(sql).bind(&hash).bind(idle.num_seconds() as f64).fetch_optional(pool).await?;
    Ok(user.map(|u| (u, hash)))
}

/// Is the session still alive? Does not count as activity (used by long-lived streams).
pub async fn is_live(pool: &PgPool, token_hash: &[u8], idle: chrono::Duration) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sessions \
         WHERE token_hash = $1 AND expires_at > now() AND last_seen_at > now() - make_interval(secs => $2))",
    )
    .bind(token_hash)
    .bind(idle.num_seconds() as f64)
    .fetch_one(pool)
    .await
}

pub async fn revoke(pool: &PgPool, token_hash: &[u8]) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1").bind(token_hash).execute(pool).await?;
    Ok(())
}
