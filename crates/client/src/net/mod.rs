//! Network side of the client. Runs a tokio runtime on its own thread; the UI thread never blocks.
//! Commands flow UI -> net over a tokio channel, events flow net -> UI over a std channel, and
//! every event is followed by `request_repaint` so the window wakes up.

pub mod api;
pub mod sse;

use std::fmt;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use domain::{Deal, LoginResponse, Position};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;
use uuid::Uuid;

use api::{Api, is_unauthorized};

pub enum Command {
    Login { username: String, password: String },
    Logout,
    /// Full reload of deals and positions. Passive reloads are machine-initiated (resync) and do
    /// not count as user activity.
    Refresh { passive: bool },
    /// Fetch one deal (after an SSE notification) through the entitlement-checked read path.
    FetchDeal(Uuid),
    RefreshPositions,
    /// The user is present; keep the session's idle timer fresh.
    Heartbeat,
}

impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::Login { username, .. } => {
                f.debug_struct("Login").field("username", username).field("password", &"[redacted]").finish()
            }
            Command::Logout => f.write_str("Logout"),
            Command::Refresh { passive } => f.debug_struct("Refresh").field("passive", passive).finish(),
            Command::FetchDeal(id) => f.debug_tuple("FetchDeal").field(id).finish(),
            Command::RefreshPositions => f.write_str("RefreshPositions"),
            Command::Heartbeat => f.write_str("Heartbeat"),
        }
    }
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
    /// The server no longer accepts this session (logout elsewhere, expiry, idle timeout).
    SessionExpired(String),
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

    /// Route an error from an authenticated call: 401 ends the session, anything else is shown.
    fn fail(&self, what: &str, e: anyhow::Error) {
        if is_unauthorized(&e) {
            self.send(NetEvent::SessionExpired("your session has ended; please log in again".into()));
        } else {
            self.send(NetEvent::Error(format!("{what}: {e}")));
        }
    }
}

pub struct Target {
    pub url: String,
    pub ca: Option<reqwest::Certificate>,
}

pub fn spawn(ctx: egui::Context, target: Target) -> NetHandle {
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
            rt.block_on(run(cmd_rx, emit, target));
        })
        .expect("spawn net thread");
    NetHandle { commands: cmd_tx, events: ev_rx }
}

struct Session {
    api: Arc<Api>,
    sse: JoinHandle<()>,
}

async fn run(mut cmds: UnboundedReceiver<Command>, emit: Emitter, target: Target) {
    let (self_tx, mut self_rx) = unbounded_channel::<Command>();
    let mut session: Option<Session> = None;

    loop {
        // Commands come from the UI or from the SSE task (which re-enters through self_tx).
        let cmd = tokio::select! {
            c = cmds.recv() => match c { Some(c) => c, None => break },
            c = self_rx.recv() => match c { Some(c) => c, None => break },
        };

        match cmd {
            Command::Login { username, password } => {
                if let Some(s) = session.take() {
                    s.sse.abort();
                }
                let api = Api::new(&target.url, target.ca.clone());
                match api.login(&username, &password).await {
                    Ok((api, resp)) => {
                        let api = Arc::new(api);
                        emit.send(NetEvent::LoginOk(resp));
                        let sse = tokio::spawn(sse::run(api.clone(), emit.clone(), self_tx.clone()));
                        session = Some(Session { api: api.clone(), sse });
                        load_all(api, emit.clone(), false);
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
            Command::Refresh { passive } => {
                if let Some(s) = &session {
                    load_all(s.api.clone(), emit.clone(), passive);
                }
            }
            Command::RefreshPositions => {
                if let Some(s) = &session {
                    let (api, emit) = (s.api.clone(), emit.clone());
                    tokio::spawn(async move {
                        match api.positions(true).await {
                            Ok(p) => emit.send(NetEvent::Positions(p)),
                            Err(e) => emit.fail("positions", e),
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
                            Err(e) if is_unauthorized(&e) => emit.fail("deal", e),
                            // 404 here means "not entitled or gone"; nothing to show.
                            Err(e) => tracing::debug!(error = %e, %id, "deal fetch skipped"),
                        }
                    });
                }
            }
            Command::Heartbeat => {
                if let Some(s) = &session {
                    let (api, emit) = (s.api.clone(), emit.clone());
                    tokio::spawn(async move {
                        if let Err(e) = api.me().await {
                            emit.fail("heartbeat", e);
                        }
                    });
                }
            }
        }
    }
}

fn load_all(api: Arc<Api>, emit: Emitter, passive: bool) {
    tokio::spawn(async move {
        let (deals, positions) = tokio::join!(api.deals(None, passive), api.positions(passive));
        match deals {
            Ok(r) => emit.send(NetEvent::Deals { deals: r.deals, as_of: r.as_of }),
            Err(e) => emit.fail("deals", e),
        }
        match positions {
            Ok(p) => emit.send(NetEvent::Positions(p)),
            Err(e) => emit.fail("positions", e),
        }
    });
}
