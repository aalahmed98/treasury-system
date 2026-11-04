//! NBB Treasury desktop shell. A small Windows application that opens the treasury web client
//! from the bank's server in a window of its own, with an icon in the taskbar, a tray entry and a
//! remembered window. The pages themselves come from the server, so every desk sees an update the
//! moment the server has it; nothing is bundled but the first-run page that asks for the address.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs;
use std::path::PathBuf;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const FILE: &str = "server.txt";

fn config_file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join(FILE))
}

/// The server to open: the `--server` argument, then `TREASURY_SERVER`, then the remembered one.
fn configured_server(app: &AppHandle) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--server") {
        if let Some(u) = args.get(i + 1) {
            return normalise(u);
        }
    }
    if let Ok(u) = std::env::var("TREASURY_SERVER") {
        if let Some(n) = normalise(&u) {
            return Some(n);
        }
    }
    let path = config_file(app)?;
    fs::read_to_string(path).ok().and_then(|s| normalise(s.trim()))
}

/// `https://host[:port]` with any path dropped; plain http only for loopback addresses.
fn normalise(input: &str) -> Option<String> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    let with_scheme = if s.contains("://") { s.to_string() } else { format!("https://{s}") };
    let u = url::Url::parse(&with_scheme).ok()?;
    let host = u.host_str()?;
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]");
    if u.scheme() == "http" && !loopback {
        return None;
    }
    if u.scheme() != "http" && u.scheme() != "https" {
        return None;
    }
    let mut out = format!("{}://{}", u.scheme(), host);
    if let Some(p) = u.port() {
        out.push_str(&format!(":{p}"));
    }
    Some(out)
}

fn app_url(server: &str) -> url::Url {
    url::Url::parse(&format!("{server}/app/")).expect("normalised server url")
}

/// Called by the first-run page: remember the server and open the client.
#[tauri::command]
fn connect(app: AppHandle, url: String) -> Result<(), String> {
    let server = normalise(&url).ok_or_else(|| "enter the server address as https://host or http://localhost:8787 for a local server".to_string())?;
    if let Some(path) = config_file(&app) {
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        fs::write(&path, &server).map_err(|e| format!("could not remember the server: {e}"))?;
    }
    if let Some(w) = app.get_webview_window("main") {
        w.navigate(app_url(&server)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Forget the remembered server and show the first-run page again.
#[tauri::command]
fn forget(app: AppHandle) -> Result<(), String> {
    if let Some(path) = config_file(&app) {
        let _ = fs::remove_file(path);
    }
    if let Some(w) = app.get_webview_window("main") {
        w.navigate(url::Url::parse("tauri://localhost/index.html").map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .invoke_handler(tauri::generate_handler![connect, forget])
        .setup(|app| {
            let handle = app.handle().clone();
            let target = match configured_server(&handle) {
                Some(server) => WebviewUrl::External(app_url(&server)),
                None => WebviewUrl::App("index.html".into()),
            };
            WebviewWindowBuilder::new(app, "main", target)
                .title("NBB Treasury")
                .inner_size(1360.0, 820.0)
                .min_inner_size(960.0, 600.0)
                .build()?;

            let open = MenuItem::with_id(app, "open", "Open NBB Treasury", true, None::<&str>)?;
            let change = MenuItem::with_id(app, "forget", "Change server…", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &change, &quit])?;
            let mut tray = TrayIconBuilder::with_id("main").menu(&menu).tooltip("NBB Treasury").show_menu_on_left_click(false);
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.on_menu_event(|app, event| match event.id().as_ref() {
                "open" => {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.show();
                        let _ = w.unminimize();
                        let _ = w.set_focus();
                    }
                }
                "forget" => {
                    let _ = forget(app.clone());
                }
                "quit" => app.exit(0),
                _ => {}
            })
            .build(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the NBB Treasury shell");
}
