use chrono::{DateTime, Local, Utc};
use domain::Source;
use egui::{Align, Layout, RichText};

use crate::app::App;
use crate::net::StreamState;
use crate::state::fmt_local;
use crate::ui::{AMBER, GREEN, MUTED, RED};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if let Some(u) = &app.user {
            ui.label(RichText::new(&u.display_name).strong());
            ui.label(RichText::new(format!("({})", u.username)).color(MUTED));
        }
        ui.separator();
        for d in &app.desks {
            let mark = match (d.can_book, d.can_resolve) {
                (true, _) => " ✎",
                (false, true) => " ✓",
                _ => "",
            };
            ui.label(format!("{}{mark}", d.code))
                .on_hover_text(match (d.can_book, d.can_resolve) {
                    (true, _) => "can book and resolve breaks",
                    (false, true) => "read, can resolve breaks",
                    _ => "read only",
                });
        }
        ui.separator();

        let (color, text) = match app.stream {
            StreamState::Connected => (GREEN, "live"),
            StreamState::Connecting => (AMBER, "connecting"),
            StreamState::Disconnected => (RED, "offline"),
        };
        ui.colored_label(color, "●");
        ui.label(text);

        feeds(app, ui);

        if let Some(t) = app.last_event {
            ui.label(RichText::new(format!("last update {}", t.with_timezone(&Local).format("%H:%M:%S"))).color(MUTED));
        }
        if let Some(as_of) = app.store.as_of {
            ui.label(RichText::new(format!("snapshot {}", fmt_local(as_of))).color(MUTED)).on_hover_text("server time of the last full load");
        }
        if let Some((_, msg)) = &app.status_notice {
            ui.separator();
            ui.colored_label(AMBER, msg);
        }
        if let Some((at, msg)) = &app.last_error {
            ui.separator();
            ui.colored_label(RED, format!("{} {}", at.with_timezone(&Local).format("%H:%M:%S"), msg));
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Log out").clicked() {
                app.logout();
            }
            if ui.button("Refresh").clicked() {
                app.net.send(crate::net::Command::Refresh { passive: false });
            }
        });
    });
}

/// Per-feed age against the reconciler's clock. A quiet feed still heartbeats; a stale one does not.
fn feeds(app: &App, ui: &mut egui::Ui) {
    let Some(f) = &app.feeds else {
        ui.label(RichText::new("feeds: —").color(MUTED));
        return;
    };
    let reference: DateTime<Utc> = f.recon.as_ref().map(|r| r.now).unwrap_or(f.server_time);
    ui.separator();
    for feed in &f.feeds {
        if feed.source == Source::Internal || feed.source == Source::User {
            continue;
        }
        let (text, color) = match feed.last_heartbeat_at {
            Some(hb) => {
                let age = (reference - hb).num_seconds().max(0);
                let color = if age < 180 { GREEN } else if age < 300 { AMBER } else { RED };
                (format!("{} {}", feed.source.short(), fmt_age(age)), color)
            }
            None => (format!("{} —", feed.source.short()), MUTED),
        };
        ui.colored_label(color, text).on_hover_text(format!(
            "{}: {} events, {} duplicates dropped, last event {}",
            feed.source.as_str(),
            feed.events_total,
            feed.duplicates_dropped,
            feed.last_event_at.map(fmt_local).unwrap_or_else(|| "never".into())
        ));
    }
    match &f.recon {
        Some(r) => {
            let lag = (f.server_time - r.last_tick_at).num_seconds();
            let color = if lag < 30 { MUTED } else if lag < 120 { AMBER } else { RED };
            ui.colored_label(color, format!("recon {}{}", if r.pending > 0 { format!("pending {} · ", r.pending) } else { String::new() }, fmt_age(lag.max(0))))
                .on_hover_text(format!("reconciler clock {}, last tick {}", fmt_local(r.now), fmt_local(r.last_tick_at)));
        }
        None => {
            ui.colored_label(RED, "recon down").on_hover_text("the reconciliation worker has never reported");
        }
    }
}

fn fmt_age(secs: i64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}
