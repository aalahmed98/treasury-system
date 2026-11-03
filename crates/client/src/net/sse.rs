//! Server-sent events loop with reconnect. Each `deal` event becomes a FetchDeal command;
//! `resync` and every reconnect become a passive Refresh. A 401 means the session is over.

use std::sync::Arc;
use std::time::Duration;

use domain::ServerEvent;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use tokio::sync::mpsc::UnboundedSender;

use super::api::Api;
use super::{Command, Emitter, NetEvent, StreamState};

pub async fn run(api: Arc<Api>, emit: Emitter, cmds: UnboundedSender<Command>) {
    let client = api.stream_client();
    let url = format!("{}/events", api.base());
    let Some(token) = api.token().map(str::to_owned) else { return };
    let mut backoff = Duration::from_secs(1);
    let mut first = true;

    loop {
        emit.send(NetEvent::Stream(StreamState::Connecting));
        match client.get(&url).bearer_auth(&token).send().await {
            Ok(res) if res.status().is_success() => {
                emit.send(NetEvent::Stream(StreamState::Connected));
                backoff = Duration::from_secs(1);
                if !first {
                    // Anything pushed while we were away is lost; reload.
                    let _ = cmds.send(Command::Refresh { passive: true });
                }
                first = false;

                let mut stream = res.bytes_stream().eventsource();
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(ev) => match ev.event.as_str() {
                            "deal" => match serde_json::from_str::<ServerEvent>(&ev.data) {
                                Ok(ServerEvent::Deal { id, .. }) => {
                                    let _ = cmds.send(Command::FetchDeal(id));
                                }
                                Ok(ServerEvent::Resync) => {
                                    let _ = cmds.send(Command::Refresh { passive: true });
                                }
                                Ok(ServerEvent::Break { .. }) => {} // breaks are handled by the "break" event name (M6)
                                Err(e) => tracing::warn!(error = %e, data = %ev.data, "bad deal event"),
                            },
                            "break" => match serde_json::from_str::<ServerEvent>(&ev.data) {
                                Ok(ServerEvent::Break { id, .. }) => {
                                    let _ = cmds.send(Command::FetchBreak(id));
                                }
                                Ok(_) => {}
                                Err(e) => tracing::warn!(error = %e, data = %ev.data, "bad break event"),
                            },
                            "resync" => {
                                let _ = cmds.send(Command::Refresh { passive: true });
                            }
                            other => tracing::debug!(event = other, "ignored event"),
                        },
                        Err(e) => {
                            tracing::warn!(error = %e, "event stream broke");
                            break;
                        }
                    }
                }
                // The server closes the stream when the session ends; the reconnect below finds out.
            }
            Ok(res) if res.status() == reqwest::StatusCode::UNAUTHORIZED => {
                emit.send(NetEvent::Stream(StreamState::Disconnected));
                emit.send(NetEvent::SessionExpired("your session has ended; please log in again".into()));
                return;
            }
            Ok(res) => emit.send(NetEvent::Error(format!("event stream: HTTP {}", res.status()))),
            Err(e) => emit.send(NetEvent::Error(format!("event stream: {e}"))),
        }

        emit.send(NetEvent::Stream(StreamState::Disconnected));
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}
