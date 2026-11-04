//! The bank's parameter set: the active version, its history, and an editor that saves a new
//! version with a note. Any ALM user may save; the server validates and audits.

use egui::{Align, Layout, RichText};

use crate::alm_state::{AlmState, SetupState};
use crate::app::App;
use crate::net::Command;
use crate::ui::alm::common::{bp, pct};
use crate::ui::fmt::fmt_local;
use crate::ui::theme::{Palette, pal};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    let Some(resp) = app.alm.params.view.as_ref() else {
        ui.label(RichText::new("loading…").color(p.muted));
        return;
    };
    let active = resp.active.clone();
    let history = resp.history.clone();
    ui.horizontal(|ui| {
        ui.strong(format!("Active parameters: version {}", active.version));
        ui.label(RichText::new(format!("saved {} by {}", fmt_local(active.created_at), active.created_by.as_deref().unwrap_or("the application"))).color(p.muted));
        if let Some(n) = &active.note {
            ui.label(RichText::new(n).color(p.muted));
        }
    });
    ui.add_space(6.0);
    egui::Panel::right("alm_setup_history").resizable(true).default_size(360.0).min_size(240.0).show(ui, |ui| {
        ui.label(RichText::new("Versions").strong());
        egui::ScrollArea::vertical().id_salt("alm_setup_versions").show(ui, |ui| {
            for v in &history {
                ui.horizontal_wrapped(|ui| {
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
        });
    });
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("alm_setup_editor").show(ui, |ui| editor(app, ui, &p));
    });
}

fn editor(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let AlmState { setup, bank, overrides, .. } = &mut app.alm;
    let Some(bank) = bank.clone() else { return };
    ui.horizontal(|ui| {
        ui.strong("New version");
        if ui.small_button("Start from the active version").clicked() {
            setup.draft = Some((*bank).clone());
        }
        if overrides.is_some() && ui.small_button("Start from the current what-if").clicked() {
            setup.draft = overrides.clone();
        }
        if setup.draft.is_some() && ui.small_button("Discard").clicked() {
            setup.draft = None;
            setup.saving = Default::default();
        }
    });
    let Some(draft) = &mut setup.draft else {
        ui.label(RichText::new("Start a draft from the active version or from the what-if you have been editing on the tools; every number below is then editable. Saving creates a new audited version that every open client picks up.").color(p.muted).small());
        return;
    };
    let mut req = None;
    ui.horizontal(|ui| {
        ui.label("Note");
        ui.add(egui::TextEdit::singleline(&mut setup.note).desired_width(380.0).hint_text("why this version"));
        let can = !setup.note.trim().is_empty() && !setup.saving.loading;
        if ui.add_enabled(can, egui::Button::new("Save as a new version")).clicked() {
            req = Some(());
        }
        if setup.saving.loading {
            ui.spinner();
        }
        if let Some(e) = &setup.saving.error {
            ui.colored_label(p.negative, e);
        }
        if let Some(v) = &setup.saving.view {
            ui.colored_label(p.positive, format!("saved as version {}", v.version));
        }
    });
    ui.add_space(6.0);
    draft_editor(draft, ui, p);
    if req.is_some() {
        let body = Box::new(domain::SaveParamsRequest { params: draft.clone(), note: setup.note.trim().to_string() });
        let r = app.issue();
        app.alm.setup.saving.begin(r);
        app.net.send(Command::SaveAlmParams { req: r, body });
    }
}

fn section(ui: &mut egui::Ui, p: &Palette, title: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(title).color(p.muted).small().strong());
}

fn draft_editor(d: &mut domain::AlmParams, ui: &mut egui::Ui, p: &Palette) {
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        section(ui, p, "General");
        egui::Grid::new("setup_general").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
            ui.label("Reporting currency");
            ui.add(egui::TextEdit::singleline(&mut d.reporting_ccy).desired_width(60.0));
            ui.end_row();
            ui.label("Tier 1 capital");
            ui.add(egui::DragValue::new(&mut d.tier1_capital).range(1.0..=1e12).speed(1e6).fixed_decimals(0));
            ui.end_row();
        });
        section(ui, p, "LCR thresholds, haircuts and caps");
        egui::Grid::new("setup_lcr").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
            for (label, v) in [("Regulatory minimum", &mut d.lcr.minimum), ("Internal minimum", &mut d.lcr.internal_minimum), ("Early warning", &mut d.lcr.early_warning)] {
                ui.label(label);
                pct(ui, v, 500.0);
                ui.end_row();
            }
            for level in ["L1", "L2A", "L2B"] {
                ui.label(format!("{level} haircut"));
                pct(ui, d.lcr.haircuts.entry(level.to_string()).or_insert(0.0), 100.0);
                ui.end_row();
            }
            ui.label("Level 2 cap");
            pct(ui, &mut d.lcr.cap_l2_total_pct, 99.0);
            ui.end_row();
            ui.label("Level 2B cap");
            pct(ui, &mut d.lcr.cap_l2b_pct, 99.0);
            ui.end_row();
            ui.label("Inflow cap");
            pct(ui, &mut d.lcr.inflow_cap_pct, 100.0);
            ui.end_row();
        });
        section(ui, p, "LCR run-off and inflow rates");
        egui::Grid::new("setup_lcr_rates").num_columns(2).spacing([12.0, 3.0]).striped(true).show(ui, |ui| {
            for (k, v) in d.lcr.outflow_rates.iter_mut().chain(d.lcr.inflow_rates.iter_mut()) {
                ui.label(k.replace('_', " ").to_lowercase());
                pct(ui, v, 100.0);
                ui.end_row();
            }
        });
        section(ui, p, "Funding concentration limits");
        egui::Grid::new("setup_funding").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
            for (label, v) in [("Largest depositor", &mut d.funding.top1_limit_pct), ("Ten largest depositors", &mut d.funding.top10_limit_pct), ("Wholesale funding", &mut d.funding.wholesale_share_limit_pct)] {
                ui.label(label);
                pct(ui, v, 100.0);
                ui.end_row();
            }
        });

        let ui = &mut cols[1];
        section(ui, p, "NSFR thresholds and factors (< 6M · 6M-1Y · ≥ 1Y)");
        egui::Grid::new("setup_nsfr_t").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
            ui.label("Regulatory minimum");
            pct(ui, &mut d.nsfr.minimum, 500.0);
            ui.end_row();
            ui.label("Internal minimum");
            pct(ui, &mut d.nsfr.internal_minimum, 500.0);
            ui.end_row();
        });
        egui::Grid::new("setup_nsfr").num_columns(4).spacing([8.0, 3.0]).striped(true).show(ui, |ui| {
            for (k, f) in d.nsfr.asf.iter_mut().chain(d.nsfr.rsf.iter_mut()) {
                ui.label(k.replace('_', " ").to_lowercase());
                for v in f.iter_mut() {
                    pct(ui, v, 100.0);
                }
                ui.end_row();
            }
        });
        section(ui, p, "Gap, EVE and NII limits");
        egui::Grid::new("setup_gap").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
            for (k, v) in d.gap.cumulative_gap_limit_pct_assets.iter_mut() {
                ui.label(format!("Cumulative gap {k} (% assets)"));
                pct(ui, v, 100.0);
                ui.end_row();
            }
            ui.label("EVE loss (% of Tier 1)");
            pct(ui, &mut d.gap.eve_limit_pct_tier1, 100.0);
            ui.end_row();
            ui.label("NII sensitivity (%)");
            pct(ui, &mut d.gap.nii_sensitivity_limit_pct, 100.0);
            ui.end_row();
            for (k, v) in d.gap.nmd_behavioural_months.iter_mut() {
                ui.label(format!("{} behavioural months", k.replace('_', " ").to_lowercase()));
                ui.add(egui::DragValue::new(v).range(1.0..=240.0).speed(1.0).fixed_decimals(0));
                ui.end_row();
            }
        });
        section(ui, p, "Stress shocks (parallel · short · long, basis points)");
        egui::Grid::new("setup_shocks").num_columns(4).spacing([8.0, 3.0]).striped(true).show(ui, |ui| {
            for (ccy, s) in d.stress.shocks.iter_mut() {
                ui.label(ccy.as_str());
                for v in [&mut s.parallel, &mut s.short, &mut s.long] {
                    ui.add(egui::DragValue::new(v).range(0.0..=2000.0).speed(5.0).fixed_decimals(0));
                }
                ui.end_row();
            }
        });
        egui::Grid::new("setup_stress2").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
            ui.label("Post-shock floor");
            bp(ui, &mut d.stress.floor_bp);
            ui.end_row();
            ui.label("Floor rises per year (bp)");
            ui.add(egui::DragValue::new(&mut d.stress.floor_slope_bp_per_year).range(0.0..=50.0).speed(1.0).fixed_decimals(0));
            ui.end_row();
            ui.label("Liquidity run-off multiplier");
            ui.add(egui::DragValue::new(&mut d.stress.liquidity.outflow_multiplier).range(1.0..=5.0).speed(0.05).fixed_decimals(2));
            ui.end_row();
            ui.label("Extra HQLA haircut");
            pct(ui, &mut d.stress.liquidity.extra_hqla_haircut_pct, 100.0);
            ui.end_row();
            ui.label("Inflow multiplier");
            ui.add(egui::DragValue::new(&mut d.stress.liquidity.inflow_multiplier).range(0.0..=1.0).speed(0.05).fixed_decimals(2));
            ui.end_row();
            for (ccy, v) in d.stress.fx_shock_pct.iter_mut() {
                ui.label(format!("FX shock {ccy}"));
                pct(ui, v, 100.0);
                ui.end_row();
            }
            ui.label("NII horizon (months)");
            ui.add(egui::DragValue::new(&mut d.nii.horizon_months).range(1..=60).speed(1.0));
            ui.end_row();
            ui.label("Default deposit beta");
            let mut beta = d.nii.default_beta * 100.0;
            if pct(ui, &mut beta, 100.0) {
                d.nii.default_beta = beta / 100.0;
            }
            ui.end_row();
        });
        ui.with_layout(Layout::left_to_right(Align::Min), |_| {});
    });
}

#[allow(dead_code)]
fn unused(_: &SetupState) {}
