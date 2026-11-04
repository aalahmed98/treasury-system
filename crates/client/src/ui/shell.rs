//! The main window: a status bar, a top bar with the tabs, feed health, the attention bell, theme
//! buttons and the user menu, and the active tab in the middle. Positions keep their side panel on
//! the trading tabs only.

use domain::Source;
use egui::{Align, CentralPanel, Layout, Panel, Popup, PopupCloseBehavior, RectAlign, RichText, ScrollArea};

use crate::app::{App, Severity, Tab, Target};
use crate::ui::fmt::{feed_health, fmt_age, fmt_local, recon_health};
use crate::ui::theme::{Palette, pal};
use crate::ui::{blotter, breaks, ladder, overview, positions, status_bar, ticket};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    Panel::bottom("status_bar").show(ui, |ui| status_bar::show(app, ui));
    Panel::top("topbar").show(ui, |ui| topbar(app, ui));

    if matches!(app.tab, Tab::Blotter | Tab::Breaks | Tab::Ladder) {
        Panel::right("positions")
            .default_size(340.0)
            .min_size(240.0)
            .resizable(true)
            .show(ui, |ui| positions::show(app, ui));
    }

    CentralPanel::default().show(ui, |ui| match app.tab {
        Tab::Overview => overview::show(app, ui),
        Tab::Blotter => blotter::show(app, ui),
        Tab::Breaks => breaks::show(app, ui),
        Tab::Ladder => ladder::show(app, ui),
        Tab::Alm => crate::ui::alm::show(app, ui),
    });

    ticket::window(app, ui.ctx());
}

pub fn severity_color(p: &Palette, s: Severity) -> egui::Color32 {
    match s {
        Severity::Critical => p.critical,
        Severity::Warning => p.warning,
        Severity::Info => p.info,
    }
}

fn topbar(app: &mut App, ui: &mut egui::Ui) {
    ui.add_space(3.0);
    ui.horizontal(|ui| {
        let p = pal(ui);
        ui.label(RichText::new("NBB Treasury").strong().size(16.0));
        if let Some(code) = app.home_desk_code() {
            ui.label(RichText::new(code).color(p.muted)).on_hover_text("home desk");
        }
        ui.separator();

        tab(app, ui, Tab::Overview, RichText::new("Overview"));
        tab(app, ui, Tab::Blotter, RichText::new(format!("Blotter ({})", app.store.len())));
        let open = app.breaks.open_count();
        let label = if open > 0 { RichText::new(format!("Breaks ({open} open)")).color(p.negative) } else { RichText::new("Breaks") };
        tab(app, ui, Tab::Breaks, label);
        tab(app, ui, Tab::Ladder, RichText::new("Liquidity"));
        if app.can_see_alm() {
            tab(app, ui, Tab::Alm, RichText::new("ALM"));
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            user_menu(app, ui);
            egui::global_theme_preference_buttons(ui);
            ui.separator();
            bell(app, ui);
            ui.separator();
            feeds(app, ui);
        });
    });
    ui.add_space(3.0);
}

fn tab(app: &mut App, ui: &mut egui::Ui, t: Tab, label: RichText) {
    if ui.selectable_label(app.tab == t, label).clicked() {
        app.open_tab(t);
    }
}

fn user_menu(app: &mut App, ui: &mut egui::Ui) {
    let name = app.user.as_ref().map(|u| u.display_name.clone()).unwrap_or_default();
    let mut logout = false;
    ui.menu_button(format!("{name} ▾"), |ui| {
        let p = pal(ui);
        ui.set_min_width(220.0);
        if let Some(u) = &app.user {
            ui.label(RichText::new(&u.username).color(p.muted));
        }
        ui.separator();
        ui.label(RichText::new("Desks").color(p.muted).small());
        for d in &app.desks {
            let rights = match (d.can_book, d.can_resolve) {
                (true, _) => "book and resolve",
                (false, true) => "read, resolve breaks",
                _ => "read only",
            };
            ui.horizontal(|ui| {
                ui.strong(&d.code);
                ui.label(RichText::new(rights).color(p.muted).small());
            });
        }
        ui.separator();
        if ui.button("Log out").clicked() {
            logout = true;
            ui.close();
        }
    });
    if logout {
        app.logout();
    }
}

fn bell(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    let n = app.attention.len();
    let color = match app.attention.first().map(|a| a.severity) {
        Some(s) => severity_color(&p, s),
        None => p.muted,
    };
    let text = if n == 0 { "🔔".to_string() } else { format!("🔔 {n}") };
    let resp = ui.button(RichText::new(text).color(color)).on_hover_text("what needs attention on your desks");
    let mut nav: Option<Target> = None;
    let force_open = app.dev_force_bell();
    Popup::from_toggle_button_response(&resp)
        .open_memory(if force_open { Some(egui::SetOpenCommand::Bool(true)) } else { None })
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .align(RectAlign::BOTTOM_END)
        .width(460.0)
        .show(|ui| {
            let p = pal(ui);
            ui.strong("Needs attention");
            if app.attention.is_empty() {
                ui.label(RichText::new("Nothing outstanding for your desks.").color(p.muted));
                return;
            }
            ScrollArea::vertical().max_height(440.0).show(ui, |ui| {
                for item in &app.attention {
                    let row = ui
                        .horizontal(|ui| {
                            ui.colored_label(severity_color(&p, item.severity), "●");
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&item.title).strong());
                                ui.label(RichText::new(&item.detail).color(p.muted).small());
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(&item.since).color(p.muted).small());
                            });
                        })
                        .response
                        .interact(egui::Sense::click());
                    if row.hovered() {
                        ui.painter().rect_filled(row.rect.expand(2.0), 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
                    }
                    if row.clicked() {
                        nav = Some(item.target.clone());
                        ui.close();
                    }
                    ui.separator();
                }
            });
        });
    if let Some(t) = nav {
        app.navigate(t);
    }
}

/// Per-feed age against the reconciler's clock, right to left so it reads K B Q · recon.
fn feeds(app: &App, ui: &mut egui::Ui) {
    let p = pal(ui);
    let Some(f) = &app.feeds else {
        ui.label(RichText::new("feeds —").color(p.muted));
        return;
    };
    let reference = f.recon.as_ref().map(|r| r.now).unwrap_or(f.server_time);
    match &f.recon {
        Some(r) => {
            let lag = (f.server_time - r.last_tick_at).num_seconds().max(0);
            let pending = if r.pending > 0 { format!("pending {} · ", r.pending) } else { String::new() };
            ui.colored_label(recon_health(lag).color(&p), format!("recon {pending}{}", fmt_age(lag)))
                .on_hover_text(format!("reconciler clock {}, last tick {}", fmt_local(r.now), fmt_local(r.last_tick_at)));
        }
        None => {
            ui.colored_label(p.negative, "recon down").on_hover_text("the reconciliation worker has never reported");
        }
    }
    for feed in f.feeds.iter().rev() {
        if matches!(feed.source, Source::Internal | Source::User) {
            continue;
        }
        let age = feed.last_heartbeat_at.map(|hb| (reference - hb).num_seconds().max(0));
        let text = match age {
            Some(a) => format!("{} {}", feed.source.short(), fmt_age(a)),
            None => format!("{} —", feed.source.short()),
        };
        ui.colored_label(feed_health(age).color(&p), text).on_hover_text(format!(
            "{}: {} events, {} duplicates dropped, last event {}",
            feed.source.as_str(),
            feed.events_total,
            feed.duplicates_dropped,
            feed.last_event_at.map(fmt_local).unwrap_or_else(|| "never".into())
        ));
    }
}
