//! Small layout pieces shared by the dashboard pages: a card with a title and a stat tile. Both
//! reserve their width up front so a wrapped row knows where to break.

use egui::{Align, Layout, RichText, Stroke};

use crate::ui::theme::Palette;

pub fn frame(p: &Palette) -> egui::Frame {
    egui::Frame::new().fill(p.card).stroke(Stroke::new(1.0, p.card_stroke)).corner_radius(6).inner_margin(10)
}

/// A titled card of a fixed width; the height follows the content.
pub fn card(ui: &mut egui::Ui, p: &Palette, title: &str, width: f32, body: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(egui::vec2(width + 22.0, 10.0), Layout::top_down(Align::Min), |ui| {
        frame(p).show(ui, |ui| {
            ui.set_width(width);
            ui.label(RichText::new(title).color(p.muted).small().strong());
            ui.add_space(4.0);
            body(ui);
        });
    });
}

/// A row of cards or tiles that wraps and keeps every item's top on the same line.
pub fn wrap_row(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    ui.with_layout(Layout::left_to_right(Align::Min).with_main_wrap(true), body);
}
