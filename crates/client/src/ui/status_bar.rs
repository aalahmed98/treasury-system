//! The strip under everything: connection state, the last update, notices and errors.

use chrono::Local;
use egui::{Align, Layout, RichText};

use crate::app::App;
use crate::net::StreamState;
use crate::ui::fmt::fmt_local;
use crate::ui::theme::{medium, pal};
use crate::ui::widgets::dot;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let p = pal(ui);
        ui.spacing_mut().item_spacing.x = 6.0;
        let (color, text) = match app.stream {
            StreamState::Connected => (p.good, "live"),
            StreamState::Connecting => (p.warn, "connecting"),
            StreamState::Disconnected => (p.bad, "offline"),
        };
        dot(ui, color);
        ui.label(RichText::new(text).family(medium()).size(11.0).color(p.text_2));

        if let Some(t) = app.last_event {
            ui.label(p.small(format!("last update {}", t.with_timezone(&Local).format("%H:%M:%S"))));
        }
        if let Some(as_of) = app.store.as_of {
            ui.label(p.small(format!("snapshot {}", fmt_local(as_of)))).on_hover_text("server time of the last full load");
        }
        if let Some((_, msg)) = &app.status_notice {
            ui.separator();
            ui.label(RichText::new(msg).size(11.0).color(p.warn));
        }
        if let Some((at, msg)) = &app.last_error {
            ui.separator();
            ui.label(RichText::new(format!("{} {}", at.with_timezone(&Local).format("%H:%M:%S"), msg)).size(11.0).color(p.bad));
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add(egui::Button::new(RichText::new("Refresh").size(11.0).family(medium()).color(p.text_2)).frame(false)).clicked() {
                app.net.send(crate::net::Command::Refresh { passive: false });
            }
            ui.label(p.small(&app.server_url));
        });
    });
}
