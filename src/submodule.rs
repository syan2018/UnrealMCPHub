use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "mcphub.exe"
    } else {
        "mcphub"
    }
}

fn current_exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
}

fn bundled_mcphub_binary_path() -> Option<PathBuf> {
    let candidate = current_exe_dir()?.join(exe_name());
    candidate.is_file().then_some(candidate)
}

fn source_repo_root() -> Option<PathBuf> {
    current_exe_dir()?.ancestors().find_map(|candidate| {
        let candidate = candidate.to_path_buf();
        candidate
            .join("vendor")
            .join("MCPHub")
            .join("Cargo.toml")
            .is_file()
            .then_some(candidate)
    })
}

fn source_mcphub_binary_candidates(root: &Path) -> [PathBuf; 2] {
    let bin_name = exe_name();
    [
        root.join("vendor")
            .join("MCPHub")
            .join("target")
            .join("release")
            .join(bin_name),
        root.join("vendor")
            .join("MCPHub")
            .join("target")
            .join("debug")
            .join(bin_name),
    ]
}

pub fn mcphub_manifest_path() -> PathBuf {
    bundled_mcphub_binary_path()
        .or_else(|| {
            source_repo_root().map(|root| root.join("vendor").join("MCPHub").join("Cargo.toml"))
        })
        .unwrap_or_else(|| PathBuf::from("vendor").join("MCPHub").join("Cargo.toml"))
}

pub fn mcphub_binary_path() -> PathBuf {
    if let Some(path) = bundled_mcphub_binary_path() {
        return path;
    }

    if let Some(root) = source_repo_root() {
        for candidate in source_mcphub_binary_candidates(&root) {
            if candidate.is_file() {
                return candidate;
            }
        }
        return source_mcphub_binary_candidates(&root)[0].clone();
    }

    current_exe_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(exe_name())
}

pub fn ensure_mcphub_built() -> Result<PathBuf> {
    if let Some(path) = bundled_mcphub_binary_path() {
        return Ok(path);
    }

    let Some(root) = source_repo_root() else {
        bail!("bundled MCPHub binary is missing and no MCPHub source checkout was found")
    };
    let manifest = root.join("vendor").join("MCPHub").join("Cargo.toml");
    if !manifest.is_file() {
        bail!(
            "MCPHub source checkout is missing at {}",
            manifest.display()
        );
    }

    for candidate in source_mcphub_binary_candidates(&root) {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--manifest-path")
        .arg(&manifest)
        .status()
        .context("failed to build MCPHub submodule")?;
    if !status.success() {
        bail!("building MCPHub submodule failed with status {status}");
    }

    let binary = root
        .join("vendor")
        .join("MCPHub")
        .join("target")
        .join("release")
        .join(exe_name());
    if !binary.is_file() {
        bail!("MCPHub release binary is missing at {}", binary.display());
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
