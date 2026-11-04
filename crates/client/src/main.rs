//! NBB Treasury desktop client: native egui window over the treasury server's HTTP + SSE API.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod alm_state;
mod app;
mod attention;
#[cfg(debug_assertions)]
mod dev;
mod net;
mod state;
mod ui;

use tracing_subscriber::EnvFilter;

const DEFAULT_SERVER: &str = "http://localhost:8787";

/// Where the client sends credentials. Fixed at start-up from the command line or environment,
/// never from a field the user can edit, so nobody can be talked into pointing the client at a
/// hostile server.
pub struct ServerTarget {
    pub url: String,
    pub ca: Option<reqwest::Certificate>,
    /// Why this target is unusable, if it is. Login stays disabled while set.
    pub problem: Option<String>,
}

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,wgpu=warn,naga=warn".into()))
        .init();

    // reqwest is built without a bundled crypto backend (avoids CMake/NASM on Windows); use ring.
    if rustls::crypto::ring::default_provider().install_default().is_err() {
        tracing::warn!("a TLS crypto provider was already installed");
    }

    let target = resolve_target();
    if let Some(p) = &target.problem {
        tracing::error!(server = %target.url, "{p}");
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("NBB Treasury")
            .with_app_id("nbb-treasury")
            .with_inner_size([1360.0, 820.0])
            .with_min_inner_size([960.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native("NBB Treasury", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, target)))))
}

/// `treasury-client [https://host:port] [--ca path.pem]`, or TREASURY_SERVER / TREASURY_CA.
fn resolve_target() -> ServerTarget {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut url = std::env::var("TREASURY_SERVER").ok();
    let mut ca_path = std::env::var("TREASURY_CA").ok();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--ca" => {
                ca_path = args.get(i + 1).cloned();
                i += 2;
            }
            a if a.starts_with("http") => {
                url = Some(a.to_string());
                i += 1;
            }
            _ => i += 1,
        }
    }
    let url = url.unwrap_or_else(|| DEFAULT_SERVER.to_string());
    let allow_insecure = std::env::var("TREASURY_ALLOW_INSECURE").map(|v| v == "1").unwrap_or(false);
    let mut problem = check_url(&url, allow_insecure).err();

    let ca = ca_path.and_then(|p| {
        let loaded = std::fs::read(&p)
            .map_err(|e| e.to_string())
            .and_then(|pem| reqwest::Certificate::from_pem(&pem).map_err(|e| e.to_string()));
        match loaded {
            Ok(c) => Some(c),
            Err(e) => {
                problem.get_or_insert(format!("cannot load CA certificate {p}: {e}"));
                None
            }
        }
    });
    ServerTarget { url, ca, problem }
}

/// Plain http is acceptable only to a loopback address; everything else must be https.
pub fn check_url(url: &str, allow_insecure: bool) -> Result<(), String> {
    let (scheme, rest) = url.split_once("://").ok_or("server URL must start with http:// or https://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    if host.is_empty() {
        return Err("server URL has no host".into());
    }
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "::1");
    match scheme {
        "https" => Ok(()),
        "http" if loopback || allow_insecure => Ok(()),
        "http" => Err(format!(
            "refusing to send credentials over plain http to {host}; use https:// \
             (TREASURY_ALLOW_INSECURE=1 overrides this on a throwaway development box)"
        )),
        other => Err(format!("unsupported URL scheme {other}://")),
    }
}

#[cfg(test)]
mod tests {
    use super::check_url;

    #[test]
    fn plaintext_only_to_loopback() {
        assert!(check_url("http://localhost:8787", false).is_ok());
        assert!(check_url("http://127.0.0.1:8787/", false).is_ok());
        assert!(check_url("http://[::1]:8787", false).is_ok());
        assert!(check_url("https://treasury.bank.internal", false).is_ok());
        assert!(check_url("http://treasury.bank.internal:8787", false).is_err());
        assert!(check_url("http://10.0.0.5:8787", false).is_err());
        assert!(check_url("http://10.0.0.5:8787", true).is_ok());
        assert!(check_url("ftp://x", false).is_err());
        assert!(check_url("localhost:8787", false).is_err());
    }
}
