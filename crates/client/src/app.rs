use std::time::Duration;

use chrono::{DateTime, Utc};
use domain::{DeskAccess, Position, UserInfo};
use uuid::Uuid;

use crate::net::{self, Command, NetEvent, NetHandle, StreamState};
use crate::state::DealStore;
use crate::ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Login,
    Main,
}

pub struct App {
    pub net: NetHandle,
    pub screen: Screen,

    // login form
    pub server: String,
    pub username: String,
    pub password: String,
    pub login_error: Option<String>,
    pub logging_in: bool,

    // session
    pub user: Option<UserInfo>,
    pub desks: Vec<DeskAccess>,
    pub expires_at: Option<DateTime<Utc>>,

    // data
    pub store: DealStore,
    pub positions: Vec<Position>,
    pub stream: StreamState,
    pub last_event: Option<DateTime<Utc>>,
    pub last_error: Option<(DateTime<Utc>, String)>,

    // view state
    pub desk_filter: Option<i16>,
    pub search: String,
    pub selected: Option<Uuid>,
    pub filtered: Vec<usize>,
    filter_key: (Option<i16>, String, u64),
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, server: String) -> Self {
        cc.egui_ctx.all_styles_mut(|style| style.spacing.item_spacing = egui::vec2(6.0, 4.0));

        Self {
            net: net::spawn(cc.egui_ctx.clone()),
            screen: Screen::Login,
            server,
            username: String::new(),
            password: String::new(),
            login_error: None,
            logging_in: false,
            user: None,
            desks: Vec::new(),
            expires_at: None,
            store: DealStore::default(),
            positions: Vec::new(),
            stream: StreamState::Disconnected,
            last_event: None,
            last_error: None,
            desk_filter: None,
            search: String::new(),
            selected: None,
            filtered: Vec::new(),
            filter_key: (None, String::new(), u64::MAX),
        }
    }

    pub fn submit_login(&mut self) {
        if self.logging_in || self.username.trim().is_empty() || self.password.is_empty() {
            return;
        }
        self.logging_in = true;
        self.login_error = None;
        self.net.send(Command::Login {
            server: self.server.trim().to_string(),
            username: self.username.trim().to_string(),
            password: std::mem::take(&mut self.password),
        });
    }

    pub fn logout(&mut self) {
        self.net.send(Command::Logout);
        self.reset_session();
    }

    fn reset_session(&mut self) {
        self.screen = Screen::Login;
        self.user = None;
        self.desks.clear();
        self.expires_at = None;
        self.store.clear();
        self.positions.clear();
        self.stream = StreamState::Disconnected;
        self.last_event = None;
        self.selected = None;
        self.desk_filter = None;
        self.search.clear();
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
                }
                NetEvent::LoginFailed(msg) => {
                    self.logging_in = false;
                    self.login_error = Some(msg);
                }
                NetEvent::Deals { deals, as_of } => {
                    self.store.replace(deals, as_of);
                    self.last_event = Some(Utc::now());
                }
                NetEvent::DealUpdated(deal) => {
                    self.store.upsert(deal);
                    self.last_event = Some(Utc::now());
                }
                NetEvent::Positions(p) => self.positions = p,
                NetEvent::Stream(s) => self.stream = s,
                NetEvent::Error(msg) => {
                    tracing::warn!(%msg);
                    self.last_error = Some((Utc::now(), msg));
                }
                NetEvent::LoggedOut => self.reset_session(),
            }
        }
    }

    /// Recompute the visible row list only when the filter inputs or the store changed.
    fn refresh_filter(&mut self) {
        let key = (self.desk_filter, self.search.to_lowercase(), self.store.version);
        if key == self.filter_key {
            return;
        }
        let needle = key.1.clone();
        self.filtered = self
            .store
            .rows()
            .iter()
            .enumerate()
            .filter(|(_, r)| self.desk_filter.is_none_or(|d| r.deal.desk_id == d))
            .filter(|(_, r)| needle.is_empty() || r.search.contains(&needle))
            .map(|(i, _)| i)
            .collect();
        self.filter_key = key;
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        if self.store.tick_flash(4) {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        if self.screen == Screen::Main {
            self.refresh_filter();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        match self.screen {
            Screen::Login => ui::login::show(self, ui),
            Screen::Main => ui::main::show(self, ui),
        }
    }
}
