//! The page after login: what matters on the user's desks right now, built once from the data the
//! client already holds and redrawn from pre-formatted strings. ALM tiles join once the ALM
//! endpoints exist.

use chrono::{DateTime, Utc};
use domain::{BreakKind, BreakStatus, DealStatus, MatchStatus};
use egui::{Align, Layout, RichText, ScrollArea, Stroke};
use rust_decimal::prelude::ToPrimitive;

use crate::app::{App, Tab, Target};
use crate::attention::reference_now;
use crate::state::match_label;
use crate::ui::charts;
use crate::ui::fmt::{fmt_amount, fmt_date, fmt_money_short, fmt_time};
use crate::ui::theme::{Palette, pal};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Neutral,
    Positive,
    Warning,
    Negative,
}

impl Tone {
    fn color(self, p: &Palette, ui: &egui::Ui) -> egui::Color32 {
        match self {
            Tone::Neutral => ui.visuals().text_color(),
            Tone::Positive => p.positive,
            Tone::Warning => p.warning,
            Tone::Negative => p.negative,
        }
    }
}

pub struct Tile {
    pub title: String,
    pub value: String,
    pub sub: String,
    pub tone: Tone,
    pub target: Option<Target>,
}

pub struct ActivityRow {
    pub when: String,
    pub kind: String,
    pub tone: Tone,
    pub text: String,
    pub target: Target,
    pub at: DateTime<Utc>,
}

#[derive(Default)]
pub struct OverviewView {
    pub as_of: String,
    pub tiles: Vec<Tile>,
    pub breaks_by_kind: Vec<(String, f64)>,
    pub match_counts: Vec<(String, f64)>,
    pub ladder_title: String,
    pub ladder_bars: Vec<(String, f64)>,
    pub activity: Vec<ActivityRow>,
    /// desk, currency, formatted net, negative?
    pub positions: Vec<(String, String, String, bool)>,
}

pub fn build(app: &App) -> OverviewView {
    let now = reference_now(app);
    let rows = app.store.rows();
    let latest = rows.iter().map(|r| r.deal.trade_date).max();
    let on_latest = latest.map(|d| rows.iter().filter(|r| r.deal.trade_date == d).count()).unwrap_or(0);
    let provisional = rows.iter().filter(|r| !r.deal.confirmed && r.deal.status != DealStatus::Cancelled).count();
    let unmatched = rows.iter().filter(|r| matches!(r.deal.match_status, MatchStatus::Unmatched | MatchStatus::Partial)).count();
    let with_breaks = rows.iter().filter(|r| r.deal.match_status == MatchStatus::Break).count();
    let open = app.breaks.open_count();
    let sla = app.breaks.rows().iter().filter(|r| r.view.kind == BreakKind::Missing && r.view.status != BreakStatus::Resolved).count();

    let mut tiles = vec![
        Tile {
            title: match latest {
                Some(d) => format!("Deals on {}", fmt_date(d)),
                None => "Deals".into(),
            },
            value: on_latest.to_string(),
            sub: format!("{} in the blotter", rows.len()),
            tone: Tone::Neutral,
            target: Some(Target::Tab(Tab::Blotter)),
        },
        Tile {
            title: "Provisional".into(),
            value: provisional.to_string(),
            sub: "awaiting back-office confirmation".into(),
            tone: if provisional > 0 { Tone::Warning } else { Tone::Positive },
            target: Some(Target::Provisional),
        },
        Tile {
            title: "Unmatched".into(),
            value: unmatched.to_string(),
            sub: format!("{with_breaks} with breaks"),
            tone: if with_breaks > 0 { Tone::Negative } else if unmatched > 0 { Tone::Warning } else { Tone::Positive },
            target: Some(Target::Tab(Tab::Blotter)),
        },
        Tile {
            title: "Open breaks".into(),
            value: open.to_string(),
            sub: kinds_summary(app),
            tone: if open > 0 { Tone::Negative } else { Tone::Positive },
            target: Some(Target::Tab(Tab::Breaks)),
        },
        Tile {
            title: "SLA breaches".into(),
            value: sla.to_string(),
            sub: "sources missing past their expected lag".into(),
            tone: if sla > 0 { Tone::Negative } else { Tone::Positive },
            target: Some(Target::Tab(Tab::Breaks)),
        },
    ];

    if let Some(f) = &app.feeds {
        let mut healthy = 0;
        let mut total = 0;
        for feed in &f.feeds {
            if matches!(feed.source, domain::Source::Internal | domain::Source::User) {
                continue;
            }
            total += 1;
            let age = feed.last_heartbeat_at.map(|hb| (now - hb).num_seconds().max(0));
            if crate::ui::fmt::feed_health(age) == crate::ui::fmt::Health::Good {
                healthy += 1;
            }
        }
        tiles.push(Tile {
            title: "Feeds".into(),
            value: format!("{healthy}/{total}"),
            sub: match &f.recon {
                Some(r) if r.pending > 0 => format!("fresh · {} events pending", r.pending),
                Some(_) => "fresh · reconciler idle".into(),
                None => "reconciler down".into(),
            },
            tone: if healthy == total && f.recon.is_some() { Tone::Positive } else { Tone::Negative },
            target: None,
        });
    }

    let mut breaks_by_kind = Vec::new();
    for kind in BreakKind::ALL {
        let n = app.breaks.rows().iter().filter(|r| r.view.kind == *kind && r.view.status != BreakStatus::Resolved).count();
        breaks_by_kind.push((title_case(kind.as_str()), n as f64));
    }
    let match_counts = [MatchStatus::Unmatched, MatchStatus::Partial, MatchStatus::Matched, MatchStatus::Break]
        .iter()
        .map(|m| (title_case(match_label(*m)), rows.iter().filter(|r| r.deal.match_status == *m).count() as f64))
        .collect();

    let today = now.date_naive();
    let (ladder_title, ladder_bars) = match &app.ladder {
        Some(l) => (
            format!("Expected net cash {} · next five value dates", l.ccy),
            l.rows.iter().filter(|r| r.date >= today).take(5).map(|r| (r.date.format("%d %b").to_string(), r.net_expected.to_f64().unwrap_or(0.0))).collect(),
        ),
        None => ("Expected net cash".into(), Vec::new()),
    };

    let mut activity: Vec<ActivityRow> = Vec::new();
    let mut recent: Vec<&crate::state::DealRow> = rows.iter().collect();
    recent.sort_by(|a, b| b.deal.updated_at.cmp(&a.deal.updated_at));
    for r in recent.into_iter().take(12) {
        let d = &r.deal;
        activity.push(ActivityRow {
            when: fmt_time(d.updated_at),
            kind: if d.version <= 1 { "booked".into() } else { d.status.as_str().to_lowercase() },
            tone: match d.status {
                DealStatus::Cancelled => Tone::Negative,
                DealStatus::Confirmed | DealStatus::Settled => Tone::Positive,
                _ => Tone::Neutral,
            },
            text: format!("{} {} {} {} · {}", d.deal_ref, r.product, r.pair, r.notional, match_label(d.match_status)),
            target: Target::Deal(d.id),
            at: d.updated_at,
        });
    }
    for b in app.breaks.rows().iter().take(12) {
        let v = &b.view;
        activity.push(ActivityRow {
            when: fmt_time(v.updated_at),
            kind: format!("break {}", v.status.as_str().to_lowercase()),
            tone: if v.status == BreakStatus::Resolved { Tone::Positive } else { Tone::Negative },
            text: format!("#{} {} {}{}", v.id, b.kind, b.deal_ref, if b.field.is_empty() { String::new() } else { format!(" · {}", b.field) }),
            target: Target::Break(v.id),
            at: v.updated_at,
        });
    }
    activity.sort_by(|a, b| b.at.cmp(&a.at));
    activity.truncate(14);

    let mut positions: Vec<_> = app.positions.iter().collect();
    positions.sort_by(|a, b| b.net_notional.abs().cmp(&a.net_notional.abs()));
    let positions = positions.into_iter().take(8).map(|p| (p.desk_code.clone(), p.ccy.clone(), fmt_amount(p.net_notional), p.net_notional.is_sign_negative())).collect();

    OverviewView {
        as_of: format!("as of {}", fmt_time(now)),
        tiles,
        breaks_by_kind,
        match_counts,
        ladder_title,
        ladder_bars,
        activity,
        positions,
    }
}

fn kinds_summary(app: &App) -> String {
    let parts: Vec<String> = BreakKind::ALL
        .iter()
        .filter_map(|k| {
            let n = app.breaks.rows().iter().filter(|r| r.view.kind == *k && r.view.status != BreakStatus::Resolved).count();
            (n > 0).then(|| format!("{n} {}", k.as_str().to_lowercase()))
        })
        .collect();
    if parts.is_empty() { "none open".into() } else { parts.join(" · ") }
}

fn title_case(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
        None => String::new(),
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let mut nav: Option<Target> = None;
    ScrollArea::vertical().id_salt("overview").show(ui, |ui| {
        let p = pal(ui);
        ui.horizontal(|ui| {
            ui.heading("Overview");
            ui.label(RichText::new(&app.overview.as_of).color(p.muted));
        });
        ui.add_space(6.0);

        ui.horizontal_wrapped(|ui| {
            for t in &app.overview.tiles {
                if tile(ui, &p, t).clicked() {
                    nav = t.target.clone();
                }
            }
        });
        ui.add_space(10.0);

        ui.horizontal_wrapped(|ui| {
            card(ui, &p, "Open breaks by kind", 300.0, |ui| charts::bars(ui, &app.overview.breaks_by_kind, p.series[1], 150.0, |v| format!("{v:.0}")));
            card(ui, &p, "Match status", 300.0, |ui| charts::bars(ui, &app.overview.match_counts, p.series[0], 150.0, |v| format!("{v:.0}")));
            card(ui, &p, &app.overview.ladder_title, 380.0, |ui| {
                charts::bars(ui, &app.overview.ladder_bars, p.series[2], 150.0, fmt_money_short);
                if ui.link(RichText::new("open the liquidity ladder").small()).clicked() {
                    nav = Some(Target::Tab(Tab::Ladder));
                }
            });
        });
        ui.add_space(10.0);

        ui.horizontal_wrapped(|ui| {
            card(ui, &p, "Recent activity", 520.0, |ui| {
                if app.overview.activity.is_empty() {
                    ui.label(RichText::new("nothing yet").color(p.muted));
                }
                for a in &app.overview.activity {
                    let row = ui
                        .horizontal(|ui| {
                            ui.label(RichText::new(&a.when).color(p.muted).monospace().small());
                            ui.colored_label(a.tone.color(&p, ui), RichText::new(&a.kind).small());
                            ui.label(RichText::new(&a.text).small());
                        })
                        .response
                        .interact(egui::Sense::click());
                    if row.clicked() {
                        nav = Some(a.target.clone());
                    }
                }
            });
            card(ui, &p, "Largest positions", 320.0, |ui| {
                if app.overview.positions.is_empty() {
                    ui.label(RichText::new("no positions").color(p.muted));
                }
                for (desk, ccy, net, negative) in &app.overview.positions {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(desk).color(p.muted).small());
                        ui.monospace(ccy);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.monospace(RichText::new(net).color(if *negative { p.negative } else { p.positive }));
                        });
                    });
                }
            });
        });
        ui.add_space(12.0);
    });
    if let Some(t) = nav {
        app.navigate(t);
    }
}

fn frame(p: &Palette) -> egui::Frame {
    egui::Frame::new().fill(p.card).stroke(Stroke::new(1.0, p.card_stroke)).corner_radius(6).inner_margin(10)
}

fn tile(ui: &mut egui::Ui, p: &Palette, t: &Tile) -> egui::Response {
    let resp = frame(p)
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_min_width(170.0);
                ui.label(RichText::new(&t.title).color(p.muted).small());
                ui.label(RichText::new(&t.value).size(24.0).strong().color(t.tone.color(p, ui)));
                ui.label(RichText::new(&t.sub).color(p.muted).small());
            });
        })
        .response;
    if t.target.is_some() { resp.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

fn card(ui: &mut egui::Ui, p: &Palette, title: &str, width: f32, body: impl FnOnce(&mut egui::Ui)) {
    frame(p).show(ui, |ui| {
        ui.vertical(|ui| {
            ui.set_width(width);
            ui.label(RichText::new(title).color(p.muted).small().strong());
            ui.add_space(4.0);
            body(ui);
        });
    });
}
