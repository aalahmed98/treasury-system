//! Breaks tab: what the reconciler could not settle on its own, and the actions a user may take.

use domain::{BreakKind, BreakStatus, ResolveAction, ResolveRequest};
use egui::{Align, Layout, RichText, Sense};
use egui_extras::{Column, TableBuilder};
use uuid::Uuid;

use crate::app::App;
use crate::net::Command;
use crate::ui::{AMBER, GREEN, MUTED, break_color};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.heading("Breaks");
        ui.separator();
        ui.add(egui::TextEdit::singleline(&mut app.break_search).hint_text("search kind, deal, source, field…").desired_width(240.0));
        if ui.toggle_value(&mut app.show_resolved, "show resolved").changed() {
            app.net.send(Command::FetchBreaks { all: app.show_resolved, passive: false });
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut counts = std::collections::BTreeMap::<&str, usize>::new();
            for r in app.breaks.rows() {
                if r.view.status != BreakStatus::Resolved {
                    *counts.entry(r.view.kind.as_str()).or_default() += 1;
                }
            }
            for (k, n) in counts {
                ui.colored_label(break_color(k.parse().unwrap_or(BreakKind::Missing)), format!("{k} {n}"));
            }
        });
    });
    ui.separator();

    let avail = ui.available_height();
    let action_h = if app.selected_break.is_some() { 120.0 } else { 0.0 };
    let clicked = table(app, ui, avail - action_h);
    if let Some(id) = clicked {
        let next = if app.selected_break == Some(id) { None } else { Some(id) };
        app.select_break(next);
    }
    if let Some(id) = app.selected_break {
        ui.separator();
        actions(app, ui, id);
    }
}

fn table(app: &App, ui: &mut egui::Ui, height: f32) -> Option<i64> {
    let rows = app.breaks.rows();
    let filtered = &app.break_filtered;
    let flash = &app.breaks.flash;
    let selected = app.selected_break;
    let mut clicked = None;

    ui.push_id("breaks_scroll", |ui| {
        ui.set_height(height.max(100.0));
        TableBuilder::new(ui)
            .id_salt("breaks")
            .striped(true)
            .resizable(true)
            .sense(Sense::click())
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::exact(14.0))
            .column(Column::initial(60.0).at_least(50.0))     // id
            .column(Column::initial(90.0).at_least(80.0))     // kind
            .column(Column::initial(92.0).at_least(70.0))     // deal
            .column(Column::initial(120.0).at_least(80.0))    // source ref
            .column(Column::initial(90.0).at_least(60.0))     // field
            .column(Column::initial(120.0).at_least(80.0))    // expected
            .column(Column::initial(120.0).at_least(80.0))    // observed
            .column(Column::initial(44.0).at_least(40.0))     // desk
            .column(Column::initial(120.0).at_least(100.0))   // detected
            .column(Column::remainder().at_least(110.0))      // status
            .header(22.0, |mut h| {
                h.col(|_| {});
                for t in ["#", "Kind", "Deal", "Source", "Field", "Expected", "Observed", "Desk", "Detected", "Status"] {
                    h.col(|ui| { ui.strong(t); });
                }
            })
            .body(|body| {
                body.rows(19.0, filtered.len(), |mut row| {
                    let r = &rows[filtered[row.index()]];
                    let id = r.view.id;
                    row.set_selected(selected == Some(id));
                    row.col(|ui| { if flash.contains_key(&id) { ui.colored_label(GREEN, "●"); } });
                    row.col(|ui| { ui.label(RichText::new(id.to_string()).color(MUTED)); });
                    row.col(|ui| { ui.colored_label(break_color(r.view.kind), &r.kind); });
                    row.col(|ui| { ui.monospace(&r.deal_ref); });
                    row.col(|ui| { ui.monospace(&r.source); });
                    row.col(|ui| { ui.label(&r.field); });
                    row.col(|ui| { ui.monospace(&r.expected); });
                    row.col(|ui| { ui.monospace(&r.observed); });
                    row.col(|ui| { ui.label(&r.view.desk_code); });
                    row.col(|ui| { ui.label(RichText::new(&r.detected).color(MUTED)); });
                    row.col(|ui| {
                        let color = match r.view.status {
                            BreakStatus::Open => AMBER,
                            BreakStatus::Acknowledged => ui.visuals().text_color(),
                            BreakStatus::Resolved => MUTED,
                        };
                        ui.colored_label(color, &r.status);
                    });
                    if row.response().clicked() {
                        clicked = Some(id);
                    }
                });
            });
    });
    clicked
}

fn actions(app: &mut App, ui: &mut egui::Ui, id: i64) {
    let Some(r) = app.breaks.get(id) else { return };
    let view = r.view.clone();
    let can = app.can_resolve(view.desk_id);
    let resolved = view.status == BreakStatus::Resolved;

    ui.horizontal(|ui| {
        ui.colored_label(break_color(view.kind), view.kind.as_str());
        ui.label(RichText::new(format!("#{}", view.id)).color(MUTED));
        if let Some(d) = &view.deal_ref {
            ui.monospace(d);
        }
        if let Some(s) = view.source {
            ui.label(format!("{} {}", s.as_str(), view.source_ref.clone().unwrap_or_default()));
        }
        ui.label(RichText::new(match view.kind {
            BreakKind::Missing => "expected in this source by the deadline and not seen; it resolves itself when the source delivers",
            BreakKind::Mismatch => "the source disagrees with the golden record on this field; dismiss to accept the difference",
            BreakKind::Orphan => "a source record with no golden deal; link it to the deal it belongs to",
            BreakKind::Ambiguous => "several golden deals fit this record; pick the right one",
            BreakKind::Duplicate => "a second record from a source already linked to this deal; link it elsewhere or dismiss",
        })
        .color(MUTED)
        .small());
    });
    if !can {
        ui.colored_label(MUTED, "you can see this break but not act on it (no resolve right on this desk)");
    }
    if resolved {
        ui.colored_label(MUTED, format!("resolved: {}{}", view.resolution.map(|r| r.as_str().to_lowercase()).unwrap_or_default(), view.note.as_deref().map(|n| format!(" · {n}")).unwrap_or_default()));
        return;
    }

    ui.horizontal(|ui| {
        if ui.add_enabled(can && view.status == BreakStatus::Open, egui::Button::new("Acknowledge")).clicked() {
            app.net.send(Command::AckBreak(id));
        }
        ui.separator();
        ui.add(egui::TextEdit::singleline(&mut app.resolve_note).hint_text("note (required to dismiss)").desired_width(260.0));
        let note_ok = !app.resolve_note.trim().is_empty();
        if ui.add_enabled(can && note_ok, egui::Button::new("Dismiss")).on_hover_text("accept as-is; recorded with your note").clicked() {
            app.net.send(Command::ResolveBreak {
                id,
                req: ResolveRequest { action: ResolveAction::Dismiss, deal_id: None, note: Some(app.resolve_note.trim().to_string()) },
            });
        }
    });

    if matches!(view.kind, BreakKind::Orphan | BreakKind::Ambiguous | BreakKind::Duplicate) {
        ui.horizontal(|ui| {
            ui.label("Link to:");
            let mut link_to: Option<(Uuid, String)> = None;
            if let Some(cands) = view.details.get("candidates").and_then(|c| c.as_array()) {
                for c in cands.iter().take(6) {
                    let (Some(idstr), Some(r)) = (c.get("deal_id").and_then(|v| v.as_str()), c.get("deal_ref").and_then(|v| v.as_str())) else { continue };
                    let score = c.get("score").and_then(|v| v.as_i64()).unwrap_or(0);
                    if ui.add_enabled(can, egui::Button::new(format!("{r} ({score})"))).clicked() {
                        if let Ok(u) = Uuid::parse_str(idstr) {
                            link_to = Some((u, r.to_string()));
                        }
                    }
                }
            }
            ui.add(egui::TextEdit::singleline(&mut app.link_ref).hint_text("deal ref, e.g. FX-000123").desired_width(140.0));
            let typed = app.link_ref.trim().to_uppercase();
            let found = app.store.rows().iter().find(|d| d.deal_ref == typed).map(|d| (d.deal.id, d.deal_ref.clone()));
            if ui.add_enabled(can && found.is_some(), egui::Button::new("Link")).clicked() {
                link_to = found.clone();
            }
            if !typed.is_empty() && found.is_none() {
                ui.colored_label(AMBER, "no such deal in your blotter");
            }
            if let Some((deal_id, r)) = link_to {
                app.net.send(Command::ResolveBreak {
                    id,
                    req: ResolveRequest {
                        action: ResolveAction::Link,
                        deal_id: Some(deal_id),
                        note: Some(if app.resolve_note.trim().is_empty() { format!("linked to {r}") } else { app.resolve_note.trim().to_string() }),
                    },
                });
            }
        });
    }
}
