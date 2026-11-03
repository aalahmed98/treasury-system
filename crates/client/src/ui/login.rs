use egui::{Align, Key, Layout, RichText, TextEdit};

use crate::app::App;
use crate::ui::RED;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let enter = ui.input(|i| i.key_pressed(Key::Enter));

    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.add_space(ui.available_height() * 0.22);
        ui.heading(RichText::new("NBB Treasury").size(28.0).strong());
        ui.label(RichText::new("Position keeping and intraday liquidity").color(crate::ui::MUTED));
        ui.add_space(24.0);

        egui::Frame::group(ui.style()).inner_margin(20.0).show(ui, |ui| {
            ui.set_width(360.0);
            egui::Grid::new("login_grid").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                ui.label("Server");
                ui.add(TextEdit::singleline(&mut app.server).desired_width(240.0));
                ui.end_row();

                ui.label("Username");
                let u = ui.add(TextEdit::singleline(&mut app.username).desired_width(240.0));
                if app.username.is_empty() && app.user.is_none() {
                    u.request_focus();
                }
                ui.end_row();

                ui.label("Password");
                ui.add(TextEdit::singleline(&mut app.password).password(true).desired_width(240.0));
                ui.end_row();
            });

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let ready = !app.logging_in && !app.username.trim().is_empty() && !app.password.is_empty();
                if ui.add_enabled(ready, egui::Button::new("Log in")).clicked() || (ready && enter) {
                    app.submit_login();
                }
                if app.logging_in {
                    ui.spinner();
                    ui.label("Signing in…");
                }
            });
            if let Some(err) = &app.login_error {
                ui.add_space(6.0);
                ui.label(RichText::new(err).color(RED));
            }
        });

        ui.add_space(16.0);
        ui.label(RichText::new("Dev users: fx1, fx2, mm1, alm1, head1  ·  password: password").color(crate::ui::MUTED).small());
    });
}
