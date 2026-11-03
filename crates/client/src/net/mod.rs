//! Network side of the client. Runs a tokio runtime on its own thread; the UI thread never blocks.
//! Commands flow UI -> net over a tokio channel, events flow net -> UI over a std channel, and
//! every event is followed by `request_repaint` so the window wakes up.

pub mod api;
pub mod sse;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use domain::{Deal, LoginResponse, Position};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;
use uuid::Uuid;

use api::Api;

#[derive(Debug)]
pub enum Command {
    Login { server: String, username: String, password: String },
    Logout,
    /// Full reload of deals and positions.
    Refresh,
    /// Fetch one deal (after an SSE notification) through the entitlement-checked read path.
    FetchDeal(Uuid),
    RefreshPositions,
}

#[derive(Debug)]
pub enum NetEvent {
    LoginOk(LoginResponse),
    LoginFailed(String),
    Deals { deals: Vec<Deal>, as_of: DateTime<Utc> },
    DealUpdated(Deal),
    Positions(Vec<Position>),
    Stream(StreamState),
    Error(String),
    LoggedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamState {
    Disconnected,
    Connecting,
    Connected,
}

pub struct NetHandle {
    pub commands: UnboundedSender<Command>,
    pub events: std::sync::mpsc::Receiver<NetEvent>,
}

impl NetHandle {
    pub fn send(&self, cmd: Command) {
        if self.commands.send(cmd).is_err() {
            tracing::error!("network thread is gone");
        }
    }
}

#[derive(Clone)]
pub struct Emitter {
    tx: std::sync::mpsc::Sender<NetEvent>,
    ctx: egui::Context,
}

impl Emitter {
    pub fn send(&self, ev: NetEvent) {
        let _ = self.tx.send(ev);
        self.ctx.request_repaint();
    }
}

pub fn spawn(ctx: egui::Context) -> NetHandle {
    let (cmd_tx, cmd_rx) = unbounded_channel();
    let (ev_tx, ev_rx) = std::sync::mpsc::channel();
    let emit = Emitter { tx: ev_tx, ctx };
    std::thread::Builder::new()
        .name("net".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(run(cmd_rx, emit));
        })
        .expect("spawn net thread");
    NetHandle { commands: cmd_tx, events: ev_rx }
}

struct Session {
    api: Arc<Api>,
    sse: JoinHandle<()>,
}

async fn run(mut cmds: UnboundedReceiver<Command>, emit: Emitter) {
    let (self_tx, mut self_rx) = unbounded_channel::<Command>();
    let mut session: Option<Session> = None;

    loop {
        // Commands come from the UI or from the SSE task (which re-enters through self_tx).
        let cmd = tokio::select! {
            c = cmds.recv() => match c { Some(c) => c, None => break },
            c = self_rx.recv() => match c { Some(c) => c, None => break },
        };

        match cmd {
            Command::Login { server, username, password } => {
                if let Some(s) = session.take() {
                    s.sse.abort();
                }
                let api = Api::new(&server);
                match api.login(&username, &password).await {
                    Ok((api, resp)) => {
                        let api = Arc::new(api);
                        emit.send(NetEvent::LoginOk(resp));
                        let sse = tokio::spawn(sse::run(api.clone(), emit.clone(), self_tx.clone()));
                        session = Some(Session { api: api.clone(), sse });
                        load_all(api, emit.clone());
                    }
                    Err(e) => emit.send(NetEvent::LoginFailed(e.to_string())),
                }
            }
            Command::Logout => {
                if let Some(s) = session.take() {
                    s.sse.abort();
                    if let Err(e) = s.api.logout().await {
                        tracing::warn!(error = %e, "logout");
                    }
                }
                emit.send(NetEvent::LoggedOut);
            }
            Command::Refresh => {
                if let Some(s) = &session {
                    load_all(s.api.clone(), emit.clone());
                }
            }
            Command::RefreshPositions => {
                if let Some(s) = &session {
                    let (api, emit) = (s.api.clone(), emit.clone());
                    tokio::spawn(async move {
                        match api.positions().await {
                            Ok(p) => emit.send(NetEvent::Positions(p)),
                            Err(e) => emit.send(NetEvent::Error(format!("positions: {e}"))),
                        }
                    });
                }
            }
            Command::FetchDeal(id) => {
                if let Some(s) = &session {
                    let (api, emit, tx) = (s.api.clone(), emit.clone(), self_tx.clone());
                    tokio::spawn(async move {
                        match api.deal(id).await {
                            Ok(d) => {
                                emit.send(NetEvent::DealUpdated(d));
                                let _ = tx.send(Command::RefreshPositions);
                            }
                            // 404 here means "not entitled or gone"; nothing to show.
                            Err(e) => tracing::debug!(error = %e, %id, "deal fetch skipped"),
                        }
                    });
                }
            }
        }
    }
}

fn load_all(api: Arc<Api>, emit: Emitter) {
    tokio::spawn(async move {
        let (deals, positions) = tokio::join!(api.deals(None), api.positions());
        match deals {
            Ok(r) => emit.send(NetEvent::Deals { deals: r.deals, as_of: r.as_of }),
            Err(e) => emit.send(NetEvent::Error(format!("deals: {e}"))),
        }
        match positions {
            Ok(p) => emit.send(NetEvent::Positions(p)),
            Err(e) => emit.send(NetEvent::Error(format!("positions: {e}"))),
        }
    });
}
