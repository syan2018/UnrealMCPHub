use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn mcphub_manifest_path() -> PathBuf {
    repo_root().join("vendor").join("MCPHub").join("Cargo.toml")
}

pub fn mcphub_binary_path() -> PathBuf {
    let exe = if cfg!(windows) {
        "mcphub.exe"
    } else {
        "mcphub"
    };
    repo_root()
        .join("vendor")
        .join("MCPHub")
        .join("target")
        .join("debug")
        .join(exe)
}

pub fn ensure_mcphub_built() -> Result<PathBuf> {
    let manifest = mcphub_manifest_path();
    if !manifest.is_file() {
        bail!("MCPHub submodule is missing at {}", manifest.display());
    }

    let binary = mcphub_binary_path();
    if binary.is_file() {
        return Ok(binary);
    }

    let status = Command::new("cargo")
        .arg("build")
        .arg("--manifest-path")
        .arg(&manifest)
        .status()
        .context("failed to build MCPHub submodule")?;
    if !status.success() {
        bail!("building MCPHub submodule failed with status {status}");
    }
    Ok(binary)
}

pub fn sync_endpoint_with_mcphub(
    endpoint_id: &str,
    endpoint_url: &str,
    endpoint_name: &str,
) -> Result<String> {
    let binary = ensure_mcphub_built()?;
    let register = Command::new(&binary)
        .arg("register-http")
        .arg(endpoint_id)
        .arg(endpoint_url)
        .arg("--name")
        .arg(endpoint_name)
        .output()
        .context("failed to run bundled MCPHub")?;
    if !register.status.success() {
        bail!(
            "mcphub register-http failed: {}",
            String::from_utf8_lossy(&register.stderr)
        );
    }

    let discover = Command::new(&binary)
        .arg("discover")
        .arg(endpoint_id)
        .output()
        .context("failed to refresh bundled MCPHub catalog")?;
    if !discover.status.success() {
        bail!(
            "mcphub discover failed: {}",
            String::from_utf8_lossy(&discover.stderr)
        );
    }

    Ok(format!(
        "{}\n{}",
        String::from_utf8_lossy(&register.stdout).trim(),
        String::from_utf8_lossy(&discover.stdout).trim()
    ))
}
