//! Server-sent events, filtered to the caller's desks. Only ids travel on this channel; the client
//! fetches each deal through the RLS-checked read path. The stream is tied to the session: it ends
//! at once on logout and within one check interval of expiry, idle timeout, or an entitlement
//! change (the desk filter is reloaded on every check).

use std::convert::Infallible;
use std::time::Duration;

use axum::Extension;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use domain::ServerEvent;
use futures::Stream;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::auth::{AuthUser, load_identity, session};
use crate::state::AppState;

pub async fn stream(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::channel(64);
    tracing::debug!(user = %auth.user.username, "event stream opened");
    tokio::spawn(pump(state, auth, tx));
    Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)).text("ping"))
}

async fn pump(state: AppState, auth: AuthUser, tx: mpsc::Sender<Result<Event, Infallible>>) {
    let mut events = state.events.subscribe();
    let mut revoked = state.revoked.subscribe();
    let mut desk_ids = auth.desk_ids();
    let mut check = tokio::time::interval(state.config.stream_check);
    check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    check.tick().await; // the first tick fires immediately; the request was just authenticated

    loop {
        let out = tokio::select! {
            _ = tx.closed() => break,
            r = events.recv() => match r {
                Ok(ServerEvent::Resync) | Err(RecvError::Lagged(_)) => Some(ServerEvent::Resync),
                Ok(ev) => match ev.desk_id() {
                    Some(d) if desk_ids.contains(&d) => Some(ev),
                    _ => None,
                },
                Err(RecvError::Closed) => break,
            },
            r = revoked.recv() => match r {
                Ok(hash) if hash == auth.token_hash => {
                    tracing::debug!(user = %auth.user.username, "event stream closed: logged out");
                    break;
                }
                _ => None,
            },
            _ = check.tick() => {
                let live = session::is_live(&state.pool, &auth.token_hash, state.config.session_idle).await.unwrap_or(false);
                if !live {
                    tracing::debug!(user = %auth.user.username, "event stream closed: session no longer valid");
                    break;
                }
                match load_identity(&state.pool, auth.user.id).await {
                    Ok((_, desks)) => desk_ids = desks.into_iter().map(|d| d.desk_id).collect(),
                    Err(_) => break,
                }
                None
            }
        };

        if let Some(ev) = out {
            let name = ev.name();
            let data = serde_json::to_string(&ev).expect("ServerEvent serialises");
            if tx.send(Ok(Event::default().event(name).data(data))).await.is_err() {
                break;
            }
        }
    }
}
