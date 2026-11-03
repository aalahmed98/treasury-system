use chrono::Local;
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
        ui.label("Desks:");
        for d in &app.desks {
            let txt = if d.can_book { format!("{} ✎", d.code) } else { d.code.clone() };
            ui.label(txt).on_hover_text(if d.can_book { "can book" } else { "read only" });
        }
        ui.separator();

        let (color, text) = match app.stream {
            StreamState::Connected => (GREEN, "live"),
            StreamState::Connecting => (AMBER, "connecting"),
            StreamState::Disconnected => (RED, "offline"),
        };
        ui.colored_label(color, "●");
        ui.label(text);
        if let Some(t) = app.last_event {
            ui.label(RichText::new(format!("last update {}", t.with_timezone(&Local).format("%H:%M:%S"))).color(MUTED));
        }
        if let Some(as_of) = app.store.as_of {
            ui.label(RichText::new(format!("snapshot {}", fmt_local(as_of))).color(MUTED));
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
