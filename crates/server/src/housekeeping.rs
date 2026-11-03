//! Periodic cleanup of expired or idle sessions and stale login-throttle rows.

use std::time::Duration;

use sqlx::PgPool;

pub async fn run(pool: PgPool, idle: chrono::Duration) {
    let mut tick = tokio::time::interval(Duration::from_secs(600));
    loop {
        tick.tick().await;
        match sqlx::query_as::<_, (i64, i64)>("SELECT sessions_removed, throttle_removed FROM auth_sweep(make_interval(secs => $1))")
            .bind(idle.num_seconds() as f64)
            .fetch_one(&pool)
            .await
        {
            Ok((s, t)) if s + t > 0 => tracing::info!(sessions = s, throttle_rows = t, "housekeeping swept"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "housekeeping sweep failed"),
        }
    }
}
