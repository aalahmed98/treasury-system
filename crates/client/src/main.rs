//! NBB Treasury desktop client: native egui window over the treasury server's HTTP + SSE API.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod net;
mod state;
mod ui;

use tracing_subscriber::EnvFilter;

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,wgpu=warn,naga=warn".into()))
        .init();

    // reqwest is built without a bundled crypto backend (avoids CMake/NASM on Windows); use ring.
    if rustls::crypto::ring::default_provider().install_default().is_err() {
        tracing::warn!("a TLS crypto provider was already installed");
    }

    let server = std::env::args()
        .skip(1)
        .find(|a| a.starts_with("http"))
        .or_else(|| std::env::var("TREASURY_SERVER").ok())
        .unwrap_or_else(|| "http://localhost:8787".to_string());

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("NBB Treasury")
            .with_inner_size([1360.0, 820.0])
            .with_min_inner_size([960.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native("NBB Treasury", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, server)))))
}
