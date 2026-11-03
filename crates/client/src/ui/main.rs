use egui::{CentralPanel, Panel, RichText};

use crate::app::{App, Tab};
use crate::ui::{RED, blotter, breaks, positions, status_bar};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    Panel::bottom("status_bar").show(ui, |ui| status_bar::show(app, ui));

    Panel::top("tabs").show(ui, |ui| {
        ui.horizontal(|ui| {
            let deals = format!("Blotter  ({})", app.store.len());
            if ui.selectable_label(app.tab == Tab::Blotter, deals).clicked() {
                app.tab = Tab::Blotter;
            }
            let open = app.breaks.open_count();
            let label = if open > 0 {
                RichText::new(format!("Breaks  ({open} open)")).color(RED)
            } else {
                RichText::new("Breaks  (none open)")
            };
            if ui.selectable_label(app.tab == Tab::Breaks, label).clicked() {
                app.tab = Tab::Breaks;
            }
        });
    });

    Panel::right("positions")
        .default_size(340.0)
        .min_size(240.0)
        .resizable(true)
        .show(ui, |ui| positions::show(app, ui));

    CentralPanel::default().show(ui, |ui| match app.tab {
        Tab::Blotter => blotter::show(app, ui),
        Tab::Breaks => breaks::show(app, ui),
    });
}
