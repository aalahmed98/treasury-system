use std::time::{Duration, Instant};

use chrono::NaiveDate;
use chrono::{DateTime, Utc};
use domain::{CashFlow, DeskAccess, FeedsResponse, HistoryEntry, LadderResponse, NostroMovement, Position, ProvenanceResponse, UserInfo};
use uuid::Uuid;

use crate::ServerTarget;
use crate::alm_state::{AlmState, AlmTool, OverviewAlmView};
use crate::state::Fetched;
use crate::ui::overview::ActivityRow;
use crate::net::{self, Command, NetEvent, NetHandle, StreamState};
use crate::state::{BreakStore, DealStore};
use crate::ui;
use crate::ui::overview::OverviewView;
use crate::ui::theme::ClientPrefs;
use crate::ui::ticket::{AmendForm, TicketForm};

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
    Overview,
    Blotter,
    Breaks,
    Ladder,
    Alm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Summary,
    Sources,
    History,
}

/// Where a click on an attention item, a tile or an activity row takes the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Tab(Tab),
    Deal(Uuid),
    Break(i64),
    /// The blotter filtered to provisional deals.
    Provisional,
    AlmTool(AlmTool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Critical,
    Warning,
    Info,
}

pub struct AttentionItem {
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    pub since: String,
    pub at: DateTime<Utc>,
    pub target: Target,
}

type AttentionKey = (u64, u64, Option<DateTime<Utc>>, StreamState);
type OverviewKey = (u64, u64, Option<DateTime<Utc>>, usize, Option<DateTime<Utc>>);

pub struct App {
    pub net: NetHandle,
    pub screen: Screen,
    pub tab: Tab,
    egui_ctx: egui::Context,
    /// Request ids for fetches whose replies must be matched; never reset, so a reply from a
    /// previous login can never be accepted.
    next_req: u64,

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

    // derived views, rebuilt only when their inputs change
    pub attention: Vec<AttentionItem>,
    attention_key: Option<AttentionKey>,
    pub overview: OverviewView,
    overview_key: Option<OverviewKey>,
    /// The server's attention list and activity feed; the local versions fill in until they arrive.
    pub attention_server: Fetched<Vec<domain::AttentionItem>>,
    last_attention: Instant,
    pub activity: Fetched<Vec<ActivityRow>>,
    last_activity: Instant,
    last_alm_overview: Instant,

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

    // the ALM workbench
    pub alm: AlmState,

    // booking and amendments
    pub ticket: Option<TicketForm>,
    pub amend: Option<AmendForm>,
    pub history: Option<(Uuid, Vec<HistoryEntry>)>,

    #[cfg(debug_assertions)]
    dev: crate::dev::Dev,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, target: ServerTarget) -> Self {
        ui::theme::install(&cc.egui_ctx);
        if let Some(prefs) = cc.storage.and_then(|s| eframe::get_value::<ClientPrefs>(s, "prefs")) {
            cc.egui_ctx.set_theme(prefs.theme);
        }
        let net = net::spawn(cc.egui_ctx.clone(), net::Target { url: target.url.clone(), ca: target.ca });
        Self {
            net,
            screen: Screen::Login,
            tab: Tab::Overview,
            egui_ctx: cc.egui_ctx.clone(),
            next_req: 0,
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
            attention: Vec::new(),
            attention_key: None,
            overview: OverviewView::default(),
            overview_key: None,
            attention_server: Fetched::default(),
            last_attention: Instant::now() - Duration::from_secs(60),
            activity: Fetched::default(),
            last_activity: Instant::now() - Duration::from_secs(60),
            last_alm_overview: Instant::now() - Duration::from_secs(60),
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
            alm: AlmState::default(),
            ticket: None,
            amend: None,
            history: None,
            #[cfg(debug_assertions)]
            dev: crate::dev::from_env(),
        }
    }

    /// Debug builds only: auto-login and the window snapshot driven by environment variables.
    #[cfg(debug_assertions)]
    fn dev_tick(&mut self, ctx: &egui::Context) {
        if let Some((u, p)) = self.dev.login.take() {
            self.username = u;
            self.password = p;
            self.submit_login();
        }
        let Some(snap) = &mut self.dev.snapshot else { return };
        if self.screen != Screen::Main && !snap.login {
            return;
        }
        match snap.started {
            None => {
                snap.started = Some(Instant::now());
                let (theme, zoom, size, tab, after) = (snap.theme, snap.zoom, snap.size, snap.tab, snap.after);
                if let Some(t) = theme {
                    ctx.set_theme(t);
                }
                if let Some(z) = zoom {
                    ctx.set_zoom_factor(z);
                }
                if let Some((w, h)) = size {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
                }
                if let Some(t) = tab {
                    self.open_tab(t);
                }
                if let Some(tool) = std::env::var("TREASURY_SNAPSHOT_TOOL").ok().and_then(|t| AlmTool::from_name(&t)) {
                    self.open_alm_tool(tool);
                    if tool == AlmTool::Setup && std::env::var("TREASURY_SNAPSHOT_DRAFT").is_ok() {
                        self.alm.setup.draft = Some(domain::AlmParams::default());
                        self.alm.setup.note = "snapshot draft".into();
                    }
                }
                ctx.request_repaint_after(after);
            }
            Some(at) if at.elapsed() >= snap.after && !snap.requested => {
                snap.requested = true;
                crate::dev::capture(ctx, snap.path.clone());
            }
            _ => {}
        }
    }

    /// Debug builds only: whether the snapshot wants the attention popup open.
    pub fn dev_force_bell(&self) -> bool {
        #[cfg(debug_assertions)]
        {
            self.dev.snapshot.as_ref().is_some_and(|s| s.bell && s.started.is_some())
        }
        #[cfg(not(debug_assertions))]
        {
            false
        }
    }

    /// A fresh request id. See `state::Fetched`.
#[allow(dead_code)]
    pub fn issue(&mut self) -> u64 {
        self.next_req += 1;
        self.next_req
    }

    pub fn can_book(&self, desk_id: i16) -> bool {
        self.desks.iter().any(|d| d.desk_id == desk_id && d.can_book)
    }

    pub fn bookable_desks(&self) -> Vec<DeskAccess> {
        self.desks.iter().filter(|d| d.can_book).cloned().collect()
    }

    /// The same rule as the database's `can_see_alm()`: the ALM desk or the treasury head.
    pub fn can_see_alm(&self) -> bool {
        self.desks.iter().any(|d| d.code == "ALM" || d.code == "HEAD")
    }

    pub fn home_desk_code(&self) -> Option<String> {
        let home = self.user.as_ref()?.home_desk_id;
        self.desks.iter().find(|d| d.desk_id == home).map(|d| d.code.clone())
    }

    pub fn open_tab(&mut self, tab: Tab) {
        self.tab = tab;
        if matches!(tab, Tab::Ladder | Tab::Overview) {
            self.ladder_dirty = true;
        }
        if tab == Tab::Alm && self.alm.params.req == 0 {
            let r = self.issue();
            self.alm.params.begin(r);
            self.net.send(Command::FetchAlmParams { req: r });
            let r = self.issue();
            self.alm.snapshots.begin(r);
            self.net.send(Command::FetchAlmSnapshots { req: r });
        }
    }

    /// Recompute the open ALM tool once its inputs settled; replies are matched by request id.
    fn alm_tick(&mut self) {
        if self.screen != Screen::Main || self.tab != Tab::Alm || self.alm.bank.is_none() {
            return;
        }
        let tool = self.alm.tool;
        if tool.computed() && self.alm.due() {
            let body = Box::new(self.alm.request());
            let r = self.issue();
            self.alm.begin(tool, r);
            self.net.send(Command::AlmRun { req: r, tool, body });
        }
    }

    pub fn navigate(&mut self, target: Target) {
        match target {
            Target::Tab(t) => self.open_tab(t),
            Target::Deal(id) => {
                self.open_tab(Tab::Blotter);
                self.select_deal(Some(id));
            }
            Target::Break(id) => {
                self.open_tab(Tab::Breaks);
                self.select_break(Some(id));
            }
            Target::Provisional => {
                self.only_provisional = true;
                self.open_tab(Tab::Blotter);
            }
            Target::AlmTool(tool) => {
                self.open_tab(Tab::Alm);
                self.open_alm_tool(tool);
            }
        }
    }

    pub fn open_alm_tool(&mut self, tool: AlmTool) {
        self.alm.tool = tool;
        if tool == AlmTool::Ird && self.alm.ird.rates.req == 0 {
            let r = self.issue();
            self.alm.ird.rates.begin(r);
            self.net.send(Command::FetchRates { req: r });
        }
        if tool == AlmTool::Dashboard {
            self.last_alm_overview = Instant::now() - Duration::from_secs(60);
        }
    }

    fn fetch_alm_overview(&mut self) {
        let r = self.issue();
        self.alm.overview.result.begin(r);
        self.net.send(Command::FetchAlmOverview { req: r });
        self.last_alm_overview = Instant::now();
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
        self.tab = Tab::Overview;
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
        self.attention.clear();
        self.attention_key = None;
        self.overview = OverviewView::default();
        self.overview_key = None;
        self.attention_server = Fetched::default();
        self.activity = Fetched::default();
        self.selected = None;
        self.selected_break = None;
        self.desk_filter = None;
        self.search.clear();
        self.break_search.clear();
        self.password.clear();
        self.filtered.clear();
        self.break_filtered.clear();
        // A new login's rows arrive with store versions starting from zero again; never reuse the
        // old row indices against them.
        self.filter_key.4 = u64::MAX;
        self.break_filter_key.1 = u64::MAX;
        self.ladder = None;
        self.ladder_ccy = None;
        self.ladder_date = None;
        self.ladder_flows.clear();
        self.ladder_movements.clear();
        self.ticket = None;
        self.amend = None;
        self.history = None;
        self.alm = AlmState::default();
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
                    self.tab = Tab::Overview;
                    self.ladder_dirty = true;
                    self.last_error = None;
                    self.last_input = Instant::now();
                    self.last_heartbeat = Instant::now();
                    self.last_feeds = Instant::now();
                    self.last_attention = Instant::now() - Duration::from_secs(60);
                    self.last_activity = Instant::now() - Duration::from_secs(60);
                    self.last_alm_overview = Instant::now() - Duration::from_secs(60);
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
                    self.last_attention = self.last_attention.min(Instant::now() - Duration::from_secs(13));
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
                    self.last_attention = self.last_attention.min(Instant::now() - Duration::from_secs(13));
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
                NetEvent::AlmParams { req, result } => {
                    let fresh = result.as_ref().ok().map(|r| (r.active.version, r.params.clone()));
                    if self.alm.params.accept(req, result) {
                        if let Some((version, params)) = fresh {
                            if self.alm.bank_version != 0 && version != self.alm.bank_version && self.alm.overrides.is_some() {
                                self.alm.bank_changed = true;
                            }
                            self.alm.bank_version = version;
                            self.alm.bank = Some(std::sync::Arc::new(params));
                            if self.alm.overrides.is_none() {
                                self.alm.touch();
                            }
                        }
                    }
                }
                NetEvent::AlmSnapshots { req, result } => {
                    if self.alm.snapshots.accept(req, result) {
                        if self.alm.as_of.is_none() {
                            self.alm.as_of = self.alm.snapshots.view.as_ref().and_then(|s| s.first()).map(|s| s.as_of);
                        }
                        self.alm.touch();
                    }
                }
                NetEvent::AlmResult { req, tool, result } => {
                    self.alm.accept(tool, req, result);
                }
                NetEvent::AlmOverview { req, result } => {
                    if self.alm.overview.result.accept(req, result.map(OverviewAlmView::new)) {
                        self.overview_key = None;
                    }
                }
                NetEvent::AlmSaved { req, result } => {
                    if self.alm.setup.saving.accept(req, result) && self.alm.setup.saving.view.is_some() {
                        self.alm.setup.draft = None;
                        self.alm.setup.note.clear();
                        self.alm.overrides = None;
                        self.alm.touch();
                    }
                }
                NetEvent::Rates { req, result } => {
                    if self.alm.ird.rates.accept(req, result) {
                        if let Some(m) = &self.alm.ird.rates.view {
                            self.alm.ird.rates_as_of = m.curves.first().map(|c| c.as_of.format("%d %b %Y").to_string()).unwrap_or_default();
                        }
                        self.alm.ird.refresh_from_market();
                    }
                }
                NetEvent::Activity { req, result } => {
                    self.activity.accept(req, result.map(|rows| rows.iter().map(ActivityRow::from_server).collect()));
                }
                NetEvent::Attention { req, result } => {
                    if self.attention_server.accept(req, result) {
                        self.attention_key = None;
                    }
                }
                NetEvent::AlmChanged => {
                    self.last_alm_overview = Instant::now() - Duration::from_secs(60);
                    self.last_attention = Instant::now() - Duration::from_secs(60);
                    // Only when the workbench has been opened: it reloads the parameters and the
                    // snapshot list, and the open tool recomputes unless a what-if is in progress.
                    if self.alm.params.req != 0 {
                        let r = self.issue();
                        self.alm.params.begin(r);
                        self.net.send(Command::FetchAlmParams { req: r });
                        let r = self.issue();
                        self.alm.snapshots.begin(r);
                        self.net.send(Command::FetchAlmSnapshots { req: r });
                    }
                }
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
        // Server-computed lists: the bell on every tab, activity and the ALM block on the pages
        // that show them. All passive: a dashboard never keeps an idle session alive.
        if self.screen == Screen::Main && self.last_attention.elapsed() >= Duration::from_secs(15) {
            self.last_attention = Instant::now();
            let r = self.issue();
            self.attention_server.begin(r);
            self.net.send(Command::FetchAttention { req: r });
        }
        if self.screen == Screen::Main && self.tab == Tab::Overview && self.last_activity.elapsed() >= Duration::from_secs(30) {
            self.last_activity = Instant::now();
            let r = self.issue();
            self.activity.begin(r);
            self.net.send(Command::FetchActivity { req: r, limit: 20 });
        }
        let wants_alm = self.screen == Screen::Main
            && self.can_see_alm()
            && (self.tab == Tab::Overview || (self.tab == Tab::Alm && self.alm.tool == AlmTool::Dashboard));
        if wants_alm && self.last_alm_overview.elapsed() >= Duration::from_secs(15) {
            self.fetch_alm_overview();
        }
        // The ladder is recomputed on the server; refetch when deals changed, at most every 2 s.
        // The overview shows a slice of it, so it keeps the ladder fresh too.
        if self.screen == Screen::Main
            && matches!(self.tab, Tab::Ladder | Tab::Overview)
            && self.ladder_dirty
            && self.last_ladder.elapsed() >= Duration::from_secs(2)
        {
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

    /// The attention list and the overview are derived from the stores; rebuild them only when
    /// an input changed, never per frame.
    fn refresh_derived(&mut self) {
        let feeds_at = self.feeds.as_ref().map(|f| f.server_time);
        let akey = (self.store.version, self.breaks.version, feeds_at, self.stream);
        if self.attention_key != Some(akey) {
            self.attention = match &self.attention_server.view {
                Some(items) => crate::attention::from_server(self, items),
                None => crate::attention::build(self),
            };
            self.attention_key = Some(akey);
        }
        if self.tab == Tab::Overview {
            let okey = (self.store.version, self.breaks.version, feeds_at, self.positions.len(), self.ladder.as_ref().map(|l| l.generated_at));
            if self.overview_key != Some(okey) {
                self.overview = ui::overview::build(self);
                self.overview_key = Some(okey);
            }
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        self.track_presence(ctx);
        #[cfg(debug_assertions)]
        self.dev_tick(ctx);
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
            self.refresh_derived();
            self.alm_tick();
            if self.alm.is_pending() {
                ctx.request_repaint_after(Duration::from_millis(320));
            }
            // Wake up regularly so heartbeats, feed polls and freshness ages update without input.
            ctx.request_repaint_after(Duration::from_secs(2));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        match self.screen {
            Screen::Login => ui::login::show(self, ui),
            Screen::Main => ui::shell::show(self, ui),
        }
    }

    /// Preferences only. Widget state is deliberately not persisted (it goes stale across UI
    /// changes); window geometry is, under its own key.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let theme = self.egui_ctx.options(|o| o.theme_preference);
        eframe::set_value(storage, "prefs", &ClientPrefs { theme });
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }
}
