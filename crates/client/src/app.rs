use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use chrono::NaiveDate;
use domain::{CashFlow, DeskAccess, FeedsResponse, HistoryEntry, LadderResponse, NostroMovement, Position, ProvenanceResponse, UserInfo};
use crate::ui::ticket::{AmendForm, TicketForm};
use uuid::Uuid;

use crate::ServerTarget;
use crate::net::{self, Command, NetEvent, NetHandle, StreamState};
use crate::state::{BreakStore, DealStore};
use crate::ui;

/// How often a present user's session is refreshed on the server.
const HEARTBEAT_EVERY: Duration = Duration::from_secs(60);
/// Feed freshness is polled; it changes constantly and is not worth a push channel.
const FEEDS_EVERY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Login,
    Main,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Blotter,
    Breaks,
    Ladder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Summary,
    Sources,
    History,
}

pub struct App {
    pub net: NetHandle,
    pub screen: Screen,
    pub tab: Tab,

    // server target (fixed at start-up)
    pub server_url: String,
    pub server_problem: Option<String>,

    // login form
    pub username: String,
    pub password: String,
    pub login_error: Option<String>,
    /// Why the user is back on the login screen, e.g. the session ended.
    pub notice: Option<String>,
    pub logging_in: bool,

    // session
    pub user: Option<UserInfo>,
    pub desks: Vec<DeskAccess>,
    pub expires_at: Option<DateTime<Utc>>,
    last_input: Instant,
    last_heartbeat: Instant,
    last_feeds: Instant,

    // data
    pub store: DealStore,
    pub breaks: BreakStore,
    pub positions: Vec<Position>,
    pub feeds: Option<FeedsResponse>,
    pub provenance: Option<ProvenanceResponse>,
    pub stream: StreamState,
    pub last_event: Option<DateTime<Utc>>,
    pub last_error: Option<(DateTime<Utc>, String)>,
    pub status_notice: Option<(Instant, String)>,

    // view state
    pub desk_filter: Option<i16>,
    pub search: String,
    pub only_breaks: bool,
    pub only_provisional: bool,
    pub selected: Option<Uuid>,
    pub detail_tab: DetailTab,
    pub filtered: Vec<usize>,
    filter_key: (Option<i16>, String, bool, bool, u64),
    pub break_search: String,
    pub show_resolved: bool,
    pub selected_break: Option<i64>,
    pub resolve_note: String,
    pub link_ref: String,
    pub break_filtered: Vec<usize>,
    break_filter_key: (String, u64),

    // ladder
    pub ladder: Option<LadderResponse>,
    pub ladder_ccy: Option<String>,
    pub ladder_date: Option<NaiveDate>,
    pub ladder_flows: Vec<CashFlow>,
    pub ladder_movements: Vec<NostroMovement>,
    pub ladder_dirty: bool,
    last_ladder: Instant,

    // booking and amendments
    pub ticket: Option<TicketForm>,
    pub amend: Option<AmendForm>,
    pub history: Option<(Uuid, Vec<HistoryEntry>)>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, target: ServerTarget) -> Self {
        cc.egui_ctx.all_styles_mut(|style| style.spacing.item_spacing = egui::vec2(6.0, 4.0));
        let net = net::spawn(cc.egui_ctx.clone(), net::Target { url: target.url.clone(), ca: target.ca });
        Self {
            net,
            screen: Screen::Login,
            tab: Tab::Blotter,
            server_url: target.url,
            server_problem: target.problem,
            username: String::new(),
            password: String::new(),
            login_error: None,
            notice: None,
            logging_in: false,
            user: None,
            desks: Vec::new(),
            expires_at: None,
            last_input: Instant::now(),
            last_heartbeat: Instant::now(),
            last_feeds: Instant::now(),
            store: DealStore::default(),
            breaks: BreakStore::default(),
            positions: Vec::new(),
            feeds: None,
            provenance: None,
            stream: StreamState::Disconnected,
            last_event: None,
            last_error: None,
            status_notice: None,
            desk_filter: None,
            search: String::new(),
            only_breaks: false,
            only_provisional: false,
            selected: None,
            detail_tab: DetailTab::Summary,
            filtered: Vec::new(),
            filter_key: (None, String::new(), false, false, u64::MAX),
            break_search: String::new(),
            show_resolved: false,
            selected_break: None,
            resolve_note: String::new(),
            link_ref: String::new(),
            break_filtered: Vec::new(),
            break_filter_key: (String::new(), u64::MAX),
            ladder: None,
            ladder_ccy: None,
            ladder_date: None,
            ladder_flows: Vec::new(),
            ladder_movements: Vec::new(),
            ladder_dirty: true,
            last_ladder: Instant::now() - Duration::from_secs(60),
            ticket: None,
            amend: None,
            history: None,
        }
    }

    pub fn can_book(&self, desk_id: i16) -> bool {
        self.desks.iter().any(|d| d.desk_id == desk_id && d.can_book)
    }

    pub fn bookable_desks(&self) -> Vec<DeskAccess> {
        self.desks.iter().filter(|d| d.can_book).cloned().collect()
    }

    pub fn open_tab(&mut self, tab: Tab) {
        self.tab = tab;
        if tab == Tab::Ladder {
            self.ladder_dirty = true;
        }
    }

    pub fn open_detail_tab(&mut self, tab: DetailTab) {
        self.detail_tab = tab;
        if tab == DetailTab::History {
            if let Some(id) = self.selected {
                if self.history.as_ref().is_none_or(|(h, _)| *h != id) {
                    self.net.send(Command::FetchHistory(id));
                }
            }
        }
    }

    pub fn select_ladder_ccy(&mut self, ccy: String) {
        self.ladder_ccy = Some(ccy);
        self.ladder_date = None;
        self.ladder_flows.clear();
        self.ladder_movements.clear();
        self.ladder_dirty = true;
    }

    pub fn select_ladder_date(&mut self, date: Option<NaiveDate>) {
        self.ladder_date = date;
        self.ladder_flows.clear();
        self.ladder_movements.clear();
        if let (Some(d), Some(l)) = (date, &self.ladder) {
            self.net.send(Command::FetchLadderDetail { ccy: l.ccy.clone(), date: d });
        }
    }

    pub fn can_login(&self) -> bool {
        self.server_problem.is_none() && !self.logging_in && !self.username.trim().is_empty() && !self.password.is_empty()
    }

    pub fn can_resolve(&self, desk_id: i16) -> bool {
        self.desks.iter().any(|d| d.desk_id == desk_id && d.can_resolve)
    }

    pub fn submit_login(&mut self) {
        if !self.can_login() {
            return;
        }
        self.logging_in = true;
        self.login_error = None;
        self.notice = None;
        self.net.send(Command::Login {
            username: self.username.trim().to_string(),
            password: std::mem::take(&mut self.password),
        });
    }

    pub fn logout(&mut self) {
        self.net.send(Command::Logout);
        self.reset_session();
    }

    /// Select a deal (toggle) and load its provenance.
    pub fn select_deal(&mut self, id: Option<Uuid>) {
        if self.selected == id {
            return;
        }
        self.selected = id;
        self.provenance = None;
        self.history = None;
        self.amend = None;
        if let Some(id) = id {
            self.net.send(Command::FetchProvenance(id));
            if self.detail_tab == DetailTab::History {
                self.net.send(Command::FetchHistory(id));
            }
        }
    }

    pub fn select_break(&mut self, id: Option<i64>) {
        self.selected_break = id;
        self.resolve_note.clear();
        self.link_ref.clear();
        if let Some(deal) = id.and_then(|b| self.breaks.get(b)).and_then(|b| b.view.deal_id) {
            self.select_deal(Some(deal));
            self.detail_tab = DetailTab::Sources;
        }
    }

    fn reset_session(&mut self) {
        self.screen = Screen::Login;
        self.tab = Tab::Blotter;
        self.user = None;
        self.desks.clear();
        self.expires_at = None;
        self.store.clear();
        self.breaks.clear();
        self.positions.clear();
        self.feeds = None;
        self.provenance = None;
        self.stream = StreamState::Disconnected;
        self.last_event = None;
        self.last_error = None;
        self.status_notice = None;
        self.selected = None;
        self.selected_break = None;
        self.desk_filter = None;
        self.search.clear();
        self.break_search.clear();
        self.password.clear();
        self.ladder = None;
        self.ladder_ccy = None;
        self.ladder_date = None;
        self.ladder_flows.clear();
        self.ladder_movements.clear();
        self.ticket = None;
        self.amend = None;
        self.history = None;
    }

    fn drain_events(&mut self) {
        while let Ok(ev) = self.net.events.try_recv() {
            match ev {
                NetEvent::LoginOk(resp) => {
                    self.logging_in = false;
                    self.user = Some(resp.user);
                    self.desks = resp.desks;
                    self.expires_at = Some(resp.expires_at);
                    self.screen = Screen::Main;
                    self.last_error = None;
                    self.last_input = Instant::now();
                    self.last_heartbeat = Instant::now();
                    self.last_feeds = Instant::now();
                }
                NetEvent::LoginFailed(msg) => {
                    self.logging_in = false;
                    self.login_error = Some(msg);
                }
                NetEvent::Deals { deals, as_of } => {
                    self.store.replace(deals, as_of);
                    self.last_event = Some(Utc::now());
                    self.ladder_dirty = true;
                }
                NetEvent::DealUpdated(deal) => {
                    let refresh = self.selected == Some(deal.id);
                    self.store.upsert(deal);
                    self.last_event = Some(Utc::now());
                    self.ladder_dirty = true;
                    if refresh {
                        if let Some(id) = self.selected {
                            self.net.send(Command::FetchProvenance(id));
                            if self.detail_tab == DetailTab::History {
                                self.net.send(Command::FetchHistory(id));
                            }
                        }
                    }
                }
                NetEvent::Ladder(l) => {
                    self.ladder_ccy = Some(l.ccy.clone());
                    if let Some(d) = self.ladder_date {
                        if !l.rows.iter().any(|r| r.date == d) {
                            self.ladder_date = None;
                        }
                    }
                    self.ladder = Some(l);
                }
                NetEvent::LadderDetail { ccy, date, flows, movements } => {
                    if self.ladder_ccy.as_deref() == Some(ccy.as_str()) && self.ladder_date == Some(date) {
                        self.ladder_flows = flows;
                        self.ladder_movements = movements;
                    }
                }
                NetEvent::Booked(d) => {
                    self.ticket = None;
                    let id = d.id;
                    self.store.upsert(d);
                    self.ladder_dirty = true;
                    self.tab = Tab::Blotter;
                    self.select_deal(Some(id));
                }
                NetEvent::BookingFailed(msg) => {
                    if let Some(t) = &mut self.ticket {
                        t.submitting = false;
                        t.error = Some(msg);
                    }
                }
                NetEvent::History(id, h) => {
                    if self.selected == Some(id) {
                        self.history = Some((id, h));
                    }
                }
                NetEvent::Positions(p) => self.positions = p,
                NetEvent::Breaks(b) => self.breaks.replace(b),
                NetEvent::BreakUpdated(b) => {
                    let touches_selected = b.deal_id.is_some() && b.deal_id == self.selected;
                    self.breaks.upsert(b);
                    self.last_event = Some(Utc::now());
                    if touches_selected {
                        if let Some(id) = self.selected {
                            self.net.send(Command::FetchProvenance(id));
                        }
                    }
                }
                NetEvent::Provenance(p) => {
                    if self.selected == Some(p.deal.id) {
                        self.provenance = Some(p);
                    }
                }
                NetEvent::Feeds(f) => self.feeds = Some(f),
                NetEvent::Notice(msg) => self.status_notice = Some((Instant::now(), msg)),
                NetEvent::Stream(s) => self.stream = s,
                NetEvent::Error(msg) => {
                    tracing::warn!(%msg);
                    self.last_error = Some((Utc::now(), msg));
                }
                NetEvent::SessionExpired(msg) => {
                    if self.screen == Screen::Main {
                        self.reset_session();
                        self.notice = Some(msg);
                    }
                }
                NetEvent::LoggedOut => self.reset_session(),
            }
        }
    }

    /// Only genuine user activity keeps the server-side idle timer fresh. Event-driven fetches are
    /// marked passive, so an abandoned workstation times out even on a busy desk.
    fn track_presence(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| !i.events.is_empty() || i.pointer.any_down()) {
            self.last_input = Instant::now();
        }
        if self.screen == Screen::Main
            && self.last_heartbeat.elapsed() >= HEARTBEAT_EVERY
            && self.last_input.elapsed() < HEARTBEAT_EVERY
        {
            self.last_heartbeat = Instant::now();
            self.net.send(Command::Heartbeat);
        }
        if self.screen == Screen::Main && self.last_feeds.elapsed() >= FEEDS_EVERY {
            self.last_feeds = Instant::now();
            self.net.send(Command::FetchFeeds);
        }
        // The ladder is recomputed on the server; refetch when deals changed, at most every 2 s.
        if self.screen == Screen::Main && self.tab == Tab::Ladder && self.ladder_dirty && self.last_ladder.elapsed() >= Duration::from_secs(2) {
            self.ladder_dirty = false;
            self.last_ladder = Instant::now();
            self.net.send(Command::FetchLadder { ccy: self.ladder_ccy.clone(), passive: true });
        }
    }

    /// Recompute the visible row lists only when the filter inputs or the stores changed.
    fn refresh_filters(&mut self) {
        let key = (self.desk_filter, self.search.to_lowercase(), self.only_breaks, self.only_provisional, self.store.version);
        if key != self.filter_key {
            let needle = key.1.clone();
            self.filtered = self
                .store
                .rows()
                .iter()
                .enumerate()
                .filter(|(_, r)| self.desk_filter.is_none_or(|d| r.deal.desk_id == d))
                .filter(|(_, r)| !self.only_breaks || r.deal.match_status == domain::MatchStatus::Break)
                .filter(|(_, r)| !self.only_provisional || !r.deal.confirmed)
                .filter(|(_, r)| needle.is_empty() || r.search.contains(&needle))
                .map(|(i, _)| i)
                .collect();
            self.filter_key = key;
        }
        let bkey = (self.break_search.to_lowercase(), self.breaks.version);
        if bkey != self.break_filter_key {
            let needle = bkey.0.clone();
            self.break_filtered = self
                .breaks
                .rows()
                .iter()
                .enumerate()
                .filter(|(_, r)| needle.is_empty() || r.search.contains(&needle))
                .map(|(i, _)| i)
                .collect();
            self.break_filter_key = bkey;
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        self.track_presence(ctx);
        let flashing = self.store.tick_flash(4) | self.breaks.tick_flash(4);
        if flashing {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        if let Some((at, _)) = self.status_notice {
            if at.elapsed() > Duration::from_secs(8) {
                self.status_notice = None;
            }
        }
        if self.screen == Screen::Main {
            self.refresh_filters();
            // Wake up regularly so heartbeats, feed polls and freshness ages update without input.
            ctx.request_repaint_after(Duration::from_secs(2));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        match self.screen {
            Screen::Login => ui::login::show(self, ui),
            Screen::Main => ui::main::show(self, ui),
        }
    }
}
