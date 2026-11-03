use egui::{CentralPanel, Panel};

use crate::app::App;
use crate::ui::{blotter, positions, status_bar};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    Panel::bottom("status_bar").show(ui, |ui| status_bar::show(app, ui));

    Panel::right("positions")
        .default_size(340.0)
        .min_size(240.0)
        .resizable(true)
        .show(ui, |ui| positions::show(app, ui));

    CentralPanel::default().show(ui, |ui| blotter::show(app, ui));
}
