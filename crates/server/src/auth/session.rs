//! Opaque bearer tokens. The database stores only the SHA-256 of the token.

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

/// Returns the user id for a live token and records the access. Expired tokens yield `None`.
pub async fn authenticate(pool: &PgPool, raw: &str) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar(
        "UPDATE sessions SET last_seen_at = now() \
         WHERE token_hash = $1 AND expires_at > now() \
         RETURNING user_id",
    )
    .bind(hash_token(raw))
    .fetch_optional(pool)
    .await
}

pub async fn revoke(pool: &PgPool, raw: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(hash_token(raw))
        .execute(pool)
        .await?;
    Ok(())
}
