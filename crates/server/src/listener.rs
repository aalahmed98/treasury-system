//! Bridges Postgres NOTIFY on `deal_events` into the in-process broadcast channel.

use std::time::Duration;

use domain::{DealOp, ServerEvent};
use serde::Deserialize;
use sqlx::PgPool;
use sqlx::postgres::PgListener;
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Deserialize)]
struct DealPayload {
    op: DealOp,
    id: Uuid,
    desk_id: i16,
    version: i32,
}

#[derive(Deserialize)]
struct AlmPayload {
    #[serde(default)]
    version: Option<i32>,
    #[serde(default)]
    snapshot: Option<chrono::NaiveDate>,
    #[serde(default)]
    ftp: Option<chrono::NaiveDate>,
    #[serde(default)]
    pmu: bool,
}

#[derive(Deserialize)]
struct BreakPayload {
    id: i64,
    desk_id: i16,
    deal_id: Option<Uuid>,
    status: domain::BreakStatus,
}

fn parse(channel: &str, payload: &str) -> Result<ServerEvent, serde_json::Error> {
    match channel {
        "break_events" => {
            let p: BreakPayload = serde_json::from_str(payload)?;
            Ok(ServerEvent::Break { id: p.id, desk_id: p.desk_id, deal_id: p.deal_id, status: p.status })
        }
        "alm_events" => {
            let p: AlmPayload = serde_json::from_str(payload)?;
            Ok(ServerEvent::Alm { params_version: p.version, snapshot: p.snapshot, ftp: p.ftp, pmu: p.pmu })
        }
        _ => {
            let p: DealPayload = serde_json::from_str(payload)?;
            Ok(ServerEvent::Deal { op: p.op, id: p.id, desk_id: p.desk_id, version: p.version })
        }
    }
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
        if let Err(e) = listener.listen_all(["deal_events", "break_events", "alm_events"]).await {
            tracing::warn!(error = %e, "LISTEN failed; retrying");
            tokio::time::sleep(backoff).await;
            continue;
        }
        backoff = Duration::from_secs(1);
        tracing::info!("listening on deal_events, break_events and alm_events");

        loop {
            match listener.try_recv().await {
                Ok(Some(n)) => match parse(n.channel(), n.payload()) {
                    Ok(ev) => {
                        let _ = tx.send(ev);
                    }
                    Err(e) => tracing::warn!(error = %e, channel = n.channel(), payload = n.payload(), "bad notify payload"),
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
