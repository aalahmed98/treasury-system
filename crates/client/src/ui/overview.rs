//! The page after login: what matters on the user's desks right now, built once from the data the
//! client already holds and redrawn from pre-formatted strings. ALM tiles join once the ALM
//! endpoints exist.

use chrono::{DateTime, Utc};
use domain::{BreakKind, BreakStatus, DealStatus, MatchStatus};
use egui::{Align, Layout, RichText, ScrollArea};
use rust_decimal::prelude::ToPrimitive;

use crate::alm_state::{AlmTool, Standing};
use crate::app::{App, Tab, Target};
use crate::ui::alm::common::standing_color;
use crate::ui::fmt::fmt_pct;
use crate::attention::reference_now;
use crate::state::match_label;
use crate::ui::charts;
use crate::ui::fmt::{fmt_amount, fmt_date, fmt_money_short, fmt_time};
use crate::ui::theme::{Palette, medium, pal};
use crate::ui::widgets::{StatCell, Tone, card, dot, section, stat_strip, wrap_row};

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
    pub target: Option<Target>,
    pub at: DateTime<Utc>,
}

impl ActivityRow {
    /// A server activity row, formatted.
    pub fn from_server(a: &domain::ActivityRow) -> Self {
        let tone = match a.kind.as_str() {
            "cancelled" => Tone::Bad,
            k if k.starts_with("break ") && !k.contains("resolved") && !k.contains("ack") => Tone::Bad,
            "booked" | "parameters" => Tone::Good,
            _ => Tone::Neutral,
        };
        let target = match (a.break_id, a.deal_id) {
            (Some(b), _) => Some(Target::Break(b)),
            (None, Some(d)) => Some(Target::Deal(d)),
            _ => None,
        };
        Self { when: fmt_time(a.at), kind: a.kind.clone(), tone, text: format!("{} · {} · {}", a.summary, a.actor, a.desk_code), target, at: a.at }
    }
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
            tone: if provisional > 0 { Tone::Warn } else { Tone::Good },
            target: Some(Target::Provisional),
        },
        Tile {
            title: "Unmatched".into(),
            value: unmatched.to_string(),
            sub: format!("{with_breaks} with breaks"),
            tone: if with_breaks > 0 { Tone::Bad } else if unmatched > 0 { Tone::Warn } else { Tone::Good },
            target: Some(Target::Tab(Tab::Blotter)),
        },
        Tile {
            title: "Open breaks".into(),
            value: open.to_string(),
            sub: kinds_summary(app),
            tone: if open > 0 { Tone::Bad } else { Tone::Good },
            target: Some(Target::Tab(Tab::Breaks)),
        },
        Tile {
            title: "SLA breaches".into(),
            value: sla.to_string(),
            sub: "sources missing past their expected lag".into(),
            tone: if sla > 0 { Tone::Bad } else { Tone::Good },
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
            tone: if healthy == total && f.recon.is_some() { Tone::Good } else { Tone::Bad },
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
                DealStatus::Cancelled => Tone::Bad,
                DealStatus::Confirmed | DealStatus::Settled => Tone::Good,
                _ => Tone::Neutral,
            },
            text: format!("{} {} {} {} · {}", d.deal_ref, r.product, r.pair, r.notional, match_label(d.match_status)),
            target: Some(Target::Deal(d.id)),
            at: d.updated_at,
        });
    }
    for b in app.breaks.rows().iter().take(12) {
        let v = &b.view;
        activity.push(ActivityRow {
            when: fmt_time(v.updated_at),
            kind: format!("break {}", v.status.as_str().to_lowercase()),
            tone: if v.status == BreakStatus::Resolved { Tone::Good } else { Tone::Bad },
            text: format!("#{} {} {}{}", v.id, b.kind, b.deal_ref, if b.field.is_empty() { String::new() } else { format!(" · {}", b.field) }),
            target: Some(Target::Break(v.id)),
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
            ui.label(p.title("Overview"));
            ui.add_space(2.0);
            ui.label(p.small(&app.overview.as_of));
        });
        ui.add_space(10.0);

        let cells: Vec<StatCell> = app
            .overview
            .tiles
            .iter()
            .map(|t| StatCell {
                label: t.title.clone(),
                value: t.value.clone(),
                sub: t.sub.clone(),
                dot: (t.tone != Tone::Neutral).then(|| t.tone.color(&p)),
                clickable: t.target.is_some(),
            })
            .collect();
        if let Some(i) = stat_strip(ui, &p, &cells, 200.0) {
            nav = app.overview.tiles[i].target.clone();
        }

        if app.can_see_alm() {
            if let Some(t) = alm_block(app, ui, &p) {
                nav = Some(t);
            }
        }

        section(ui, &p, "Operations", "");
        wrap_row(ui, |ui| {
            card(ui, &p, "Open breaks by kind", 300.0, |ui| charts::bars(ui, &app.overview.breaks_by_kind, p.series[1], 150.0, |v| format!("{v:.0}")));
            card(ui, &p, "Match status", 300.0, |ui| charts::bars(ui, &app.overview.match_counts, p.series[0], 150.0, |v| format!("{v:.0}")));
            card(ui, &p, &app.overview.ladder_title, 380.0, |ui| {
                charts::bars(ui, &app.overview.ladder_bars, p.series[2], 150.0, fmt_money_short);
                if ui.link(RichText::new("open the liquidity ladder").small()).clicked() {
                    nav = Some(Target::Tab(Tab::Ladder));
                }
            });
        });
        ui.add_space(8.0);

        wrap_row(ui, |ui| {
            card(ui, &p, "Recent activity", 560.0, |ui| {
                let rows: &[ActivityRow] = match &app.activity.view {
                    Some(server) if !server.is_empty() => server,
                    _ => &app.overview.activity,
                };
                if rows.is_empty() {
                    ui.label(p.muted("nothing yet"));
                }
                ui.spacing_mut().item_spacing.y = 4.0;
                for a in rows {
                    let row = ui
                        .horizontal(|ui| {
                            ui.label(RichText::new(&a.when).small().color(p.muted));
                            ui.label(RichText::new(&a.kind).small().family(medium()).color(a.tone.color(&p)));
                            ui.label(RichText::new(&a.text).small().color(p.text_2));
                        })
                        .response
                        .interact(egui::Sense::click());
                    if a.target.is_some() && row.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        nav = a.target.clone();
                    }
                }
            });
            card(ui, &p, "Largest positions", 300.0, |ui| {
                if app.overview.positions.is_empty() {
                    ui.label(p.muted("no positions"));
                }
                for (desk, ccy, net, negative) in &app.overview.positions {
                    ui.horizontal(|ui| {
                        ui.label(p.small(desk));
                        ui.label(RichText::new(ccy).family(medium()).color(p.text));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(net).color(if *negative { p.bad } else { p.good }));
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

/// The ALM strip, limits and charts from the server's overview block. Returns a navigation when a
/// cell is clicked. Shared by the Overview tab and the ALM dashboard.
pub fn alm_block(app: &App, ui: &mut egui::Ui, p: &Palette) -> Option<Target> {
    let mut nav = None;
    let slot = &app.alm.overview;
    let Some(v) = &slot.result.view else {
        section(ui, p, "Balance sheet", "");
        ui.horizontal(|ui| {
            if slot.result.loading {
                ui.add(egui::Spinner::new().size(14.0));
            }
            match &slot.result.error {
                Some(e) => {
                    ui.label(RichText::new(e).color(p.warn));
                }
                None => {
                    ui.label(p.muted("loading the balance-sheet view…"));
                }
            }
        });
        return nav;
    };
    section(ui, p, "Balance sheet", &format!("as of {} · parameters v{} · in {}", v.raw.as_of.format("%d %b %Y"), v.raw.params_version, v.raw.reporting_ccy));
    let tools = [AlmTool::Lcr, AlmTool::Nsfr, AlmTool::Nii, AlmTool::Attribution, AlmTool::Attribution, AlmTool::Gap, AlmTool::Stress, AlmTool::Funding];
    let cells: Vec<StatCell> = v
        .tiles
        .iter()
        .map(|(title, value, sub, standing)| StatCell { label: title.clone(), value: value.clone(), sub: sub.clone(), dot: Some(standing_color(p, *standing)), clickable: true })
        .collect();
    if let Some(i) = stat_strip(ui, p, &cells, 200.0) {
        nav = Some(Target::AlmTool(tools[i.min(tools.len() - 1)]));
    }
    ui.add_space(10.0);
    wrap_row(ui, |ui| {
        card(ui, p, "Limits", 470.0, |ui| {
            egui::Grid::new("ov_limits").num_columns(5).spacing([10.0, 6.0]).show(ui, |ui| {
                for l in &v.limit_rows {
                    let st = if l.breach { Standing::Breach } else if l.warn { Standing::Watch } else { Standing::Comfortable };
                    dot(ui, standing_color(p, st));
                    ui.label(p.secondary(&l.metric));
                    ui.label(RichText::new(&l.value).family(medium()).color(p.text));
                    ui.label(p.small(&l.limit));
                    charts::limit_gauge(ui, l.gauge_value, l.gauge_limit, l.gauge_trigger, l.gauge_max, l.floor, p, 100.0);
                    ui.end_row();
                }
            });
        });
        card(ui, p, "LCR, last 30 snapshots", 380.0, |ui| {
            let labels = v.lcr_trend_labels.clone();
            charts::lines(ui, "ov_lcr_trend", &[("LCR".to_string(), v.lcr_trend.clone())], 150.0, |y| fmt_pct(y, 0), move |x| {
                labels.get(x.round() as usize).cloned().unwrap_or_default()
            });
        });
        card(ui, p, "NII by month, actual against budget", 420.0, |ui| charts::bars_pair(ui, &v.nii_months, ("actual", "budget"), p, 150.0, fmt_money_short));
        card(ui, p, "Repricing gap by bucket", 380.0, |ui| charts::bars(ui, &v.gap_bars, p.series[2], 150.0, fmt_money_short));
    });
    nav
}
