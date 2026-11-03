//! Bridges Postgres NOTIFY on `deal_events` into the in-process broadcast channel.

use std::time::Duration;

use domain::{DealOp, ServerEvent};
use serde::Deserialize;
use sqlx::PgPool;
use sqlx::postgres::PgListener;
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Deserialize)]
struct Payload {
    op: DealOp,
    id: Uuid,
    desk_id: i16,
    version: i32,
}

pub async fn run(pool: PgPool, tx: broadcast::Sender<ServerEvent>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let mut listener = match PgListener::connect_with(&pool).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(error = %e, "listener connect failed; retrying");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
                continue;
            }
        };
        listener.eager_reconnect(false);
        if let Err(e) = listener.listen("deal_events").await {
            tracing::warn!(error = %e, "LISTEN failed; retrying");
            tokio::time::sleep(backoff).await;
            continue;
        }
        backoff = Duration::from_secs(1);
        tracing::info!("listening on deal_events");

        loop {
            match listener.try_recv().await {
                Ok(Some(n)) => match serde_json::from_str::<Payload>(n.payload()) {
                    Ok(p) => {
                        let _ = tx.send(ServerEvent::Deal { op: p.op, id: p.id, desk_id: p.desk_id, version: p.version });
                    }
                    Err(e) => tracing::warn!(error = %e, payload = n.payload(), "bad notify payload"),
                },
                // Connection dropped and came back: anything sent meanwhile is gone.
                Ok(None) => {
                    tracing::warn!("listener reconnected; asking clients to resync");
                    let _ = tx.send(ServerEvent::Resync);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "listener error; reconnecting");
                    let _ = tx.send(ServerEvent::Resync);
                    break;
                }
            }
        }
    }
}
