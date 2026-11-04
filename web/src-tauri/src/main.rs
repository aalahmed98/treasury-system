//! NBB Treasury desktop shell. A small Windows application that opens the treasury web client
//! from the bank's server in a window of its own, with an icon in the taskbar, a tray entry and a
//! remembered window. The pages themselves come from the server, so every desk sees an update the
//! moment the server has it; nothing is bundled but the first-run page that asks for the address.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const FILE: &str = "server.txt";

/// A bank build names its server at compile time (`TREASURY_FIXED_SERVER=https://treasury.nbb.internal`
/// in the environment of `build-desktop.cmd`); the shell then never asks for an address, never
/// offers to change it, and ignores `--server` and `TREASURY_SERVER`.
const FIXED: Option<&str> = option_env!("TREASURY_FIXED_SERVER");

/// The one origin the window may navigate to; the bundled page lives on the shell's own origin.
#[derive(Clone, Default)]
struct Allowed(Arc<Mutex<Option<String>>>);

/// What the bundled page shows: nothing remembered yet, a connection under way, or why the last
/// one failed. The page asks for this when it loads and whenever the shell tells it to render.
#[derive(Clone, Default, serde::Serialize)]
struct Status {
    server: Option<String>,
    connecting: bool,
    error: Option<String>,
}

#[derive(Clone, Default)]
struct Shell {
    status: Arc<Mutex<Status>>,
    /// The bundled page's address as the webview resolved it (`http://tauri.localhost/...` on Windows).
    local: Arc<Mutex<Option<url::Url>>>,
}

impl Shell {
    fn set(&self, w: Option<&tauri::WebviewWindow>, status: Status) {
        *self.status.lock().expect("status") = status;
        if let Some(w) = w {
            let _ = w.eval("window.render && window.render()");
        }
    }
}

impl Allowed {
    fn set(&self, server: &str) {
        *self.0.lock().expect("allowed origin") = Some(server.to_string());
    }
    fn permits(&self, url: &url::Url) -> bool {
        // The bundled page: `tauri://localhost` on macOS and Linux, `http(s)://tauri.localhost` on Windows.
        if url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost") {
            return true;
        }
        let allowed = self.0.lock().expect("allowed origin").clone();
        match allowed.and_then(|s| url::Url::parse(&s).ok()) {
            Some(server) => url.origin() == server.origin(),
            None => false,
        }
    }
}

fn config_file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join(FILE))
}

/// The server to open: the `--server` argument, then `TREASURY_SERVER`, then the remembered one.
fn configured_server(app: &AppHandle) -> Option<String> {
    if let Some(fixed) = FIXED {
        return normalise(fixed);
    }
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

/// Is anything listening at the server's host and port? A plain TCP connect with a short timeout:
/// no TLS, so an internal certificate cannot produce a false alarm; a mistyped or dead address can.
fn reachable(server: &str) -> Result<(), String> {
    let u = url::Url::parse(server).map_err(|e| e.to_string())?;
    let host = u.host_str().ok_or("no host")?.to_string();
    let port = u.port_or_known_default().unwrap_or(443);
    let addrs: Vec<_> = (host.as_str(), port).to_socket_addrs().map_err(|_| format!("{host} is not a known address on this network"))?.collect();
    let mut last = String::from("no address");
    for a in addrs {
        match TcpStream::connect_timeout(&a, Duration::from_secs(3)) {
            Ok(_) => return Ok(()),
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!("nothing answers at {host}:{port} ({last})"))
}

/// Opens the server once it answers, else leaves the bundled page up with the reason, so a dead
/// address never strands the window on a browser error page.
fn open_server(app: AppHandle, allowed: Allowed, shell: Shell, server: String) {
    std::thread::spawn(move || {
        let Some(w) = app.get_webview_window("main") else { return };
        match reachable(&server) {
            Ok(()) => {
                allowed.set(&server);
                let _ = w.navigate(app_url(&server));
            }
            Err(e) => shell.set(Some(&w), Status { server: Some(server), connecting: false, error: Some(e) }),
        }
    });
}

/// The bundled page asks what to show.
#[tauri::command]
fn status(shell: tauri::State<'_, Shell>) -> Status {
    shell.status.lock().expect("status").clone()
}

/// Called by the first-run page: remember the server and open the client.
#[tauri::command]
fn connect(app: AppHandle, allowed: tauri::State<'_, Allowed>, shell: tauri::State<'_, Shell>, url: String) -> Result<(), String> {
    if FIXED.is_some() {
        return Err("this build is fixed to the bank's server".to_string());
    }
    let server = normalise(&url).ok_or_else(|| "enter the server address as https://host or http://localhost:8787 for a local server".to_string())?;
    // Only an address that answers is remembered; a wrong one is reported, not kept.
    reachable(&server).map_err(|e| format!("could not reach {server}: {e}"))?;
    allowed.set(&server);
    shell.set(None, Status { server: Some(server.clone()), connecting: true, error: None });
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
    if FIXED.is_some() {
        return Err("this build is fixed to the bank's server".to_string());
    }
    if let Some(path) = config_file(&app) {
        let _ = fs::remove_file(path);
    }
    let shell = app.state::<Shell>();
    shell.set(None, Status::default());
    let local = shell.local.lock().expect("local url").clone().ok_or("the first-run page is not available")?;
    if let Some(w) = app.get_webview_window("main") {
        w.navigate(local).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn main() {
    let allowed = Allowed::default();
    let shell = Shell::default();
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(allowed.clone())
        .manage(shell.clone())
        .invoke_handler(tauri::generate_handler![connect, forget, status])
        .setup(move |app| {
            let handle = app.handle().clone();
            let configured = configured_server(&handle);
            shell.set(None, Status { server: configured.clone(), connecting: configured.is_some(), error: None });
            let target = WebviewUrl::App("index.html".into());
            // The window shows the bank's server and the first-run page, nothing else: a link to
            // anywhere else is refused rather than followed, so no page can lead the user away.
            let gate = allowed.clone();
            let window = WebviewWindowBuilder::new(app, "main", target)
                .title("NBB Treasury")
                .inner_size(1360.0, 820.0)
                .min_inner_size(960.0, 600.0)
                .on_navigation(move |url| gate.permits(url))
                .build()?;
            if let Ok(local) = window.url() {
                *shell.local.lock().expect("local url") = Some(local);
            }
            if let Some(server) = configured {
                open_server(handle.clone(), allowed.clone(), shell.clone(), server);
            }

            let open = MenuItem::with_id(app, "open", "Open NBB Treasury", true, None::<&str>)?;
            let change = MenuItem::with_id(app, "forget", "Change server…", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = if FIXED.is_some() { Menu::with_items(app, &[&open, &quit])? } else { Menu::with_items(app, &[&open, &change, &quit])? };
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
