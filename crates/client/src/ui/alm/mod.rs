//! The ALM workbench tab: a tool strip, the snapshot in use, and one tool at a time with its
//! assumptions on the left and its results on the right.

pub mod lcr;
pub mod setup;

use egui::{Align, Layout, RichText};

use crate::alm_state::AlmTool;
use crate::app::App;
use crate::ui::theme::pal;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    ui.horizontal(|ui| {
        ui.heading("ALM");
        ui.separator();
        for tool in AlmTool::ALL {
            if ui.selectable_label(app.alm.tool == tool, tool.label()).clicked() {
                app.alm.tool = tool;
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            snapshot_picker(app, ui);
            if app.alm.params.loading || app.alm.snapshots.loading {
                ui.spinner();
            }
        });
    });
    if let Some(e) = app.alm.params.error.as_ref().or(app.alm.snapshots.error.as_ref()) {
        ui.colored_label(p.negative, e);
    }
    ui.separator();

    if app.alm.bank.is_none() {
        ui.label(RichText::new("loading the bank's parameters…").color(p.muted));
        return;
    }
    match app.alm.tool {
        AlmTool::Lcr => lcr::show(app, ui),
        AlmTool::Setup => setup::show(app, ui),
        other => {
            ui.add_space(8.0);
            ui.label(RichText::new(format!("{} arrives with the next milestones.", other.label())).color(p.muted));
        }
    }
}

fn snapshot_picker(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    let snaps = app.alm.snapshots.view.clone().unwrap_or_default();
    let label = app.alm.as_of.map(|d| d.format("%d %b %Y").to_string()).unwrap_or_else(|| "no snapshot".into());
    let mut pick = None;
    egui::ComboBox::from_id_salt("alm_as_of").selected_text(format!("as of {label}")).width(170.0).show_ui(ui, |ui| {
        for s in &snaps {
            if ui.selectable_label(app.alm.as_of == Some(s.as_of), format!("{}  ({} cohorts)", s.as_of.format("%d %b %Y"), s.positions)).clicked() {
                pick = Some(s.as_of);
            }
        }
    });
    if let Some(d) = pick {
        app.alm.as_of = Some(d);
        app.alm.mark_dirty();
    }
    ui.label(RichText::new("snapshot").color(p.muted).small());
}
