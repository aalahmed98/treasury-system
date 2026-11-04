//! The main window: a top bar with the wordmark, the tabs, feed health, the attention bell, the
//! theme switch and the user menu; the active tab in the middle; a status bar underneath.
//! Positions keep their side panel on the trading tabs only.

use domain::Source;
use egui::{Align, CentralPanel, Color32, FontId, Layout, Margin, Panel, Popup, PopupCloseBehavior, RectAlign, RichText, ScrollArea, Sense, Stroke};

use crate::app::{App, Severity, Tab, Target};
use crate::ui::fmt::{feed_health, fmt_age, fmt_local, recon_health};
use crate::ui::theme::{Palette, medium, pal, semibold};
use crate::ui::widgets::{Tone, badge, canvas_frame, dot, side_frame, tab, table_frame, theme_switch};
use crate::ui::{blotter, breaks, ladder, overview, positions, status_bar, ticket};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    Panel::bottom("status_bar")
        .frame(egui::Frame::new().fill(p.surface_alt).inner_margin(Margin::symmetric(16, 5)))
        .show(ui, |ui| status_bar::show(app, ui));
    Panel::top("topbar")
        .frame(egui::Frame::new().fill(p.surface).inner_margin(Margin { left: 16, right: 16, top: 0, bottom: 0 }))
        .show(ui, |ui| topbar(app, ui));

    if matches!(app.tab, Tab::Blotter | Tab::Breaks | Tab::Ladder) {
        Panel::right("positions")
            .default_size(340.0)
            .min_size(240.0)
            .resizable(true)
            .frame(side_frame(&p))
            .show(ui, |ui| positions::show(app, ui));
    }

    let frame = match app.tab {
        Tab::Overview => canvas_frame(&p),
        Tab::Alm => egui::Frame::new().fill(p.canvas),
        Tab::Blotter | Tab::Breaks | Tab::Ladder => table_frame(&p),
    };
    CentralPanel::default().frame(frame).show(ui, |ui| match app.tab {
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
        Severity::Critical => p.bad,
        Severity::Warning => p.warn,
        Severity::Info => p.info,
    }
}

fn severity_tone(s: Option<Severity>) -> Tone {
    match s {
        Some(Severity::Critical) => Tone::Bad,
        Some(Severity::Warning) => Tone::Warn,
        Some(Severity::Info) => Tone::Info,
        None => Tone::Neutral,
    }
}

fn topbar(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    ui.horizontal(|ui| {
        ui.set_height(40.0);
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label(RichText::new("NBB").family(semibold()).size(15.0).color(p.brand));
        ui.add_space(4.0);
        ui.label(RichText::new("Treasury").family(semibold()).size(15.0).color(p.text));
        if let Some(code) = app.home_desk_code() {
            ui.add_space(8.0);
            badge(ui, &p, &code, Tone::Neutral).on_hover_text("home desk");
        }
        ui.add_space(20.0);

        let deals = app.store.len().to_string();
        let open = app.breaks.open_count();
        let open_s = open.to_string();
        tab_to(app, ui, &p, Tab::Overview, "Overview", None);
        tab_to(app, ui, &p, Tab::Blotter, "Blotter", Some((deals.as_str(), Tone::Neutral)));
        tab_to(app, ui, &p, Tab::Breaks, "Breaks", (open > 0).then_some((open_s.as_str(), Tone::Bad)));
        tab_to(app, ui, &p, Tab::Ladder, "Liquidity", None);
        if app.can_see_alm() {
            tab_to(app, ui, &p, Tab::Alm, "ALM", None);
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            user_menu(app, ui);
            theme_switch(ui, &p);
            bell(app, ui);
            feeds(app, ui);
        });
    });
}

fn tab_to(app: &mut App, ui: &mut egui::Ui, p: &Palette, t: Tab, label: &str, count: Option<(&str, Tone)>) {
    if tab(ui, p, app.tab == t, label, count).clicked() {
        app.open_tab(t);
    }
}

fn user_menu(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    let name = app.user.as_ref().map(|u| u.display_name.clone()).unwrap_or_default();
    let mut logout = false;
    let button = egui::Button::new(RichText::new(format!("{name}  ▾")).family(medium()).color(p.text_2)).frame(false);
    let _ = egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
        let p = pal(ui);
        ui.set_min_width(220.0);
        if let Some(u) = &app.user {
            ui.label(RichText::new(&u.display_name).family(medium()).color(p.text));
            ui.label(p.small(&u.username));
        }
        ui.separator();
        ui.label(p.eyebrow("Desks"));
        for d in &app.desks {
            let rights = match (d.can_book, d.can_resolve) {
                (true, _) => "book and resolve",
                (false, true) => "read, resolve breaks",
                _ => "read only",
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(&d.code).family(medium()).color(p.text));
                ui.label(p.small(rights));
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
    let tone = severity_tone(app.attention.first().map(|a| a.severity));
    let text = if n == 0 { "🔔".to_string() } else { format!("🔔 {n}") };
    let galley = ui.painter().layout_no_wrap(text, FontId::new(12.0, medium()), Color32::PLACEHOLDER);
    let size = egui::vec2(galley.size().x + 18.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    let fill = if n > 0 { tone.soft(&p) } else if resp.hovered() { p.hover_wash } else { Color32::TRANSPARENT };
    painter.rect_filled(rect, 12.0, fill);
    painter.galley(rect.center() - galley.size() / 2.0, galley, if n > 0 { tone.color(&p) } else { p.text_2 });
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("what needs attention on your desks");

    let mut nav: Option<Target> = None;
    let force_open = app.dev_force_bell();
    Popup::from_toggle_button_response(&resp)
        .open_memory(if force_open { Some(egui::SetOpenCommand::Bool(true)) } else { None })
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .align(RectAlign::BOTTOM_END)
        .width(460.0)
        .frame(egui::Frame::new().fill(p.raised).stroke(Stroke::new(1.0, p.hairline)).corner_radius(10).inner_margin(Margin::symmetric(12, 10)).shadow(p.shadow()))
        .show(|ui| {
            let p = pal(ui);
            ui.horizontal(|ui| {
                ui.label(p.heading("Needs attention"));
                if n > 0 {
                    badge(ui, &p, &n.to_string(), tone);
                }
            });
            if app.attention.is_empty() {
                ui.add_space(4.0);
                ui.label(p.muted("Nothing outstanding for your desks."));
                return;
            }
            ui.add_space(4.0);
            ScrollArea::vertical().max_height(440.0).show(ui, |ui| {
                for (i, item) in app.attention.iter().enumerate() {
                    if i > 0 {
                        ui.separator();
                    }
                    let row = ui
                        .horizontal(|ui| {
                            ui.add_space(2.0);
                            dot(ui, severity_color(&p, item.severity));
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.label(RichText::new(&item.title).family(medium()).color(p.text));
                                ui.label(p.small(&item.detail));
                            });
                            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                                ui.label(p.small(&item.since));
                            });
                        })
                        .response
                        .interact(Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if row.clicked() {
                        nav = Some(item.target.clone());
                        ui.close();
                    }
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
        ui.label(p.small("feeds —"));
        return;
    };
    let reference = f.recon.as_ref().map(|r| r.now).unwrap_or(f.server_time);
    match &f.recon {
        Some(r) => {
            let lag = (f.server_time - r.last_tick_at).num_seconds().max(0);
            let pending = if r.pending > 0 { format!("pending {} · ", r.pending) } else { String::new() };
            feed_item(ui, &p, recon_health(lag).color(&p), "recon", &format!("{pending}{}", fmt_age(lag)))
                .on_hover_text(format!("reconciler clock {}, last tick {}", fmt_local(r.now), fmt_local(r.last_tick_at)));
        }
        None => {
            feed_item(ui, &p, p.bad, "recon", "down").on_hover_text("the reconciliation worker has never reported");
        }
    }
    for feed in f.feeds.iter().rev() {
        if matches!(feed.source, Source::Internal | Source::User) {
            continue;
        }
        let age = feed.last_heartbeat_at.map(|hb| (reference - hb).num_seconds().max(0));
        let age_text = match age {
            Some(a) => fmt_age(a),
            None => "—".into(),
        };
        feed_item(ui, &p, feed_health(age).color(&p), feed.source.short(), &age_text).on_hover_text(format!(
            "{}: {} events, {} duplicates dropped, last event {}",
            feed.source.as_str(),
            feed.events_total,
            feed.duplicates_dropped,
            feed.last_event_at.map(fmt_local).unwrap_or_else(|| "never".into())
        ));
    }
}

/// Inside the right-to-left cluster `horizontal` lays its children out right to left too, so the
/// pieces are added in reverse there to read dot, name, age.
fn feed_item(ui: &mut egui::Ui, p: &Palette, color: Color32, name: &str, age: &str) -> egui::Response {
    let rtl = ui.layout().main_dir() == egui::Direction::RightToLeft;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let name_text = RichText::new(name).family(medium()).size(11.0).color(p.text_2);
        let age_text = RichText::new(age).size(11.0).color(p.muted);
        if rtl {
            ui.label(age_text);
            ui.label(name_text);
            dot(ui, color);
        } else {
            dot(ui, color);
            ui.label(name_text);
            ui.label(age_text);
        }
    })
    .response
}
