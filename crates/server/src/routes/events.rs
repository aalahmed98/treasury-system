use std::convert::Infallible;
use std::time::Duration;

use axum::Extension;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use domain::ServerEvent;
use futures::Stream;
use futures::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use crate::auth::AuthUser;
use crate::state::AppState;

/// Server-sent events, filtered to the caller's desks. Only ids travel on this channel; the client
/// fetches each deal through the RLS-checked read path.
pub async fn stream(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let desk_ids = auth.desk_ids();
    let rx = state.events.subscribe();
    tracing::debug!(user = %auth.user.username, "event stream opened");

    let stream = BroadcastStream::new(rx).filter_map(move |item| {
        let ev = match item {
            Ok(ev @ ServerEvent::Deal { desk_id, .. }) => desk_ids.contains(&desk_id).then_some(ev),
            Ok(ServerEvent::Resync) => Some(ServerEvent::Resync),
            Err(BroadcastStreamRecvError::Lagged(_)) => Some(ServerEvent::Resync),
        };
        futures::future::ready(ev.map(|ev| {
            let name = match ev {
                ServerEvent::Deal { .. } => "deal",
                ServerEvent::Resync => "resync",
            };
            let data = serde_json::to_string(&ev).expect("ServerEvent serialises");
            Ok(Event::default().event(name).data(data))
        }))
    });

    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)).text("ping"))
}
