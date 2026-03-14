use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::process::Command;
use tokio::time::sleep;

use crate::config::{ConfigStore, ProjectEntry, ProjectMcpEndpoint};
use crate::paths::{
    UnrealProjectPaths, find_uproject, normalize_endpoint_id, read_project_mcp_endpoints,
    resolve_project_paths,
};
use crate::process::{is_process_alive, terminate_process};
use crate::state::{InstanceState, Note, StateStore, ToolCallRecord, make_instance_key};
use crate::submodule;
use crate::ue_client::{EndpointHealth, ToolCallOutput, ToolDescriptor, UeClient};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectEndpointSummary {
    pub name: String,
    #[serde(rename = "mcp_id", alias = "endpoint_id")]
    pub endpoint_id: String,
    #[serde(rename = "mcp_url", alias = "endpoint_url")]
    pub endpoint_url: String,
    pub transport: String,
    pub auto_start: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectSummary {
    pub name: String,
    pub uproject_path: String,
    pub engine_root: String,
    #[serde(rename = "active_mcp", alias = "active_endpoint")]
    pub active_endpoint: String,
    #[serde(rename = "mcps", alias = "endpoints")]
    pub endpoints: Vec<ProjectEndpointSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HubStatus {
    pub configured_projects: Vec<ProjectSummary>,
    pub active_project: String,
    pub active_instance: Option<InstanceState>,
    pub known_instances: Vec<InstanceState>,
    pub plugin_source_local: String,
    pub plugin_source_repo: String,
    pub mcphub_submodule_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LaunchResult {
    pub project_name: String,
    pub pid: u32,
    #[serde(rename = "mcp_url", alias = "endpoint_url")]
    pub endpoint_url: String,
    pub stdout_log: String,
    pub stderr_log: String,
    pub health: Option<EndpointHealth>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiscoveryResult {
    pub active_instance_key: Option<String>,
    pub instances: Vec<InstanceState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CrashReport {
    pub crash_dir: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EndpointToolEnvelope {
    pub project_name: String,
    #[serde(rename = "mcp_id", alias = "endpoint_id")]
    pub endpoint_id: String,
    pub instance_key: String,
    #[serde(rename = "mcp_url", alias = "endpoint_url")]
    pub endpoint_url: String,
    pub output: ToolCallOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionReport {
    pub scope: String,
    pub instance: InstanceState,
    pub notes: Vec<Note>,
    pub call_history: Vec<ToolCallRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InstanceHealthReport {
    pub instance: InstanceState,
    pub process_alive: Option<bool>,
    #[serde(rename = "mcp_health", alias = "endpoint_health")]
    pub endpoint_health: Option<EndpointHealth>,
    #[serde(rename = "mcp_error", alias = "endpoint_error")]
    pub endpoint_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StopEditorResult {
    pub instance_key: String,
    pub pid: Option<u32>,
    pub was_running: bool,
    pub stopped: bool,
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RestartResult {
    pub stop: Option<StopEditorResult>,
    pub prior_crash_report: Option<CrashReport>,
    pub launch: LaunchResult,
}

pub async fn setup_project(
    path: Option<PathBuf>,
    explicit_engine_root: Option<PathBuf>,
    name: Option<String>,
) -> Result<ProjectSummary> {
    let cwd = std::env::current_dir().context("failed to read current directory")?;
    let uproject = find_uproject(path.as_deref().unwrap_or(&cwd))?;
    let project_name = name.unwrap_or_else(|| {
        uproject
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Unreal")
            .to_string()
    });
    configure_project_from_uproject(&project_name, &uproject, explicit_engine_root.as_deref()).await
}

async fn configure_project_from_uproject(
    project_name: &str,
    uproject: &Path,
    explicit_engine_root: Option<&Path>,
) -> Result<ProjectSummary> {
    let paths = resolve_project_paths(uproject, explicit_engine_root)?;
    let mut config = ConfigStore::load()?;
    let mut discovered_endpoints =
        read_project_mcp_endpoints(project_name, &paths.project_dir, config.discovery_strategies())?;
    if discovered_endpoints.is_empty() {
        discovered_endpoints.push(ProjectMcpEndpoint {
            name: format!("{project_name} default"),
            endpoint_id: normalize_endpoint_id(&paths.project_name),
            host: "127.0.0.1".to_string(),
            port: 19840,
            path: "/mcp".to_string(),
            transport: "http".to_string(),
            auto_start: false,
        });
    }
    let first_endpoint = discovered_endpoints
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("no MCPs discovered"))?;
    config.save_project(
        project_name.to_string(),
        uproject.display().to_string(),
        paths.engine_root.display().to_string(),
        paths.engine_association.clone(),
        first_endpoint,
        now_iso_like(),
    )?;
    for endpoint in discovered_endpoints.into_iter().skip(1) {
        let _ = config.save_project_endpoint(project_name, endpoint, false)?;
    }

    Ok(get_project_config()?
        .into_iter()
        .find(|project| project.name == project_name)
        .ok_or_else(|| anyhow!("failed to load saved project summary"))?)
}

pub async fn bind_project_from_current_dir() -> Result<Option<ProjectSummary>> {
    let cwd = std::env::current_dir().context("failed to read current directory")?;
    let uproject = match find_uproject(&cwd) {
        Ok(path) => path,
        Err(_) => return Ok(None),
    };

    let configured_name = {
        let config = ConfigStore::load()?;
        config
            .list_projects()
            .iter()
            .find(|(_, entry)| same_path(Path::new(&entry.uproject_path), &uproject))
            .map(|(name, _)| name.clone())
    };

    if let Some(project_name) = configured_name {
        let _ = configure_project_from_uproject(&project_name, &uproject, None).await?;
        let mut config = ConfigStore::load()?;
        let _ = config.set_active_project(&project_name)?;
        return Ok(
            get_project_config()?
                .into_iter()
                .find(|project| project.name == project_name),
        );
    }

    let project_name = uproject
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Unreal")
        .to_string();
    let summary = configure_project_from_uproject(&project_name, &uproject, None).await?;
    let mut config = ConfigStore::load()?;
    let _ = config.set_active_project(&summary.name)?;
    Ok(Some(summary))
}

pub fn get_project_config() -> Result<Vec<ProjectSummary>> {
    let config = ConfigStore::load()?;
    Ok(config
        .list_projects()
        .iter()
        .map(|(name, entry)| build_project_summary(name, entry))
        .collect())
}

pub fn hub_status() -> Result<HubStatus> {
    let config = ConfigStore::load()?;
    let state = StateStore::load()?;
    Ok(HubStatus {
        configured_projects: get_project_config()?,
        active_project: config.active_project_name().to_string(),
        active_instance: preferred_active_instance(&config, &state).cloned(),
        known_instances: state.list_instances().into_iter().cloned().collect(),
        plugin_source_local: config.plugin_source_local().to_string(),
        plugin_source_repo: config.plugin_source_repo().to_string(),
        mcphub_submodule_path: crate::submodule::mcphub_manifest_path()
            .display()
            .to_string(),
    })
}

pub fn use_project(name: &str) -> Result<bool> {
    let mut config = ConfigStore::load()?;
    let switched = config.set_active_project(name)?;
    if !switched {
        return Ok(false);
    }

    let endpoint_id = config
        .get_active_project()
        .and_then(ProjectEntry::get_active_endpoint)
        .map(|endpoint| endpoint.endpoint_id.clone());
    if let Some(endpoint_id) = endpoint_id {
        let mut state = StateStore::load()?;
        if let Some(instance_key) = state
            .list_instances()
            .into_iter()
            .find(|instance| instance.project_name == name && instance.endpoint_id == endpoint_id)
            .map(|instance| instance.key.clone())
        {
            let _ = state.set_active_instance(&instance_key)?;
        }
    }
    Ok(true)
}

pub fn use_mcp(mcp_id: &str) -> Result<bool> {
    let mut config = ConfigStore::load()?;
    let project_name = if !config.active_project_name().is_empty() {
        config.active_project_name().to_string()
    } else if config.list_projects().len() == 1 {
        config.list_projects().keys().next().cloned().unwrap_or_default()
    } else {
        String::new()
    };
    if project_name.is_empty() {
        return Ok(false);
    }
    let switched = config.set_active_endpoint(&project_name, mcp_id)?;
    if !switched {
        return Ok(false);
    }

    let mut state = StateStore::load()?;
    if let Some(instance_key) = state
        .list_instances()
        .into_iter()
        .find(|instance| instance.project_name == project_name && instance.endpoint_id == mcp_id)
        .map(|instance| instance.key.clone())
    {
        let _ = state.set_active_instance(&instance_key)?;
    }
    Ok(true)
}

pub fn add_project_mcp(
    project_name: Option<&str>,
    mcp_id: &str,
    mcp_name: Option<&str>,
    host: &str,
    port: u16,
    path: &str,
    transport: &str,
    auto_start: bool,
    activate: bool,
) -> Result<ProjectSummary> {
    let mut config = ConfigStore::load()?;
    let project_name = project_name
        .map(str::to_string)
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            (!config.active_project_name().is_empty()).then(|| config.active_project_name().to_string())
        })
        .or_else(|| {
            (config.list_projects().len() == 1)
                .then(|| config.list_projects().keys().next().cloned())
                .flatten()
        })
        .ok_or_else(|| anyhow!("no target project configured"))?;

    let endpoint = ProjectMcpEndpoint {
        name: mcp_name
            .map(str::to_string)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| mcp_id.to_string()),
        endpoint_id: mcp_id.to_string(),
        host: host.to_string(),
        port,
        path: if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        },
        transport: transport.to_ascii_lowercase(),
        auto_start,
    };
    if !config.save_project_endpoint(&project_name, endpoint, activate)? {
        bail!("project '{}' not found", project_name);
    }

    get_project_config()?
        .into_iter()
        .find(|project| project.name == project_name)
        .ok_or_else(|| anyhow!("failed to load saved project summary"))
}

pub async fn compile_project(
    target: Option<String>,
    configuration: Option<String>,
) -> Result<String> {
    let paths = active_project_paths()?;
    let target = target.unwrap_or_else(|| "Editor".to_string());
    let configuration = configuration.unwrap_or_else(|| "Development".to_string());
    let build_target = format!("{}{}", paths.project_name, target);

    let output = Command::new(&paths.build_bat)
        .arg(&build_target)
        .arg("Win64")
        .arg(&configuration)
        .arg(&paths.uproject_path)
        .arg("-waitmutex")
        .output()
        .await
        .with_context(|| format!("failed to run {}", paths.build_bat.display()))?;

    if !output.status.success() {
        bail!("build failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    Ok(format!(
        "Built {} Win64 {} successfully.\n{}",
        build_target,
        configuration,
        String::from_utf8_lossy(&output.stdout)
    ))
}

pub async fn launch_editor(wait_seconds: u64) -> Result<LaunchResult> {
    let paths = active_project_paths()?;
    let project = active_project_entry()?;
    let endpoint = active_project_endpoint(&project)?;
    let logs_dir = paths.project_dir.join("Saved").join("Logs");
    fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create {}", logs_dir.display()))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let stdout_log = logs_dir.join(format!("orchestrator-stdout-{stamp}.log"));
    let stderr_log = logs_dir.join(format!("orchestrator-stderr-{stamp}.log"));

    let stdout = fs::File::create(&stdout_log)
        .with_context(|| format!("failed to create {}", stdout_log.display()))?;
    let stderr = fs::File::create(&stderr_log)
        .with_context(|| format!("failed to create {}", stderr_log.display()))?;

    let child = Command::new(&paths.editor_exe)
        .arg(&paths.uproject_path)
        .arg("-stdout")
        .arg("-FullStdOutLogOutput")
        .arg("-NoSplash")
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .with_context(|| format!("failed to launch {}", paths.editor_exe.display()))?;
    let pid = child
        .id()
        .ok_or_else(|| anyhow!("failed to capture process id"))?;
    let endpoint_url = project_endpoint_url(&endpoint);
    let health = if wait_seconds > 0 {
        wait_for_health(&endpoint_url, wait_seconds).await.ok()
    } else {
        None
    };

    let mut state = StateStore::load()?;
    let key = make_instance_key(&paths.project_name, &endpoint.endpoint_id, endpoint.port);
    state.upsert_instance(InstanceState {
        key: key.clone(),
        project_name: paths.project_name.clone(),
        endpoint_id: endpoint.endpoint_id.clone(),
        project_path: paths.uproject_path.display().to_string(),
        engine_root: paths.engine_root.display().to_string(),
        host: endpoint.host.clone(),
        port: endpoint.port,
        url: endpoint_url.clone(),
        pid: Some(pid),
        status: if health.is_some() {
            "online"
        } else {
            "starting"
        }
        .to_string(),
        last_seen: now_iso_like(),
        crash_count: 0,
        notes: Vec::new(),
        call_history: Vec::new(),
    })?;
    state.set_active_instance(&key)?;

    Ok(LaunchResult {
        project_name: paths.project_name,
        pid,
        endpoint_url,
        stdout_log: stdout_log.display().to_string(),
        stderr_log: stderr_log.display().to_string(),
        health,
    })
}

pub async fn discover_instances() -> Result<DiscoveryResult> {
    let config = ConfigStore::load()?;
    let mut state = StateStore::load()?;
    let mut instances = Vec::new();
    let candidates = build_discovery_candidates(&config);

    for candidate in candidates {
        if UeClient::health_check(&candidate.url).await.is_ok() {
            let existing = state.get_instance(&candidate.instance_key).cloned();
            state.upsert_instance(InstanceState {
                key: candidate.instance_key.clone(),
                project_name: candidate.project_name,
                endpoint_id: candidate.endpoint_id,
                project_path: candidate.project_path,
                engine_root: candidate.engine_root,
                host: candidate.host,
                port: candidate.port,
                url: candidate.url.clone(),
                pid: existing.as_ref().and_then(|instance| instance.pid),
                status: "online".to_string(),
                last_seen: now_iso_like(),
                crash_count: existing
                    .as_ref()
                    .map(|instance| instance.crash_count)
                    .unwrap_or(0),
                notes: Vec::new(),
                call_history: Vec::new(),
            })?;
            if let Some(saved) = state.get_instance(&candidate.instance_key).cloned() {
                instances.push(saved);
            }
            continue;
        }

        if let Some(existing) = state.get_instance(&candidate.instance_key).cloned() {
            if matches!(existing.status.as_str(), "online" | "starting") {
                state.update_instance_status(&existing.key, "offline", existing.pid)?;
            }
        }
    }

    Ok(DiscoveryResult {
        active_instance_key: state
            .get_active_instance()
            .map(|instance| instance.key.clone()),
        instances,
    })
}

pub fn use_editor(instance_key: &str) -> Result<bool> {
    let mut state = StateStore::load()?;
    state.set_active_instance(instance_key)
}

pub fn add_note(content: &str) -> Result<()> {
    let mut state = StateStore::load()?;
    let active = state
        .get_active_instance()
        .map(|instance| instance.key.clone())
        .ok_or_else(|| anyhow!("no active UE instance"))?;
    state.record_note(
        &active,
        Note {
            timestamp: now_iso_like(),
            content: content.to_string(),
        },
    )
}

pub fn get_notes() -> Result<Vec<Note>> {
    let state = StateStore::load()?;
    Ok(state
        .get_active_instance()
        .map(|instance| instance.notes.clone())
        .unwrap_or_default())
}

pub fn get_session(
    instance: Option<&str>,
    scope: Option<&str>,
    limit: usize,
) -> Result<SessionReport> {
    let state = StateStore::load()?;
    let instance = resolve_instance_or_active(&state, instance)?;
    let scope = scope.unwrap_or("full").to_ascii_lowercase();
    let mut session_instance = instance.clone();
    session_instance.notes.clear();
    session_instance.call_history.clear();
    let notes = if scope == "history" {
        Vec::new()
    } else {
        instance.notes.clone()
    };
    let call_history = if scope == "notes" || limit == 0 {
        Vec::new()
    } else {
        state.get_call_history(&instance.key, limit)
    };

    Ok(SessionReport {
        scope,
        instance: session_instance,
        notes,
        call_history,
    })
}

pub async fn stop_editor(instance: Option<&str>, force: bool) -> Result<StopEditorResult> {
    let state = StateStore::load()?;
    let instance = resolve_instance_or_active(&state, instance)?;
    let pid = instance.pid;
    let was_running = pid.map(is_process_alive).unwrap_or(false);
    let mut stopped = false;

    if let Some(pid) = pid {
        if was_running {
            terminate_process(pid, force)?;
            for _ in 0..20 {
                if !is_process_alive(pid) {
                    break;
                }
                sleep(Duration::from_millis(250)).await;
            }
            stopped = !is_process_alive(pid);
        } else {
            stopped = true;
        }
    }

    let mut state = StateStore::load()?;
    state.update_instance_status(&instance.key, "offline", pid)?;

    Ok(StopEditorResult {
        instance_key: instance.key,
        pid,
        was_running,
        stopped,
        force,
    })
}

pub async fn restart_editor(wait_seconds: u64, force: bool) -> Result<RestartResult> {
    let stop = match StateStore::load()?.get_active_instance().cloned() {
        Some(_) => Some(stop_editor(None, force).await?),
        None => None,
    };
    let prior_crash_report = get_crash_report()?;
    let launch = launch_editor(wait_seconds).await?;
    Ok(RestartResult {
        stop,
        prior_crash_report,
        launch,
    })
}

pub fn set_plugin_source(local_path: Option<&str>, repo_url: Option<&str>) -> Result<String> {
    let mut config = ConfigStore::load()?;
    if let Some(local_path) = local_path {
        config.set_plugin_source_local(local_path)?;
    }
    if let Some(repo_url) = repo_url {
        config.set_plugin_source_repo(repo_url)?;
    }
    Ok(format!(
        "plugin_source_local={}\nplugin_source_repo={}",
        config.plugin_source_local(),
        config.plugin_source_repo()
    ))
}

pub fn install_plugin() -> Result<String> {
    let config = ConfigStore::load()?;
    let project = active_project_entry()?;
    let project_dir = Path::new(&project.uproject_path)
        .parent()
        .context("project path has no parent directory")?;
    let plugins_dir = project_dir.join("Plugins");
    fs::create_dir_all(&plugins_dir)
        .with_context(|| format!("failed to create {}", plugins_dir.display()))?;

    let source_root = if !config.plugin_source_local().is_empty() {
        PathBuf::from(config.plugin_source_local())
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf()
    };
    let direct = source_root.join("UnrealCopilot.uplugin");
    let nested = source_root
        .join("Plugins")
        .join("UnrealCopilot")
        .join("UnrealCopilot.uplugin");
    let plugin_source = if direct.is_file() {
        source_root.clone()
    } else if nested.is_file() {
        source_root.join("Plugins").join("UnrealCopilot")
    } else {
        bail!(
            "could not locate UnrealCopilot plugin source under {}",
            source_root.display()
        );
    };

    let plugin_target = plugins_dir.join("UnrealCopilot");
    copy_dir_recursive(&plugin_source, &plugin_target)?;
    Ok(format!(
        "Installed UnrealCopilot plugin from {} to {}",
        plugin_source.display(),
        plugin_target.display()
    ))
}

pub fn get_crash_report() -> Result<Option<CrashReport>> {
    let project = active_project_entry()?;
    let crashes_dir = Path::new(&project.uproject_path)
        .parent()
        .context("project path has no parent directory")?
        .join("Saved")
        .join("Crashes");
    if !crashes_dir.is_dir() {
        return Ok(None);
    }

    let latest = fs::read_dir(&crashes_dir)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((path, modified))
        })
        .max_by_key(|(_, modified)| *modified);

    let Some((path, _)) = latest else {
        return Ok(None);
    };
    let crash_xml = path.join("CrashContext.runtime-xml");
    let summary = if crash_xml.is_file() {
        let raw = fs::read_to_string(&crash_xml).unwrap_or_default();
        raw.lines()
            .find(|line| line.contains("<ErrorMessage>"))
            .map(|line| {
                line.replace("<ErrorMessage>", "")
                    .replace("</ErrorMessage>", "")
                    .trim()
                    .to_string()
            })
            .unwrap_or_else(|| "Crash report found but no ErrorMessage tag was parsed".to_string())
    } else {
        "Crash directory found, but CrashContext.runtime-xml is missing".to_string()
    };

    Ok(Some(CrashReport {
        crash_dir: path.display().to_string(),
        summary,
    }))
}

pub async fn get_instance_health(instance: Option<&str>) -> Result<InstanceHealthReport> {
    let state = StateStore::load()?;
    let instance = resolve_instance_or_active(&state, instance)?;
    let process_alive = instance.pid.map(is_process_alive);
    let mut health_instance = instance.clone();
    health_instance.notes.clear();
    health_instance.call_history.clear();
    if instance.url.is_empty() || instance.port == 0 {
        return Ok(InstanceHealthReport {
            instance: health_instance,
            process_alive,
            endpoint_health: None,
            endpoint_error: Some("instance has no reachable MCP target".to_string()),
        });
    }

    let (endpoint_health, endpoint_error) = match UeClient::health_check(&instance.url).await {
        Ok(health) => (Some(health), None),
        Err(error) => (None, Some(error.to_string())),
    };

    Ok(InstanceHealthReport {
        instance: health_instance,
        process_alive,
        endpoint_health,
        endpoint_error,
    })
}

pub async fn list_tools(
    project_name: Option<&str>,
    mcp_id: Option<&str>,
) -> Result<Vec<ToolDescriptor>> {
    let (_, _, endpoint) = resolve_project_and_endpoint(project_name, mcp_id)?;
    UeClient::list_tools(&project_endpoint_url(&endpoint)).await
}

pub async fn call_tool(
    project_name: Option<&str>,
    mcp_id: Option<&str>,
    tool_name: &str,
    arguments: Map<String, Value>,
) -> Result<EndpointToolEnvelope> {
    let (project_name, _, endpoint) = resolve_project_and_endpoint(project_name, mcp_id)?;
    let endpoint_url = project_endpoint_url(&endpoint);
    let output = UeClient::call_tool(&endpoint_url, tool_name, arguments).await?;
    let mut state = StateStore::load()?;
    let instance_key = preferred_active_instance_for_endpoint(&state, &project_name, &endpoint.endpoint_id)
        .map(|instance| instance.key.clone())
        .unwrap_or_else(|| make_instance_key(&project_name, &endpoint.endpoint_id, endpoint.port));
    if state.get_instance(&instance_key).is_some() {
        state.record_call(
            &instance_key,
            ToolCallRecord {
                timestamp: now_iso_like(),
                tool_name: tool_name.to_string(),
                success: output.success,
                duration_ms: output.duration_ms,
            },
        )?;
    }
    Ok(EndpointToolEnvelope {
        project_name,
        endpoint_id: endpoint.endpoint_id,
        instance_key,
        endpoint_url,
        output,
    })
}

pub fn sync_mcphub(project_name: Option<&str>, mcp_id: Option<&str>) -> Result<String> {
    let (project_name, project, endpoint) = resolve_project_and_endpoint(project_name, mcp_id)?;
    submodule::sync_endpoint_with_mcphub(
        &endpoint.endpoint_id,
        &project_endpoint_url(&endpoint),
        &endpoint_display_name(&project_name, &project, &endpoint),
    )
}

fn active_project_entry() -> Result<ProjectEntry> {
    let config = ConfigStore::load()?;
    config
        .get_active_project()
        .cloned()
        .ok_or_else(|| anyhow!("no active project configured"))
}

fn active_project_paths() -> Result<UnrealProjectPaths> {
    let project = active_project_entry()?;
    resolve_project_paths(
        Path::new(&project.uproject_path),
        Some(Path::new(&project.engine_root)),
    )
}

fn active_project_endpoint(project: &ProjectEntry) -> Result<ProjectMcpEndpoint> {
    project
        .get_active_endpoint()
        .cloned()
        .ok_or_else(|| anyhow!("no active MCP configured for project"))
}

fn resolve_project_and_endpoint(
    project_name: Option<&str>,
    endpoint_id: Option<&str>,
) -> Result<(String, ProjectEntry, ProjectMcpEndpoint)> {
    let config = ConfigStore::load()?;
    let resolved_project_name = project_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or_else(|| {
            (!config.active_project_name().is_empty()).then(|| config.active_project_name().to_string())
        })
        .or_else(|| {
            (config.list_projects().len() == 1)
                .then(|| config.list_projects().keys().next().cloned())
                .flatten()
        })
        .ok_or_else(|| anyhow!("no project selected and no active project configured"))?;
    let project = config
        .list_projects()
        .get(&resolved_project_name)
        .cloned()
        .ok_or_else(|| anyhow!("project '{}' not found", resolved_project_name))?;
    let endpoint = endpoint_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| {
            project
                .endpoints
                .iter()
                .find(|endpoint| endpoint.endpoint_id == value)
                .cloned()
        })
        .or_else(|| project.get_active_endpoint().cloned())
        .or_else(|| (project.endpoints.len() == 1).then(|| project.endpoints[0].clone()))
        .ok_or_else(|| anyhow!("no mcp selected and no active mcp configured"))?;
    Ok((resolved_project_name, project, endpoint))
}

fn resolve_instance_or_active(
    state: &StateStore,
    identifier: Option<&str>,
) -> Result<InstanceState> {
    if let Some(identifier) = identifier {
        return state
            .get_instance(identifier)
            .cloned()
            .ok_or_else(|| anyhow!("instance '{identifier}' was not found"));
    }
    let config = ConfigStore::load()?;
    preferred_active_instance(&config, state)
        .cloned()
        .ok_or_else(|| anyhow!("no active UE instance"))
}

fn preferred_active_instance<'a>(
    config: &'a ConfigStore,
    state: &'a StateStore,
) -> Option<&'a InstanceState> {
    let preferred = config
        .get_active_project()
        .and_then(ProjectEntry::get_active_endpoint)
        .and_then(|endpoint| {
            state.list_instances().into_iter().find(|instance| {
                instance.project_name == config.active_project_name()
                    && instance.endpoint_id == endpoint.endpoint_id
            })
        });
    preferred.or_else(|| state.get_active_instance())
}

fn preferred_active_instance_for_endpoint<'a>(
    state: &'a StateStore,
    project_name: &str,
    endpoint_id: &str,
) -> Option<&'a InstanceState> {
    state.list_instances().into_iter().find(|instance| {
        instance.project_name == project_name && instance.endpoint_id == endpoint_id
    })
}

fn project_endpoint_url(endpoint: &ProjectMcpEndpoint) -> String {
    format!("http://{}:{}{}", endpoint.host, endpoint.port, endpoint.path)
}

async fn wait_for_health(url: &str, wait_seconds: u64) -> Result<EndpointHealth> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(wait_seconds);
    let mut last_error = None;
    while tokio::time::Instant::now() < deadline {
        match UeClient::health_check(url).await {
            Ok(status) => return Ok(status),
            Err(error) => {
                last_error = Some(error);
                sleep(Duration::from_secs(5)).await;
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("MCP target never became healthy")))
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target).with_context(|| format!("failed to create {}", target.display()))?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "Binaries" || name == "Intermediate" || name == ".git" {
                continue;
            }
            copy_dir_recursive(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn now_iso_like() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    secs.to_string()
}

#[derive(Debug, Clone)]
struct DiscoveryCandidate {
    instance_key: String,
    project_name: String,
    endpoint_id: String,
    project_path: String,
    engine_root: String,
    host: String,
    port: u16,
    url: String,
}

fn build_project_summary(
    name: &str,
    entry: &ProjectEntry,
) -> ProjectSummary {
    ProjectSummary {
        name: name.to_string(),
        uproject_path: entry.uproject_path.to_string(),
        engine_root: entry.engine_root.to_string(),
        active_endpoint: entry.active_endpoint.clone(),
        endpoints: entry
            .endpoints
            .iter()
            .map(|endpoint| ProjectEndpointSummary {
                name: endpoint.name.clone(),
                endpoint_id: endpoint.endpoint_id.clone(),
                endpoint_url: project_endpoint_url(endpoint),
                transport: endpoint.transport.clone(),
                auto_start: endpoint.auto_start,
            })
            .collect(),
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn build_discovery_candidates(config: &ConfigStore) -> Vec<DiscoveryCandidate> {
    let mut candidates = BTreeMap::<String, DiscoveryCandidate>::new();

    for (project_name, entry) in config.list_projects() {
        for endpoint in &entry.endpoints {
            add_discovery_candidate(
                &mut candidates,
                DiscoveryCandidate {
                    instance_key: make_instance_key(project_name, &endpoint.endpoint_id, endpoint.port),
                    project_name: project_name.clone(),
                    endpoint_id: endpoint.endpoint_id.clone(),
                    project_path: entry.uproject_path.clone(),
                    engine_root: entry.engine_root.clone(),
                    host: endpoint.host.clone(),
                    port: endpoint.port,
                    url: project_endpoint_url(endpoint),
                },
            );
        }
    }

    candidates.into_values().collect()
}

fn endpoint_display_name(
    project_name: &str,
    project: &ProjectEntry,
    endpoint: &ProjectMcpEndpoint,
) -> String {
    let project_label = Path::new(&project.uproject_path)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(project_name);
    if endpoint.name.trim().is_empty() {
        format!("{project_label} {}", endpoint.endpoint_id)
    } else {
        format!("{project_label} {}", endpoint.name)
    }
}

fn add_discovery_candidate(
    candidates: &mut BTreeMap<String, DiscoveryCandidate>,
    candidate: DiscoveryCandidate,
) {
    match candidates.get(&candidate.url) {
        Some(existing)
            if !existing.project_name.is_empty() || candidate.project_name.is_empty() => {}
        _ => {
            candidates.insert(candidate.url.clone(), candidate);
        }
    }
}
