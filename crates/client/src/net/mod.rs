//! Network side of the client. Runs a tokio runtime on its own thread; the UI thread never blocks.
//! Commands flow UI -> net over a tokio channel, events flow net -> UI over a std channel, and
//! every event is followed by `request_repaint` so the window wakes up.

pub mod api;
pub mod sse;

use std::fmt;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{AmendDeal, BreakView, CashFlow, Deal, FeedsResponse, HistoryEntry, LadderResponse, LoginResponse, NewDeal, NostroMovement, Position, ProvenanceResponse, ResolveRequest};
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
    FetchBreak(i64),
    FetchBreaks { all: bool, passive: bool },
    FetchProvenance(Uuid),
    FetchFeeds,
    AckBreak(i64),
    ResolveBreak { id: i64, req: ResolveRequest },
    FetchLadder { ccy: Option<String>, passive: bool },
    FetchLadderDetail { ccy: String, date: NaiveDate },
    BookDeal(NewDeal),
    AmendDeal { id: Uuid, req: AmendDeal },
    CancelDeal(Uuid),
    FetchHistory(Uuid),
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
            Command::FetchBreak(id) => f.debug_tuple("FetchBreak").field(id).finish(),
            Command::FetchBreaks { all, passive } => f.debug_struct("FetchBreaks").field("all", all).field("passive", passive).finish(),
            Command::FetchProvenance(id) => f.debug_tuple("FetchProvenance").field(id).finish(),
            Command::FetchFeeds => f.write_str("FetchFeeds"),
            Command::AckBreak(id) => f.debug_tuple("AckBreak").field(id).finish(),
            Command::ResolveBreak { id, req } => f.debug_struct("ResolveBreak").field("id", id).field("action", &req.action).finish(),
            Command::FetchLadder { ccy, passive } => f.debug_struct("FetchLadder").field("ccy", ccy).field("passive", passive).finish(),
            Command::FetchLadderDetail { ccy, date } => f.debug_struct("FetchLadderDetail").field("ccy", ccy).field("date", date).finish(),
            Command::BookDeal(d) => f.debug_struct("BookDeal").field("desk_id", &d.desk_id).field("product", &d.product).finish(),
            Command::AmendDeal { id, .. } => f.debug_tuple("AmendDeal").field(id).finish(),
            Command::CancelDeal(id) => f.debug_tuple("CancelDeal").field(id).finish(),
            Command::FetchHistory(id) => f.debug_tuple("FetchHistory").field(id).finish(),
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
    Breaks(Vec<BreakView>),
    BreakUpdated(BreakView),
    Provenance(ProvenanceResponse),
    Feeds(FeedsResponse),
    /// A user action was accepted or refused; shown in the status bar.
    Notice(String),
    Ladder(LadderResponse),
    LadderDetail { ccy: String, date: NaiveDate, flows: Vec<CashFlow>, movements: Vec<NostroMovement> },
    Booked(Deal),
    /// The booking form should show this error and stay open.
    BookingFailed(String),
    History(Uuid, Vec<HistoryEntry>),
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
                        let _ = self_tx.send(Command::FetchBreaks { all: false, passive: false });
                        let _ = self_tx.send(Command::FetchFeeds);
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
                    let _ = self_tx.send(Command::FetchBreaks { all: false, passive });
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
            Command::FetchBreak(id) => {
                if let Some(s) = &session {
                    let (api, emit) = (s.api.clone(), emit.clone());
                    tokio::spawn(async move {
                        match api.break_detail(id).await {
                            Ok(d) => emit.send(NetEvent::BreakUpdated(d.break_)),
                            Err(e) if is_unauthorized(&e) => emit.fail("break", e),
                            Err(e) => tracing::debug!(error = %e, id, "break fetch skipped"),
                        }
                    });
                }
            }
            Command::FetchBreaks { all, passive } => {
                if let Some(s) = &session {
                    let (api, emit) = (s.api.clone(), emit.clone());
                    tokio::spawn(async move {
                        match api.breaks(all, passive).await {
                            Ok(b) => emit.send(NetEvent::Breaks(b)),
                            Err(e) => emit.fail("breaks", e),
                        }
                    });
                }
            }
            Command::FetchProvenance(id) => {
                if let Some(s) = &session {
                    let (api, emit) = (s.api.clone(), emit.clone());
                    tokio::spawn(async move {
                        match api.provenance(id).await {
                            Ok(p) => emit.send(NetEvent::Provenance(p)),
                            Err(e) => emit.fail("provenance", e),
                        }
                    });
                }
            }
            Command::FetchFeeds => {
                if let Some(s) = &session {
                    let (api, emit) = (s.api.clone(), emit.clone());
                    tokio::spawn(async move {
                        match api.feeds().await {
                            Ok(f) => emit.send(NetEvent::Feeds(f)),
                            Err(e) => emit.fail("feeds", e),
                        }
                    });
                }
            }
            Command::AckBreak(id) => {
                if let Some(s) = &session {
                    let (api, emit) = (s.api.clone(), emit.clone());
                    tokio::spawn(async move {
                        match api.ack_break(id).await {
                            Ok(b) => {
                                emit.send(NetEvent::BreakUpdated(b));
                                emit.send(NetEvent::Notice(format!("break #{id} acknowledged")));
                            }
                            Err(e) => emit.send(NetEvent::Notice(format!("acknowledge failed: {e}"))),
                        }
                    });
                }
            }
            Command::FetchLadder { ccy, passive } => {
                if let Some(s) = &session {
                    let (api, emit) = (s.api.clone(), emit.clone());
                    tokio::spawn(async move {
                        match api.ladder(ccy.as_deref(), passive).await {
                            Ok(l) => emit.send(NetEvent::Ladder(l)),
                            Err(e) => emit.fail("ladder", e),
                        }
                    });
                }
            }
            Command::FetchLadderDetail { ccy, date } => {
                if let Some(s) = &session {
                    let (api, emit) = (s.api.clone(), emit.clone());
                    tokio::spawn(async move {
                        let (flows, movements) = tokio::join!(api.flows(&ccy, date), api.movements(&ccy, date));
                        match (flows, movements) {
                            (Ok(flows), Ok(movements)) => emit.send(NetEvent::LadderDetail { ccy, date, flows, movements }),
                            (Err(e), _) | (_, Err(e)) => emit.fail("ladder detail", e),
                        }
                    });
                }
            }
            Command::BookDeal(deal) => {
                if let Some(s) = &session {
                    let (api, emit) = (s.api.clone(), emit.clone());
                    tokio::spawn(async move {
                        match api.book(&deal).await {
                            Ok(d) => {
                                emit.send(NetEvent::Notice(format!("booked {}", d.deal_ref)));
                                emit.send(NetEvent::Booked(d));
                            }
                            Err(e) if is_unauthorized(&e) => emit.fail("booking", e),
                            Err(e) => emit.send(NetEvent::BookingFailed(e.to_string())),
                        }
                    });
                }
            }
            Command::AmendDeal { id, req } => {
                if let Some(s) = &session {
                    let (api, emit) = (s.api.clone(), emit.clone());
                    tokio::spawn(async move {
                        match api.amend(id, &req).await {
                            Ok(d) => {
                                emit.send(NetEvent::Notice(format!("amended {} (v{})", d.deal_ref, d.version)));
                                emit.send(NetEvent::DealUpdated(d));
                            }
                            Err(e) => emit.send(NetEvent::Notice(format!("amend failed: {e}"))),
                        }
                    });
                }
            }
            Command::CancelDeal(id) => {
                if let Some(s) = &session {
                    let (api, emit) = (s.api.clone(), emit.clone());
                    tokio::spawn(async move {
                        match api.cancel(id).await {
                            Ok(d) => {
                                emit.send(NetEvent::Notice(format!("cancelled {}", d.deal_ref)));
                                emit.send(NetEvent::DealUpdated(d));
                            }
                            Err(e) => emit.send(NetEvent::Notice(format!("cancel failed: {e}"))),
                        }
                    });
                }
            }
            Command::FetchHistory(id) => {
                if let Some(s) = &session {
                    let (api, emit) = (s.api.clone(), emit.clone());
                    tokio::spawn(async move {
                        match api.history(id).await {
                            Ok(h) => emit.send(NetEvent::History(id, h)),
                            Err(e) => emit.fail("history", e),
                        }
                    });
                }
            }
            Command::ResolveBreak { id, req } => {
                if let Some(s) = &session {
                    let (api, emit) = (s.api.clone(), emit.clone());
                    tokio::spawn(async move {
                        match api.resolve_break(id, &req).await {
                            Ok(b) => {
                                emit.send(NetEvent::BreakUpdated(b));
                                emit.send(NetEvent::Notice(format!("break #{id}: {} sent to the reconciler", req.action.as_str().to_lowercase())));
                            }
                            Err(e) => emit.send(NetEvent::Notice(format!("resolve failed: {e}"))),
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
