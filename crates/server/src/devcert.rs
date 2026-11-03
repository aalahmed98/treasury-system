//! Self-signed certificate for local development only.

use std::path::{Path, PathBuf};

use anyhow::Context;

pub fn generate(dir: &Path) -> anyhow::Result<(PathBuf, PathBuf)> {
    let mut params = rcgen::CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()])?;
    params.distinguished_name.push(rcgen::DnType::CommonName, "NBB Treasury development server");
    let key = rcgen::KeyPair::generate()?;
    let cert = params.self_signed(&key)?;

    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let cert_path = dir.join("dev-server.pem");
    let key_path = dir.join("dev-server-key.pem");
    std::fs::write(&cert_path, cert.pem())?;
    std::fs::write(&key_path, key.serialize_pem())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok((cert_path, key_path))
}
