use chrono::Local;
use egui::{Align, Layout, RichText};

use crate::app::App;
use crate::net::StreamState;
use crate::ui::fmt::fmt_local;
use crate::ui::theme::pal;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let p = pal(ui);
        let (color, text) = match app.stream {
            StreamState::Connected => (p.positive, "live"),
            StreamState::Connecting => (p.warning, "connecting"),
            StreamState::Disconnected => (p.negative, "offline"),
        };
        ui.colored_label(color, "●");
        ui.label(text);

        if let Some(t) = app.last_event {
            ui.label(RichText::new(format!("last update {}", t.with_timezone(&Local).format("%H:%M:%S"))).color(p.muted));
        }
        if let Some(as_of) = app.store.as_of {
            ui.label(RichText::new(format!("snapshot {}", fmt_local(as_of))).color(p.muted)).on_hover_text("server time of the last full load");
        }
        if let Some((_, msg)) = &app.status_notice {
            ui.separator();
            ui.colored_label(p.warning, msg);
        }
        if let Some((at, msg)) = &app.last_error {
            ui.separator();
            ui.colored_label(p.negative, format!("{} {}", at.with_timezone(&Local).format("%H:%M:%S"), msg));
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Refresh").clicked() {
                app.net.send(crate::net::Command::Refresh { passive: false });
            }
            ui.label(RichText::new(&app.server_url).color(p.muted).small());
        });
    });
}
