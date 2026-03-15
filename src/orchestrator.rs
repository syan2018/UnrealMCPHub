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
use crate::process::{find_process_pid_by_command_line, is_process_alive, terminate_process};
use crate::state::{InstanceState, Note, StateStore, ToolCallRecord, make_instance_key};
use crate::submodule;
use crate::ue_client::{EndpointHealth, ToolCallOutput, ToolDescriptor, UeClient};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectEndpointSummary {
    pub name: String,
    #[serde(rename = "mcp_id")]
    pub endpoint_id: String,
    #[serde(rename = "mcp_url")]
    pub endpoint_url: String,
    pub transport: String,
    pub auto_start: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectSummary {
    pub name: String,
    pub uproject_path: String,
    pub engine_root: String,
    #[serde(rename = "active_mcp")]
    pub active_endpoint: String,
    #[serde(rename = "mcps")]
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
    #[serde(default)]
    pub reused_existing: bool,
    #[serde(rename = "mcp_url")]
    pub endpoint_url: String,
    pub stdout_log: String,
    pub stderr_log: String,
    pub health: Option<EndpointHealth>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
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
    #[serde(rename = "mcp_id")]
    pub endpoint_id: String,
    pub instance_key: String,
    #[serde(rename = "mcp_url")]
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
    #[serde(rename = "mcp_health")]
    pub endpoint_health: Option<EndpointHealth>,
    #[serde(rename = "mcp_error")]
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VerificationCheck {
    pub name: String,
    pub passed: bool,
    pub summary: String,
    pub details: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct VerificationSamples {
    pub cpp_header: Option<String>,
    pub cpp_symbol: Option<String>,
    pub blueprint_asset: Option<String>,
    pub skill_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VerifyUeReport {
    pub project_name: String,
    #[serde(rename = "mcp_url")]
    pub endpoint_url: String,
    pub wait_seconds: u64,
    pub compile_requested: bool,
    pub compile_output: Option<String>,
    pub launch: Option<LaunchResult>,
    pub stop: Option<StopEditorResult>,
    pub hub_status: Option<HubStatus>,
    pub discovery: Option<DiscoveryResult>,
    pub health: Option<InstanceHealthReport>,
    pub tool_names: Vec<String>,
    pub samples: VerificationSamples,
    pub notes: Vec<String>,
    pub checks: Vec<VerificationCheck>,
    pub overall_success: bool,
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
    let discovered_endpoints = discover_or_default_project_endpoints(
        project_name,
        &paths.project_dir,
        &paths.project_name,
        &config,
    )?;
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

    configured_project_summary(&config, project_name)
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
        let paths = resolve_project_paths(&uproject, None)?;
        let mut config = ConfigStore::load()?;
        let should_refresh = config
            .list_projects()
            .get(&project_name)
            .map(|entry| project_entry_needs_refresh(&project_name, entry, &paths, &config))
            .transpose()?
            .unwrap_or(true);

        if should_refresh {
            let _ = configure_project_from_uproject(&project_name, &uproject, None).await?;
            config = ConfigStore::load()?;
        }
        let _ = config.set_active_project(&project_name)?;
        return configured_project_summary(&config, &project_name).map(Some);
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
        config
            .list_projects()
            .keys()
            .next()
            .cloned()
            .unwrap_or_default()
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
            (!config.active_project_name().is_empty())
                .then(|| config.active_project_name().to_string())
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

    configured_project_summary(&config, &project_name)
}

pub async fn compile_project(
    target: Option<String>,
    configuration: Option<String>,
) -> Result<String> {
    let paths = active_project_paths()?;
    let configuration = configuration.unwrap_or_else(|| "Development".to_string());
    let build_target =
        resolve_build_target_name(&paths.project_dir, &paths.project_name, target.as_deref());

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
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let combined = [stdout, stderr]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        bail!(
            "build failed for target {build_target}: {}",
            if combined.is_empty() {
                "no compiler output captured".to_string()
            } else {
                combined
            }
        );
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
    let endpoint_url = project_endpoint_url(&endpoint);
    if let Some(pid) = existing_editor_pid_for_launch(
        &paths.project_name,
        &endpoint,
        &paths.uproject_path,
        &paths.editor_exe,
    )? {
        let health = if wait_seconds > 0 {
            wait_for_health(&endpoint_url, wait_seconds).await.ok()
        } else {
            None
        };
        let mut notes = vec![format!(
            "Reused existing Unreal Editor process for '{}' (pid {}).",
            paths.project_name, pid
        )];
        notes.extend(launch_endpoint_notes(
            &paths.project_name,
            &endpoint,
            wait_seconds,
            health.is_some(),
        ));

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
        state.update_instance_status(
            &key,
            if health.is_some() {
                "online"
            } else {
                "starting"
            },
            Some(pid),
        )?;
        state.set_active_instance(&key)?;

        return Ok(LaunchResult {
            project_name: paths.project_name,
            pid,
            reused_existing: true,
            endpoint_url,
            stdout_log: String::new(),
            stderr_log: String::new(),
            health,
            notes,
        });
    }

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
    let health = if wait_seconds > 0 {
        wait_for_health(&endpoint_url, wait_seconds).await.ok()
    } else {
        None
    };
    let notes = launch_endpoint_notes(
        &paths.project_name,
        &endpoint,
        wait_seconds,
        health.is_some(),
    );

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
        reused_existing: false,
        endpoint_url,
        stdout_log: stdout_log.display().to_string(),
        stderr_log: stderr_log.display().to_string(),
        health,
        notes,
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
            let pid = existing
                .as_ref()
                .and_then(|instance| instance.pid)
                .filter(|pid| is_process_alive(*pid))
                .or_else(|| find_project_editor_pid(&candidate.project_path));
            state.upsert_instance(InstanceState {
                key: candidate.instance_key.clone(),
                project_name: candidate.project_name,
                endpoint_id: candidate.endpoint_id,
                project_path: candidate.project_path,
                engine_root: candidate.engine_root,
                host: candidate.host,
                port: candidate.port,
                url: candidate.url.clone(),
                pid,
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
                let pid = existing.pid.filter(|pid| is_process_alive(*pid));
                let status = if pid.is_some() && existing.status == "starting" {
                    "starting"
                } else {
                    "offline"
                };
                state.update_instance_status(&existing.key, status, pid)?;
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
    let mut stopped = pid.is_none();
    let mut force_used = force;

    if let Some(pid) = pid {
        if was_running {
            force_used = terminate_process(pid, force)?;
            stopped = wait_for_process_exit(pid, 20, Duration::from_millis(250)).await;

            if !stopped && !force_used {
                force_used = terminate_process(pid, true)?;
                stopped = wait_for_process_exit(pid, 20, Duration::from_millis(250)).await;
            }
        } else {
            stopped = true;
            force_used = false;
        }
    }

    let mut state = StateStore::load()?;
    let final_pid = if stopped { None } else { pid };
    let final_status = if stopped { "offline" } else { "stopping" };
    state.update_instance_status(&instance.key, final_status, final_pid)?;

    Ok(StopEditorResult {
        instance_key: instance.key,
        pid,
        was_running,
        stopped,
        force: force_used,
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

    let plugin_source = if !config.plugin_source_local().is_empty() {
        resolve_plugin_source_root(Path::new(config.plugin_source_local())).ok_or_else(|| {
            anyhow!(
                "configured plugin source does not contain UnrealCopilot.uplugin: {}",
                config.plugin_source_local()
            )
        })?
    } else {
        detect_plugin_source_root().ok_or_else(|| {
            anyhow!(
                "could not locate UnrealCopilot plugin source relative to the current executable"
            )
        })?
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
        if instance.pid.is_some() && process_alive == Some(false) {
            let mut state = StateStore::load()?;
            state.update_instance_status(&instance.key, "offline", None)?;
            health_instance.pid = None;
            health_instance.status = "offline".to_string();
        }
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

    let refreshed_pid = match process_alive {
        Some(true) => instance.pid,
        Some(false) => None,
        None => None,
    };
    let refreshed_status = if endpoint_health.is_some() {
        "online"
    } else if process_alive == Some(false) {
        "offline"
    } else {
        instance.status.as_str()
    };
    if refreshed_status != instance.status || refreshed_pid != instance.pid {
        let mut state = StateStore::load()?;
        state.update_instance_status(&instance.key, refreshed_status, refreshed_pid)?;
    }
    health_instance.status = refreshed_status.to_string();
    health_instance.pid = refreshed_pid;

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
    let instance_key =
        preferred_active_instance_for_endpoint(&state, &project_name, &endpoint.endpoint_id)
            .map(|instance| instance.key.clone())
            .unwrap_or_else(|| {
                make_instance_key(&project_name, &endpoint.endpoint_id, endpoint.port)
            });
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

pub async fn verify_ue(
    wait_seconds: u64,
    compile_before_launch: bool,
    stop_editor_after: bool,
) -> Result<VerifyUeReport> {
    let paths = active_project_paths()?;
    let endpoint = active_project_endpoint(&active_project_entry()?)?;
    let endpoint_url = project_endpoint_url(&endpoint);
    let mut checks = Vec::new();
    let mut notes = Vec::new();
    if let Some(note) = endpoint_manual_start_note(&paths.project_name, &endpoint) {
        notes.push(note);
    }
    let mut report = VerifyUeReport {
        project_name: paths.project_name.clone(),
        endpoint_url,
        wait_seconds,
        compile_requested: compile_before_launch,
        compile_output: None,
        launch: None,
        stop: None,
        hub_status: None,
        discovery: None,
        health: None,
        tool_names: Vec::new(),
        samples: discover_verification_samples(&paths.project_dir),
        notes: Vec::new(),
        checks: Vec::new(),
        overall_success: false,
    };

    match hub_status() {
        Ok(status) => {
            let passed = !status.active_project.is_empty();
            push_verification_check(
                &mut checks,
                "hub_status",
                passed,
                if passed {
                    format!("active project is '{}'", status.active_project)
                } else {
                    "hub has no active project".to_string()
                },
                serde_json::to_value(&status).unwrap_or(Value::Null),
            );
            report.hub_status = Some(status);
        }
        Err(error) => push_verification_check(
            &mut checks,
            "hub_status",
            false,
            format!("failed to read hub status: {error}"),
            Value::String(error.to_string()),
        ),
    }

    if compile_before_launch {
        match compile_project(None, None).await {
            Ok(output) => {
                report.compile_output = Some(output.clone());
                push_verification_check(
                    &mut checks,
                    "compile_project",
                    true,
                    "project compiled successfully".to_string(),
                    Value::String(output),
                );
            }
            Err(error) => push_verification_check(
                &mut checks,
                "compile_project",
                false,
                format!("compile failed: {error}"),
                Value::String(error.to_string()),
            ),
        }
    }

    let _ = discover_instances().await;
    let existing_health = get_instance_health(None).await.ok();
    let existing_healthy = existing_health
        .as_ref()
        .map(instance_health_is_healthy)
        .unwrap_or(false);

    if existing_healthy {
        notes.push("Reused the existing healthy Unreal Editor instance.".to_string());
    } else {
        match launch_editor(wait_seconds).await {
            Ok(result) => {
                notes.push(format!("Launched Unreal Editor with PID {}.", result.pid));
                notes.extend(result.notes.iter().cloned());
                push_verification_check(
                    &mut checks,
                    "launch_editor",
                    result.health.is_some(),
                    if result.health.is_some() {
                        "editor launched and MCP endpoint became healthy".to_string()
                    } else {
                        "editor launched but MCP endpoint did not report healthy within wait window"
                            .to_string()
                    },
                    serde_json::to_value(&result).unwrap_or(Value::Null),
                );
                report.launch = Some(result);
            }
            Err(error) => push_verification_check(
                &mut checks,
                "launch_editor",
                false,
                format!("failed to launch editor: {error}"),
                Value::String(error.to_string()),
            ),
        }
    }

    match discover_instances().await {
        Ok(discovery) => {
            let passed = !discovery.instances.is_empty();
            push_verification_check(
                &mut checks,
                "discover_instances",
                passed,
                if passed {
                    format!(
                        "discovered {} Unreal instance(s)",
                        discovery.instances.len()
                    )
                } else {
                    "no reachable Unreal instances were discovered".to_string()
                },
                serde_json::to_value(&discovery).unwrap_or(Value::Null),
            );
            report.discovery = Some(discovery);
        }
        Err(error) => push_verification_check(
            &mut checks,
            "discover_instances",
            false,
            format!("discover failed: {error}"),
            Value::String(error.to_string()),
        ),
    }

    let health_target = report
        .discovery
        .as_ref()
        .and_then(|discovery| discovery.active_instance_key.as_deref());
    match get_instance_health(health_target).await {
        Ok(health) => {
            let passed = instance_health_is_healthy(&health);
            push_verification_check(
                &mut checks,
                "get_instance_health",
                passed,
                if passed {
                    format!(
                        "instance '{}' is healthy and reachable",
                        health.instance.key
                    )
                } else {
                    format!("instance '{}' is not healthy", health.instance.key)
                },
                serde_json::to_value(&health).unwrap_or(Value::Null),
            );
            report.health = Some(health);
        }
        Err(error) => push_verification_check(
            &mut checks,
            "get_instance_health",
            false,
            format!("health check failed: {error}"),
            Value::String(error.to_string()),
        ),
    }

    let tools = match list_tools(None, None).await {
        Ok(tools) => {
            report.tool_names = tools.iter().map(|tool| tool.name.clone()).collect();
            let missing = missing_expected_tools(&tools);
            push_verification_check(
                &mut checks,
                "list_tools",
                missing.is_empty(),
                if missing.is_empty() {
                    format!("all expected Unreal tools are exposed ({})", tools.len())
                } else {
                    format!("missing expected tools: {}", missing.join(", "))
                },
                serde_json::to_value(&tools).unwrap_or(Value::Null),
            );
            Some(tools)
        }
        Err(error) => {
            push_verification_check(
                &mut checks,
                "list_tools",
                false,
                format!("failed to list tools: {error}"),
                Value::String(error.to_string()),
            );
            None
        }
    };

    if let Some(symbol) = report.samples.cpp_symbol.clone() {
        let args = json_object([
            ("query", Value::String(symbol.clone())),
            ("domain", Value::String("cpp".to_string())),
            ("scope", Value::String("project".to_string())),
            ("max_results", Value::from(25)),
        ]);
        record_tool_check(
            &mut checks,
            "tool.search_cpp",
            "search",
            call_tool(None, None, "search", args).await,
            |payload| {
                payload
                    .get("cpp_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    > 0
            },
        );

        let args = json_object([("cpp_class", Value::String(symbol.clone()))]);
        record_tool_check(
            &mut checks,
            "tool.find_cpp_class_usage",
            "find_cpp_class_usage",
            call_tool(None, None, "find_cpp_class_usage", args).await,
            |payload| {
                payload.get("ok").and_then(Value::as_bool).unwrap_or(true)
                    && payload.get("as_parent_class").map_or(true, Value::is_array)
            },
        );
    } else {
        notes.push(
            "Skipped C++ verification because no representative header was found.".to_string(),
        );
    }

    if let Some(header) = report.samples.cpp_header.clone() {
        let args = json_object([
            ("file_path", Value::String(header)),
            ("format", Value::String("summary".to_string())),
        ]);
        record_tool_check(
            &mut checks,
            "tool.detect_ue_patterns",
            "detect_ue_patterns",
            call_tool(None, None, "detect_ue_patterns", args).await,
            |payload| {
                payload.get("ok").and_then(Value::as_bool).unwrap_or(true)
                    || payload.get("blueprint_callable_functions").is_some()
                    || payload.get("blueprint_properties").is_some()
            },
        );
    }

    if let Some(asset_path) = report.samples.blueprint_asset.clone() {
        let asset_name = asset_path
            .rsplit('/')
            .next()
            .unwrap_or(asset_path.as_str())
            .to_string();

        let args = json_object([
            ("query", Value::String(asset_name)),
            ("domain", Value::String("blueprint".to_string())),
            (
                "scope",
                Value::String(guess_scope_from_asset_path(&asset_path).to_string()),
            ),
            ("max_results", Value::from(25)),
        ]);
        record_tool_check(
            &mut checks,
            "tool.search_blueprint",
            "search",
            call_tool(None, None, "search", args).await,
            |payload| {
                payload
                    .get("blueprint_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    > 0
            },
        );

        let args = json_object([
            ("name", Value::String(asset_path.clone())),
            ("domain", Value::String("blueprint".to_string())),
        ]);
        record_tool_check(
            &mut checks,
            "tool.get_hierarchy",
            "get_hierarchy",
            call_tool(None, None, "get_hierarchy", args).await,
            |payload| payload.get("hierarchy").map_or(false, Value::is_array),
        );

        let args = json_object([
            ("path", Value::String(asset_path.clone())),
            ("domain", Value::String("blueprint".to_string())),
        ]);
        record_tool_check(
            &mut checks,
            "tool.get_details_blueprint",
            "get_details",
            call_tool(None, None, "get_details", args).await,
            |payload| payload.get("ok").and_then(Value::as_bool).unwrap_or(true),
        );

        let args = json_object([
            ("path", Value::String(asset_path.clone())),
            ("domain", Value::String("asset".to_string())),
        ]);
        record_tool_check(
            &mut checks,
            "tool.get_details_asset",
            "get_details",
            call_tool(None, None, "get_details", args).await,
            |payload| {
                payload.get("type").is_some()
                    || payload.get("ok").and_then(Value::as_bool).unwrap_or(false)
            },
        );

        let args = json_object([
            ("bp_path", Value::String(asset_path.clone())),
            ("graph_name", Value::String("EventGraph".to_string())),
            ("format", Value::String("summary".to_string())),
        ]);
        record_tool_check(
            &mut checks,
            "tool.get_blueprint_graph",
            "get_blueprint_graph",
            call_tool(None, None, "get_blueprint_graph", args).await,
            |payload| {
                payload.get("summary").is_some()
                    || payload.get("ok").and_then(Value::as_bool).unwrap_or(false)
            },
        );

        let args = json_object([
            ("path", Value::String(asset_path.clone())),
            ("domain", Value::String("asset".to_string())),
            ("direction", Value::String("both".to_string())),
        ]);
        record_tool_check(
            &mut checks,
            "tool.get_references",
            "get_references",
            call_tool(None, None, "get_references", args).await,
            |payload| {
                payload.get("incoming").map_or(false, Value::is_array)
                    || payload.get("outgoing").map_or(false, Value::is_array)
            },
        );

        let args = json_object([
            ("start_asset", Value::String(asset_path.clone())),
            ("max_depth", Value::from(2)),
            ("direction", Value::String("both".to_string())),
        ]);
        record_tool_check(
            &mut checks,
            "tool.trace_reference_chain",
            "trace_reference_chain",
            call_tool(None, None, "trace_reference_chain", args).await,
            |payload| payload.get("chain").map_or(false, Value::is_object),
        );
    } else {
        notes.push(
            "Skipped Blueprint/asset verification because no representative .uasset was found."
                .to_string(),
        );
    }

    let listed_skills = match call_tool(None, None, "list_unreal_skill", Map::new()).await {
        Ok(envelope) => {
            let payload = extract_tool_payload(&envelope.output).unwrap_or(Value::Null);
            let skill_names = payload
                .get("skills")
                .and_then(Value::as_array)
                .map(|skills| {
                    skills
                        .iter()
                        .filter_map(|skill| skill.get("name").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if report.samples.skill_name.is_none() {
                report.samples.skill_name = skill_names.first().cloned();
            }
            push_verification_check(
                &mut checks,
                "tool.list_unreal_skill",
                envelope.output.success && !skill_names.is_empty(),
                if skill_names.is_empty() {
                    "no Unreal skills were returned".to_string()
                } else {
                    format!("listed {} Unreal skill(s)", skill_names.len())
                },
                payload,
            );
            true
        }
        Err(error) => {
            push_verification_check(
                &mut checks,
                "tool.list_unreal_skill",
                false,
                format!("list_unreal_skill failed: {error}"),
                Value::String(error.to_string()),
            );
            false
        }
    };

    if let Some(skill_name) = report.samples.skill_name.clone() {
        let args = json_object([("skill_name", Value::String(skill_name.clone()))]);
        record_tool_check(
            &mut checks,
            "tool.read_unreal_skill",
            "read_unreal_skill",
            call_tool(None, None, "read_unreal_skill", args).await,
            |payload| {
                payload
                    .get("content")
                    .and_then(Value::as_str)
                    .map_or(false, |content| !content.is_empty())
            },
        );
    } else if listed_skills {
        notes.push(
            "Skipped read_unreal_skill because the skill list returned no usable entry."
                .to_string(),
        );
    }

    let args = json_object([
        (
            "python",
            Value::String("RESULT = {'ok': True, 'source': 'verify-ue'}".to_string()),
        ),
        ("skill_name", Value::Null),
        ("script", Value::Null),
        ("args", Value::Object(Map::new())),
    ]);
    record_tool_check(
        &mut checks,
        "tool.run_unreal_skill",
        "run_unreal_skill",
        call_tool(None, None, "run_unreal_skill", args).await,
        |payload| {
            payload.get("ok").and_then(Value::as_bool).unwrap_or(true)
                && payload.get("result").is_some()
        },
    );

    let note = format!("verify-ue {}", now_iso_like());
    match add_note(&note) {
        Ok(()) => match get_session(None, Some("full"), 20) {
            Ok(session) => {
                let note_found = session.notes.iter().any(|entry| entry.content == note);
                let has_history = !session.call_history.is_empty();
                push_verification_check(
                    &mut checks,
                    "session_state",
                    note_found && has_history,
                    if note_found && has_history {
                        "session notes and call history were persisted".to_string()
                    } else {
                        "session state is missing notes or call history".to_string()
                    },
                    serde_json::to_value(&session).unwrap_or(Value::Null),
                );
            }
            Err(error) => push_verification_check(
                &mut checks,
                "session_state",
                false,
                format!("failed to read session state: {error}"),
                Value::String(error.to_string()),
            ),
        },
        Err(error) => push_verification_check(
            &mut checks,
            "session_state",
            false,
            format!("failed to record session note: {error}"),
            Value::String(error.to_string()),
        ),
    }

    match sync_mcphub(None, None) {
        Ok(output) => push_verification_check(
            &mut checks,
            "sync_mcphub",
            true,
            "active Unreal MCP was mirrored into bundled MCPHub".to_string(),
            Value::String(output),
        ),
        Err(error) => push_verification_check(
            &mut checks,
            "sync_mcphub",
            false,
            format!("sync_mcphub failed: {error}"),
            Value::String(error.to_string()),
        ),
    }

    match get_crash_report() {
        Ok(report_or_none) => push_verification_check(
            &mut checks,
            "get_crash_report",
            true,
            if report_or_none.is_some() {
                "crash report lookup succeeded and found a report".to_string()
            } else {
                "crash report lookup succeeded (no crash reports present)".to_string()
            },
            serde_json::to_value(report_or_none).unwrap_or(Value::Null),
        ),
        Err(error) => push_verification_check(
            &mut checks,
            "get_crash_report",
            false,
            format!("get_crash_report failed: {error}"),
            Value::String(error.to_string()),
        ),
    }

    if stop_editor_after {
        match stop_editor(None, false).await {
            Ok(result) => {
                notes.push(format!(
                    "Stopped editor instance '{}' after verification.",
                    result.instance_key
                ));
                report.stop = Some(result);
            }
            Err(error) => match stop_editor(None, true).await {
                Ok(result) => {
                    notes.push(format!(
                        "Graceful stop failed after verification ({error}); forced stop succeeded for '{}'.",
                        result.instance_key
                    ));
                    report.stop = Some(result);
                }
                Err(force_error) => notes.push(format!(
                    "Failed to stop editor after verification: graceful={error}; forced={force_error}"
                )),
            },
        }
    }

    report.notes = notes;
    report.overall_success = checks.iter().all(|check| check.passed);
    report.checks = checks;
    let _ = tools;
    Ok(report)
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

fn configured_project_summary(config: &ConfigStore, project_name: &str) -> Result<ProjectSummary> {
    let entry = config
        .list_projects()
        .get(project_name)
        .ok_or_else(|| anyhow!("failed to load saved project summary"))?;
    Ok(build_project_summary(project_name, entry))
}

fn discover_or_default_project_endpoints(
    project_name: &str,
    project_dir: &Path,
    normalized_project_name: &str,
    config: &ConfigStore,
) -> Result<Vec<ProjectMcpEndpoint>> {
    let mut discovered_endpoints =
        read_project_mcp_endpoints(project_name, project_dir, config.discovery_strategies())?;
    if discovered_endpoints.is_empty() {
        discovered_endpoints.push(ProjectMcpEndpoint {
            name: format!("{project_name} default"),
            endpoint_id: normalize_endpoint_id(normalized_project_name),
            host: "127.0.0.1".to_string(),
            port: 19840,
            path: "/mcp".to_string(),
            transport: "http".to_string(),
            auto_start: false,
        });
    }
    Ok(discovered_endpoints)
}

fn project_entry_needs_refresh(
    project_name: &str,
    entry: &ProjectEntry,
    paths: &UnrealProjectPaths,
    config: &ConfigStore,
) -> Result<bool> {
    if !same_path(Path::new(&entry.uproject_path), &paths.uproject_path) {
        return Ok(true);
    }
    if !same_path(Path::new(&entry.engine_root), &paths.engine_root) {
        return Ok(true);
    }

    let discovered_endpoints = discover_or_default_project_endpoints(
        project_name,
        &paths.project_dir,
        &paths.project_name,
        config,
    )?;
    Ok(entry.endpoints != discovered_endpoints)
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
            (!config.active_project_name().is_empty())
                .then(|| config.active_project_name().to_string())
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

fn reusable_instance_for_endpoint(
    project_name: &str,
    endpoint: &ProjectMcpEndpoint,
    uproject_path: &Path,
) -> Result<Option<InstanceState>> {
    let state = StateStore::load()?;
    Ok(
        preferred_active_instance_for_endpoint(&state, project_name, &endpoint.endpoint_id)
            .filter(|instance| {
                instance.port == endpoint.port
                    && instance.host == endpoint.host
                    && same_path(Path::new(&instance.project_path), uproject_path)
                    && instance.pid.map(is_process_alive).unwrap_or(false)
            })
            .cloned(),
    )
}

fn existing_editor_pid_for_launch(
    project_name: &str,
    endpoint: &ProjectMcpEndpoint,
    uproject_path: &Path,
    editor_exe: &Path,
) -> Result<Option<u32>> {
    if let Some(instance) = reusable_instance_for_endpoint(project_name, endpoint, uproject_path)? {
        if let Some(pid) = instance.pid {
            return Ok(Some(pid));
        }
    }
    Ok(find_editor_pid(editor_exe, uproject_path))
}

fn find_project_editor_pid(project_path: &str) -> Option<u32> {
    let path = Path::new(project_path);
    find_editor_pid(Path::new(default_editor_process_name()), path)
}

fn find_editor_pid(editor_exe: &Path, uproject_path: &Path) -> Option<u32> {
    let process_name = editor_exe
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| default_editor_process_name().to_string());
    let needle = uproject_path.display().to_string();
    find_process_pid_by_command_line(&process_name, &needle)
}

fn default_editor_process_name() -> &'static str {
    if cfg!(windows) {
        "UnrealEditor.exe"
    } else {
        "UnrealEditor"
    }
}

fn project_endpoint_url(endpoint: &ProjectMcpEndpoint) -> String {
    format!(
        "http://{}:{}{}",
        endpoint.host, endpoint.port, endpoint.path
    )
}

fn endpoint_manual_start_note(project_name: &str, endpoint: &ProjectMcpEndpoint) -> Option<String> {
    if endpoint.auto_start {
        None
    } else {
        Some(format!(
            "Active MCP '{}' for project '{}' is configured with auto_start=false. UnrealMCPHub can launch the editor, but the embedded endpoint may stay offline until the Unreal plugin starts it. Enable UnrealCopilot auto-start in project settings or start the MCP server manually inside the editor.",
            endpoint.endpoint_id, project_name
        ))
    }
}

fn launch_endpoint_notes(
    project_name: &str,
    endpoint: &ProjectMcpEndpoint,
    wait_seconds: u64,
    endpoint_became_healthy: bool,
) -> Vec<String> {
    let mut notes = Vec::new();
    if let Some(note) = endpoint_manual_start_note(project_name, endpoint) {
        notes.push(note);
    }
    if wait_seconds > 0 && !endpoint_became_healthy {
        notes.push(format!(
            "Timed out waiting up to {}s for '{}' at {}. If the editor is running but the endpoint is still offline, confirm the plugin is enabled and the MCP server is started for this project.",
            wait_seconds,
            endpoint.endpoint_id,
            project_endpoint_url(endpoint)
        ));
    }
    notes
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

async fn wait_for_process_exit(pid: u32, attempts: usize, interval: Duration) -> bool {
    for _ in 0..attempts {
        if !is_process_alive(pid) {
            return true;
        }
        sleep(interval).await;
    }
    !is_process_alive(pid)
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

fn detect_plugin_source_root() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .and_then(|dir| {
            dir.ancestors()
                .find_map(|candidate| resolve_plugin_source_root(candidate))
        })
}

fn resolve_plugin_source_root(candidate: &Path) -> Option<PathBuf> {
    if candidate.join("UnrealCopilot.uplugin").is_file() {
        return Some(candidate.to_path_buf());
    }

    let nested = candidate
        .join("Plugins")
        .join("UnrealCopilot")
        .join("UnrealCopilot.uplugin");
    nested
        .is_file()
        .then(|| candidate.join("Plugins").join("UnrealCopilot"))
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

fn build_project_summary(name: &str, entry: &ProjectEntry) -> ProjectSummary {
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
                    instance_key: make_instance_key(
                        project_name,
                        &endpoint.endpoint_id,
                        endpoint.port,
                    ),
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

fn push_verification_check(
    checks: &mut Vec<VerificationCheck>,
    name: &str,
    passed: bool,
    summary: String,
    details: Value,
) {
    checks.push(VerificationCheck {
        name: name.to_string(),
        passed,
        summary,
        details,
    });
}

fn instance_health_is_healthy(report: &InstanceHealthReport) -> bool {
    report.endpoint_error.is_none()
        && report.endpoint_health.is_some()
        && report.process_alive.unwrap_or(true)
}

fn missing_expected_tools(tools: &[ToolDescriptor]) -> Vec<String> {
    const EXPECTED: &[&str] = &[
        "search",
        "get_hierarchy",
        "get_references",
        "get_details",
        "get_blueprint_graph",
        "detect_ue_patterns",
        "trace_reference_chain",
        "find_cpp_class_usage",
        "list_unreal_skill",
        "read_unreal_skill",
        "run_unreal_skill",
    ];

    let names = tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    EXPECTED
        .iter()
        .filter(|expected| !names.iter().any(|name| name == *expected))
        .map(|name| (*name).to_string())
        .collect()
}

fn json_object<const N: usize>(pairs: [(&str, Value); N]) -> Map<String, Value> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn record_tool_check<F>(
    checks: &mut Vec<VerificationCheck>,
    check_name: &str,
    tool_name: &str,
    result: Result<EndpointToolEnvelope>,
    predicate: F,
) where
    F: Fn(&Value) -> bool,
{
    match result {
        Ok(envelope) => {
            let payload = extract_tool_payload(&envelope.output).unwrap_or(Value::Null);
            let payload_ok = payload_declares_success(&payload).unwrap_or(true);
            let passed = envelope.output.success && payload_ok && predicate(&payload);
            push_verification_check(
                checks,
                check_name,
                passed,
                if passed {
                    format!("{tool_name} returned a valid live UE response")
                } else {
                    format!("{tool_name} returned an unexpected payload")
                },
                serde_json::to_value(&envelope).unwrap_or(Value::Null),
            );
        }
        Err(error) => push_verification_check(
            checks,
            check_name,
            false,
            format!("{tool_name} failed: {error}"),
            Value::String(error.to_string()),
        ),
    }
}

fn extract_tool_payload(output: &ToolCallOutput) -> Option<Value> {
    if let Some(payload) = &output.structured_content {
        return Some(payload.clone());
    }

    for item in &output.content {
        if let Some(json) = item.get("json") {
            return Some(json.clone());
        }
        if let Some(text) = item.get("text").and_then(Value::as_str) {
            if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                return Some(parsed);
            }
        }
    }

    None
}

fn payload_declares_success(payload: &Value) -> Option<bool> {
    payload.get("ok").and_then(Value::as_bool)
}

fn discover_verification_samples(project_dir: &Path) -> VerificationSamples {
    let cpp_header = find_sample_cpp_header(&project_dir.join("Source"));
    let cpp_symbol = cpp_header
        .as_ref()
        .and_then(|path| extract_cpp_symbol(path).ok());
    let blueprint_asset = find_sample_blueprint_asset(project_dir);

    VerificationSamples {
        cpp_header: cpp_header.map(|path| path.display().to_string()),
        cpp_symbol,
        blueprint_asset,
        skill_name: None,
    }
}

fn find_sample_cpp_header(source_dir: &Path) -> Option<PathBuf> {
    let mut headers = Vec::new();
    collect_files_recursive(source_dir, &mut headers, |path| {
        path.extension().and_then(|ext| ext.to_str()) == Some("h")
    });

    headers.sort_by(|left, right| {
        score_cpp_header_candidate(right)
            .cmp(&score_cpp_header_candidate(left))
            .then_with(|| left.cmp(right))
    });

    headers.into_iter().next()
}

fn score_cpp_header_candidate(path: &Path) -> usize {
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut score = 0usize;
    for (needle, weight) in [
        ("healthcomponent", 90usize),
        ("gamemode", 80usize),
        ("herocomponent", 70usize),
        ("gameplayability", 40usize),
        ("character", 30usize),
        ("component", 20usize),
    ] {
        if stem.contains(needle) {
            score += weight;
        }
    }
    score
}

fn extract_cpp_symbol(header_path: &Path) -> Result<String> {
    let raw = fs::read_to_string(header_path)
        .with_context(|| format!("failed to read {}", header_path.display()))?;

    for line in raw.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("class ") {
            continue;
        }
        let mut tokens = trimmed.split_whitespace().skip(1);
        while let Some(token) = tokens.next() {
            if token.ends_with("_API") {
                continue;
            }

            let candidate = token
                .trim_matches('{')
                .trim_matches(':')
                .trim_matches(';')
                .trim();
            if candidate
                .chars()
                .next()
                .map(|ch| ch.is_ascii_alphabetic() || ch == '_')
                .unwrap_or(false)
                && candidate
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
            {
                return Ok(candidate.to_string());
            }
        }
    }

    header_path
        .file_stem()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow!(
                "could not infer a C++ symbol from {}",
                header_path.display()
            )
        })
}

fn find_sample_blueprint_asset(project_dir: &Path) -> Option<String> {
    let mut assets = Vec::new();
    collect_files_recursive(project_dir, &mut assets, |path| {
        path.extension().and_then(|ext| ext.to_str()) == Some("uasset")
    });

    assets.sort_by(|left, right| {
        score_blueprint_candidate(right)
            .cmp(&score_blueprint_candidate(left))
            .then_with(|| left.cmp(right))
    });

    assets
        .into_iter()
        .find_map(|path| to_unreal_asset_path(project_dir, &path))
}

fn score_blueprint_candidate(path: &Path) -> usize {
    let name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut score = 0usize;
    for (needle, weight) in [
        ("b_", 80usize),
        ("bp_", 80usize),
        ("ga_", 60usize),
        ("hero", 40usize),
        ("weapon", 30usize),
        ("lyra", 20usize),
    ] {
        if name.contains(needle) {
            score += weight;
        }
    }
    score
}

fn collect_files_recursive<F>(dir: &Path, out: &mut Vec<PathBuf>, predicate: F)
where
    F: Fn(&Path) -> bool + Copy,
{
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if matches!(
                name.as_str(),
                "Binaries" | "Intermediate" | "Saved" | "DerivedDataCache" | ".git" | "target"
            ) {
                continue;
            }
            collect_files_recursive(&path, out, predicate);
        } else if predicate(&path) {
            out.push(path);
        }
    }
}

fn to_unreal_asset_path(project_dir: &Path, file_path: &Path) -> Option<String> {
    let game_content = project_dir.join("Content");
    if let Ok(relative) = file_path.strip_prefix(&game_content) {
        return Some(format!("/Game/{}", normalize_asset_relative_path(relative)));
    }

    let mut current = file_path.parent();
    while let Some(dir) = current {
        let plugin_name = fs::read_dir(dir)
            .ok()?
            .filter_map(|entry| entry.ok())
            .find_map(|entry| {
                let path = entry.path();
                (path.extension().and_then(|ext| ext.to_str()) == Some("uplugin"))
                    .then(|| {
                        path.file_stem()
                            .and_then(|stem| stem.to_str())
                            .map(str::to_string)
                    })
                    .flatten()
            });

        if let Some(plugin_name) = plugin_name {
            let content_dir = dir.join("Content");
            if let Ok(relative) = file_path.strip_prefix(&content_dir) {
                return Some(format!(
                    "/{plugin_name}/{}",
                    normalize_asset_relative_path(relative)
                ));
            }
        }

        if same_path(dir, project_dir) {
            break;
        }
        current = dir.parent();
    }

    None
}

fn normalize_asset_relative_path(relative: &Path) -> String {
    let mut normalized = relative.to_string_lossy().replace('\\', "/");
    if let Some(stripped) = normalized.strip_suffix(".uasset") {
        normalized = stripped.to_string();
    }
    normalized
}

fn guess_scope_from_asset_path(asset_path: &str) -> &'static str {
    if asset_path.starts_with("/Game/") {
        "project"
    } else if asset_path.starts_with("/Engine/") || asset_path.starts_with("/Script/") {
        "engine"
    } else {
        "plugin"
    }
}

fn resolve_build_target_name(
    project_dir: &Path,
    project_name: &str,
    requested_target: Option<&str>,
) -> String {
    let source_dir = project_dir.join("Source");

    if let Some(requested) = requested_target
        .map(str::trim)
        .filter(|requested| !requested.is_empty())
    {
        if let Some(found) = find_matching_target_rule(&source_dir, requested) {
            return found;
        }

        if requested.eq_ignore_ascii_case("Editor")
            || requested.eq_ignore_ascii_case("Game")
            || requested.eq_ignore_ascii_case("Client")
            || requested.eq_ignore_ascii_case("Server")
        {
            if let Some(found) =
                find_matching_target_rule(&source_dir, &format!("{project_name}{requested}"))
            {
                return found;
            }
        }

        return requested.to_string();
    }

    find_matching_target_rule(&source_dir, "Editor")
        .or_else(|| find_matching_target_rule(&source_dir, &format!("{project_name}Editor")))
        .unwrap_or_else(|| format!("{project_name}Editor"))
}

fn find_matching_target_rule(source_dir: &Path, query: &str) -> Option<String> {
    let entries = fs::read_dir(source_dir).ok()?;
    let mut matches = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            let file_name = path.file_name()?.to_str()?;
            if !file_name.ends_with(".Target.cs") {
                return None;
            }

            let target_name = file_name.strip_suffix(".Target.cs")?.to_string();
            let lowered = target_name.to_ascii_lowercase();
            let query = query.to_ascii_lowercase();
            (lowered == query || lowered.ends_with(&query)).then_some(target_name)
        })
        .collect::<Vec<_>>();
    matches.sort();
    matches.into_iter().next()
}
