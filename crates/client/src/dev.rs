//! Development-only helpers, compiled into debug builds alone: an automatic login and a window
//! snapshot, so the layout can be checked from a script without a person at the keyboard.
//!
//! `TREASURY_DEV_LOGIN=user:password` logs in at start-up.
//! `TREASURY_SNAPSHOT=/path/shot.png` saves the window as a PNG once the main screen has been up
//! for `TREASURY_SNAPSHOT_AFTER` seconds (default 4) and then closes the window.
//! `TREASURY_SNAPSHOT_TAB=overview|blotter|breaks|ladder|alm`, `TREASURY_SNAPSHOT_THEME=light|dark`
//! `TREASURY_SNAPSHOT_BELL=1` (the attention popup open), `TREASURY_SNAPSHOT_SCREEN=login` (the
//! sign-in screen, without logging in), `TREASURY_SNAPSHOT_SIZE=1360x820` and
//! `TREASURY_SNAPSHOT_ZOOM=0.5` (to counter a scaled display) shape what is captured.

use std::time::{Duration, Instant};

use crate::app::Tab;

pub struct DevSnapshot {
    pub path: String,
    pub after: Duration,
    pub tab: Option<Tab>,
    pub theme: Option<egui::ThemePreference>,
    pub bell: bool,
    /// Capture the login screen instead of waiting for a session.
    pub login: bool,
    pub size: Option<(f32, f32)>,
    pub zoom: Option<f32>,
    pub started: Option<Instant>,
    pub requested: bool,
}

pub struct Dev {
    pub login: Option<(String, String)>,
    pub snapshot: Option<DevSnapshot>,
}

pub fn from_env() -> Dev {
    let login = std::env::var("TREASURY_DEV_LOGIN").ok().and_then(|v| v.split_once(':').map(|(u, p)| (u.to_string(), p.to_string())));
    let snapshot = std::env::var("TREASURY_SNAPSHOT").ok().map(|path| DevSnapshot {
        path,
        after: Duration::from_secs(std::env::var("TREASURY_SNAPSHOT_AFTER").ok().and_then(|s| s.parse().ok()).unwrap_or(4)),
        tab: std::env::var("TREASURY_SNAPSHOT_TAB").ok().and_then(|t| match t.as_str() {
            "overview" => Some(Tab::Overview),
            "blotter" => Some(Tab::Blotter),
            "breaks" => Some(Tab::Breaks),
            "ladder" => Some(Tab::Ladder),
            "alm" => Some(Tab::Alm),
            _ => None,
        }),
        theme: std::env::var("TREASURY_SNAPSHOT_THEME").ok().and_then(|t| match t.as_str() {
            "light" => Some(egui::ThemePreference::Light),
            "dark" => Some(egui::ThemePreference::Dark),
            _ => None,
        }),
        bell: std::env::var("TREASURY_SNAPSHOT_BELL").is_ok_and(|v| v == "1"),
        login: std::env::var("TREASURY_SNAPSHOT_SCREEN").is_ok_and(|v| v == "login"),
        zoom: std::env::var("TREASURY_SNAPSHOT_ZOOM").ok().and_then(|v| v.parse().ok()),
        size: std::env::var("TREASURY_SNAPSHOT_SIZE").ok().and_then(|v| v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))),
        started: None,
        requested: false,
    });
    Dev { login, snapshot }
}

/// Ask egui for the rendered frame, write it, then close the window.
pub fn capture(ctx: &egui::Context, path: String) {
    let ctx2 = ctx.clone();
    ctx.request_screenshot(move |img| {
        let [w, h] = img.size;
        match image::save_buffer(&path, img.as_raw(), w as u32, h as u32, image::ExtendedColorType::Rgba8) {
            Ok(()) => tracing::info!(%path, w, h, "snapshot written"),
            Err(e) => tracing::error!(%path, error = %e, "snapshot failed"),
        }
        ctx2.send_viewport_cmd(egui::ViewportCommand::Close);
    });
    ctx.request_repaint();
}
