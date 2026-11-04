//! The bank's parameter set: which version is active, who saved it and why, and what it holds.
//! Read-only until the save flow lands.

use egui::RichText;

use crate::app::App;
use crate::ui::fmt::fmt_local;
use crate::ui::theme::pal;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    let Some(resp) = &app.alm.params.view else {
        ui.label(RichText::new("loading…").color(p.muted));
        return;
    };
    ui.horizontal(|ui| {
        ui.strong(format!("Active parameters: version {}", resp.active.version));
        ui.label(RichText::new(format!("saved {} by {}", fmt_local(resp.active.created_at), resp.active.created_by.as_deref().unwrap_or("the application"))).color(p.muted));
    });
    if let Some(n) = &resp.active.note {
        ui.label(RichText::new(n).color(p.muted));
    }
    ui.add_space(6.0);
    ui.columns(2, |cols| {
        cols[0].label(RichText::new("Versions").strong());
        for v in &resp.history {
            cols[0].horizontal(|ui| {
                ui.monospace(format!("v{}", v.version));
                if v.is_active {
                    ui.colored_label(p.positive, "active");
                }
                ui.label(RichText::new(fmt_local(v.created_at)).color(p.muted).small());
                ui.label(RichText::new(v.created_by.as_deref().unwrap_or("application")).color(p.muted).small());
                if let Some(n) = &v.note {
                    ui.label(RichText::new(n).small());
                }
            });
        }
        cols[1].label(RichText::new("Document").strong());
        let text = serde_json::to_string_pretty(&resp.params).unwrap_or_default();
        egui::ScrollArea::vertical().id_salt("alm_setup_doc").max_height(520.0).show(&mut cols[1], |ui| {
            ui.monospace(RichText::new(text).small());
        });
    });
    ui.add_space(6.0);
    ui.label(RichText::new("Editing and saving a new version arrives with a later milestone; what-ifs are made on each tool's assumptions panel.").color(p.muted).small());
}
