//! Append-only audit writes. A failure here is logged, never surfaced to the caller.

use std::net::{IpAddr, SocketAddr};

use axum::http::HeaderMap;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct AuditCtx {
    pub client_ip: Option<IpAddr>,
    pub request_id: Option<String>,
}

impl AuditCtx {
    pub fn new(peer: Option<SocketAddr>, headers: &HeaderMap) -> Self {
        Self {
            client_ip: peer.map(|p| p.ip()),
            request_id: headers.get("x-request-id").and_then(|v| v.to_str().ok()).map(str::to_owned),
        }
    }
}

pub async fn write(
    pool: &PgPool,
    user_id: Option<Uuid>,
    username: &str,
    action: &str,
    ctx: &AuditCtx,
    detail: Option<serde_json::Value>,
) {
    let res = sqlx::query(
        "INSERT INTO audit_log (user_id, username, action, client_ip, request_id, detail) \
         VALUES ($1, $2, $3, $4::inet, $5, $6)",
    )
    .bind(user_id)
    .bind(username)
    .bind(action)
    .bind(ctx.client_ip.map(|ip| ip.to_string()))
    .bind(&ctx.request_id)
    .bind(detail)
    .execute(pool)
    .await;
    if let Err(e) = res {
        tracing::warn!(error = %e, action, "audit write failed");
    }
}
