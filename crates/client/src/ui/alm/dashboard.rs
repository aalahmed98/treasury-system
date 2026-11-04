//! The ALM dashboard: the overview block's tiles, limits and trends, for the workbench itself.

use crate::app::App;
use crate::ui::overview::alm_block;
use crate::ui::theme::pal;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    egui::ScrollArea::vertical().id_salt("alm_dashboard").show(ui, |ui| {
        if let Some(t) = alm_block(app, ui, &p) {
            app.navigate(t);
        }
    });
}
