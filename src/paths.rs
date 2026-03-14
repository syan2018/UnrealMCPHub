use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnrealProjectPaths {
    pub project_name: String,
    pub project_dir: PathBuf,
    pub uproject_path: PathBuf,
    pub engine_association: String,
    pub engine_root: PathBuf,
    pub editor_exe: PathBuf,
    pub build_bat: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotTransportConfig {
    pub host: String,
    pub port: u16,
    pub path: String,
    pub transport: String,
    pub auto_start: bool,
}

impl Default for CopilotTransportConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 19840,
            path: "/mcp".to_string(),
            transport: "http".to_string(),
            auto_start: false,
        }
    }
}

pub fn find_uproject(start: &Path) -> Result<PathBuf> {
    if start.is_file() {
        if start.extension().and_then(|ext| ext.to_str()) == Some("uproject") {
            return Ok(start.to_path_buf());
        }
        bail!("expected a .uproject file, got {}", start.display());
    }

    for directory in start.ancestors() {
        let mut matches = fs::read_dir(directory)
            .with_context(|| format!("failed to read {}", directory.display()))?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("uproject"))
            .collect::<Vec<_>>();
        matches.sort();
        if let Some(path) = matches.into_iter().next() {
            return Ok(path);
        }
    }

    bail!(
        "could not locate a .uproject by walking upward from {}",
        start.display()
    )
}

pub fn read_engine_association(uproject_path: &Path) -> Result<String> {
    let raw = fs::read_to_string(uproject_path)
        .with_context(|| format!("failed to read {}", uproject_path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", uproject_path.display()))?;
    Ok(value
        .get("EngineAssociation")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

pub fn resolve_project_paths(
    uproject_path: &Path,
    explicit_engine_root: Option<&Path>,
) -> Result<UnrealProjectPaths> {
    let uproject_path = uproject_path.to_path_buf();
    let project_dir = uproject_path
        .parent()
        .context("uproject path has no parent directory")?
        .to_path_buf();
    let project_name = uproject_path
        .file_stem()
        .and_then(|name| name.to_str())
        .context("uproject filename is invalid UTF-8")?
        .to_string();
    let engine_association = read_engine_association(&uproject_path)?;
    let engine_root = explicit_engine_root
        .map(PathBuf::from)
        .or_else(|| resolve_engine_root(&engine_association))
        .ok_or_else(|| {
            anyhow::anyhow!("could not resolve engine root for '{}'", engine_association)
        })?;
    let editor_exe = engine_root
        .join("Engine")
        .join("Binaries")
        .join("Win64")
        .join("UnrealEditor.exe");
    let build_bat = engine_root
        .join("Engine")
        .join("Build")
        .join("BatchFiles")
        .join("Build.bat");

    Ok(UnrealProjectPaths {
        project_name,
        project_dir,
        uproject_path,
        engine_association,
        engine_root,
        editor_exe,
        build_bat,
    })
}

pub fn read_copilot_transport(project_dir: &Path) -> Result<CopilotTransportConfig> {
    let mut config = CopilotTransportConfig::default();
    let files = [
        project_dir
            .join("Config")
            .join("DefaultEditorPerProjectUserSettings.ini"),
        project_dir
            .join("Saved")
            .join("Config")
            .join("WindowsEditor")
            .join("EditorPerProjectUserSettings.ini"),
    ];

    for path in files {
        if !path.is_file() {
            continue;
        }

        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let mut in_section = false;
        for line in raw.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                in_section =
                    trimmed.eq_ignore_ascii_case("[/Script/UnrealCopilot.UnrealCopilotSettings]");
                continue;
            }
            if !in_section || trimmed.is_empty() || trimmed.starts_with(';') {
                continue;
            }
            let Some((key, value)) = trimmed.split_once('=') else {
                continue;
            };
            match key.trim() {
                "McpHost" => config.host = value.trim().to_string(),
                "McpPort" => {
                    if let Ok(port) = value.trim().parse::<u16>() {
                        config.port = port;
                    }
                }
                "McpPath" => config.path = normalize_path(value.trim()),
                "Transport" => config.transport = normalize_transport(value.trim()),
                "bAutoStartMcpServer" => config.auto_start = parse_bool(value.trim()),
                _ => {}
            }
        }
    }

    Ok(config)
}

pub fn normalize_endpoint_id(project_name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in project_name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        "unreal-local".to_string()
    } else {
        format!("{out}-local")
    }
}

fn normalize_path(path: &str) -> String {
    if path.is_empty() {
        "/mcp".to_string()
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

fn normalize_transport(value: &str) -> String {
    let value = value.to_ascii_lowercase();
    if value.contains("stdio") {
        "stdio".to_string()
    } else if value.contains("sse") {
        "sse".to_string()
    } else {
        "http".to_string()
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn resolve_engine_root(engine_association: &str) -> Option<PathBuf> {
    let registry_candidates = [
        (
            format!(r"HKLM\SOFTWARE\EpicGames\Unreal Engine\{engine_association}"),
            "InstalledDirectory".to_string(),
        ),
        (
            r"HKCU\SOFTWARE\Epic Games\Unreal Engine\Builds".to_string(),
            engine_association.to_string(),
        ),
    ];

    for (key, value_name) in registry_candidates {
        if let Some(value) = query_registry_value(&key, &value_name) {
            let path = PathBuf::from(value);
            if path.is_dir() {
                return Some(path);
            }
        }
    }

    let fallbacks = [
        PathBuf::from(format!(r"D:\Epic Games\UE_{engine_association}")),
        PathBuf::from(format!(
            r"C:\Program Files\Epic Games\UE_{engine_association}"
        )),
        PathBuf::from(format!(r"C:\Epic Games\UE_{engine_association}")),
    ];
    fallbacks.into_iter().find(|path| path.is_dir())
}

fn query_registry_value(key: &str, value_name: &str) -> Option<String> {
    let output = std::process::Command::new("reg")
        .args(["query", key, "/v", value_name])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().find_map(|line| {
        let trimmed = line.trim();
        if !trimmed.starts_with(value_name) {
            return None;
        }
        let mut parts = trimmed.split_whitespace();
        let name = parts.next()?;
        if name != value_name {
            return None;
        }
        let _type = parts.next()?;
        Some(parts.collect::<Vec<_>>().join(" "))
    })
}
