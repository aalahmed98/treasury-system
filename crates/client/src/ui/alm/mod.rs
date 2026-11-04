//! The ALM workbench tab: a tool strip grouped by purpose, the snapshot in use, and one tool at a
//! time with its assumptions on the left and its results on the right.

pub mod attribution;
pub mod common;
pub mod dashboard;
pub mod funding;
pub mod gap;
pub mod ird;
pub mod lcr;
pub mod nii;
pub mod nsfr;
pub mod setup;
pub mod stress;

use egui::{Align, Layout, Margin, Panel, RichText};

use crate::alm_state::AlmTool;
use crate::app::App;
use crate::ui::theme::{Palette, pal};
use crate::ui::widgets::{canvas_frame, chip};

const GROUPS: [(&str, &[AlmTool]); 4] = [
    ("", &[AlmTool::Dashboard]),
    ("Liquidity", &[AlmTool::Lcr, AlmTool::Nsfr, AlmTool::Funding]),
    ("Rates", &[AlmTool::Gap, AlmTool::Nii, AlmTool::Stress, AlmTool::Ird]),
    ("Income", &[AlmTool::Attribution]),
];

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    Panel::top("alm_strip")
        .frame(egui::Frame::new().fill(p.surface).inner_margin(Margin { left: 16, right: 16, top: 7, bottom: 7 }))
        .show(ui, |ui| strip(app, ui, &p));

    if app.alm.bank.is_none() {
        egui::CentralPanel::default().frame(canvas_frame(&p)).show(ui, |ui| {
            if let Some(e) = app.alm.params.error.as_ref().or(app.alm.snapshots.error.as_ref()) {
                ui.label(RichText::new(e).color(p.bad));
            } else {
                ui.label(p.muted("loading the bank's parameters…"));
            }
        });
        return;
    }
    match app.alm.tool {
        AlmTool::Dashboard => dashboard::show(app, ui),
        AlmTool::Lcr => lcr::show(app, ui),
        AlmTool::Nsfr => nsfr::show(app, ui),
        AlmTool::Nii => nii::show(app, ui),
        AlmTool::Stress => stress::show(app, ui),
        AlmTool::Attribution => attribution::show(app, ui),
        AlmTool::Ird => ird::show(app, ui),
        AlmTool::Gap => gap::show(app, ui),
        AlmTool::Funding => funding::show(app, ui),
        AlmTool::Setup => setup::show(app, ui),
    }
}

fn strip(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for (gi, (name, tools)) in GROUPS.iter().enumerate() {
            if gi > 0 {
                ui.add_space(6.0);
                ui.add(egui::Separator::default().vertical().spacing(6.0).shrink(4.0));
                ui.add_space(4.0);
                ui.label(p.eyebrow(*name));
                ui.add_space(2.0);
            }
            for tool in tools.iter() {
                if chip(ui, p, app.alm.tool == *tool, tool.label()).clicked() {
                    app.open_alm_tool(*tool);
                }
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if chip(ui, p, app.alm.tool == AlmTool::Setup, "Setup").clicked() {
                app.open_alm_tool(AlmTool::Setup);
            }
            ui.add(egui::Separator::default().vertical().spacing(6.0).shrink(4.0));
            snapshot_picker(app, ui);
            if app.alm.params.loading || app.alm.snapshots.loading {
                ui.add(egui::Spinner::new().size(14.0));
            }
            if let Some(e) = app.alm.params.error.as_ref().or(app.alm.snapshots.error.as_ref()) {
                ui.label(RichText::new(e).small().color(p.bad));
            }
        });
    });
}

fn snapshot_picker(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    let snaps = app.alm.snapshots.view.clone().unwrap_or_default();
    let label = app.alm.as_of.map(|d| d.format("%d %b %Y").to_string()).unwrap_or_else(|| "no snapshot".into());
    let mut pick = None;
    egui::ComboBox::from_id_salt("alm_as_of").selected_text(label).width(150.0).show_ui(ui, |ui| {
        for s in &snaps {
            if ui.selectable_label(app.alm.as_of == Some(s.as_of), format!("{}  ({} cohorts)", s.as_of.format("%d %b %Y"), s.positions)).clicked() {
                pick = Some(s.as_of);
            }
        }
    });
    if let Some(d) = pick {
        app.alm.as_of = Some(d);
        app.alm.touch();
    }
    ui.label(p.small("snapshot"));
}
