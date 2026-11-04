//! The sign-in screen: the wordmark, one card, two fields and the primary action.

use egui::{Align, Key, Layout, Margin, RichText, TextEdit};

use crate::app::App;
use crate::ui::theme::{pal, semibold};
use crate::ui::widgets::{frame, primary_sized};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let enter = ui.input(|i| i.key_pressed(Key::Enter));
    let p = pal(ui);
    ui.painter().rect_filled(ui.max_rect(), 0.0, p.canvas);

    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.add_space(ui.available_height() * 0.18);
        ui.horizontal(|ui| {
            ui.add_space((ui.available_width() - 190.0) / 2.0);
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label(RichText::new("NBB").family(semibold()).size(30.0).color(p.brand));
            ui.add_space(8.0);
            ui.label(RichText::new("Treasury").family(semibold()).size(30.0).color(p.text));
        });
        ui.label(p.muted("Positions, reconciliation, liquidity and ALM"));
        ui.add_space(24.0);

        frame(&p).inner_margin(Margin::same(28)).corner_radius(12).show(ui, |ui| {
            ui.set_width(340.0);
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.label(p.label("Username"));
                let u = ui.add(TextEdit::singleline(&mut app.username).desired_width(f32::INFINITY).margin(Margin::symmetric(10, 7)));
                if app.username.is_empty() && app.user.is_none() && !app.logging_in {
                    u.request_focus();
                }
                ui.add_space(8.0);
                ui.label(p.label("Password"));
                ui.add(TextEdit::singleline(&mut app.password).password(true).desired_width(f32::INFINITY).margin(Margin::symmetric(10, 7)));
                ui.add_space(16.0);

                let ready = app.can_login();
                let b = primary_sized(ui, &p, ready, if app.logging_in { "Signing in…" } else { "Sign in" }, egui::vec2(ui.available_width(), 34.0));
                if b.clicked() || (ready && enter && !app.logging_in) {
                    app.submit_login();
                }
                if let Some(err) = &app.login_error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(err).color(p.bad));
                }
                if let Some(n) = &app.notice {
                    ui.add_space(6.0);
                    ui.label(RichText::new(n).color(p.warn));
                }
                ui.add_space(14.0);
                ui.separator();
                ui.add_space(4.0);
                // The server is configured, not typed: a fixed target cannot be phished.
                ui.horizontal(|ui| {
                    let secure = app.server_url.starts_with("https://");
                    ui.label(RichText::new(if secure { "🔒" } else { "⚠" }).size(11.0).color(if secure { p.muted } else { p.warn }))
                        .on_hover_text(if secure { "TLS" } else { "plain HTTP (loopback only)" });
                    ui.label(p.small(&app.server_url));
                });
                if let Some(problem) = &app.server_problem {
                    ui.label(RichText::new(problem).small().color(p.bad));
                }
            });
        });

        if cfg!(debug_assertions) {
            ui.add_space(16.0);
            ui.label(p.small("dev build · seeded users fx1, fx2, mm1, alm1, head1 · password \"password\""));
        }
    });
}
