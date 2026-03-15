use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ProjectMcpEndpoint {
    pub name: String,
    #[serde(rename = "mcp_id")]
    pub endpoint_id: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub transport: String,
    pub auto_start: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EndpointDiscoveryStrategy {
    pub name: String,
    pub config_files: Vec<String>,
    pub section: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_start_key: Option<String>,
    pub default_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectEntry {
    pub uproject_path: String,
    pub engine_root: String,
    pub engine_association: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "mcps")]
    pub endpoints: Vec<ProjectMcpEndpoint>,
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "active_mcp")]
    pub active_endpoint: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub configured_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub projects: BTreeMap<String, ProjectEntry>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub active_project: String,
    #[serde(default)]
    pub discovery_strategies: Vec<EndpointDiscoveryStrategy>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub plugin_source_local: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub plugin_source_repo: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            projects: BTreeMap::new(),
            active_project: String::new(),
            discovery_strategies: builtin_discovery_strategies(),
            plugin_source_local: String::new(),
            plugin_source_repo: String::new(),
        }
    }
}

impl ProjectEntry {
    pub fn normalize(&mut self) {
        if let Some(active) = self.get_active_endpoint().cloned() {
            self.active_endpoint = active.endpoint_id;
        } else {
            self.active_endpoint.clear();
        }
    }

    pub fn get_active_endpoint(&self) -> Option<&ProjectMcpEndpoint> {
        if self.active_endpoint.is_empty() {
            return self.endpoints.first();
        }
        self.endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint_id == self.active_endpoint)
            .or_else(|| self.endpoints.first())
    }
}

fn default_unrealcopilot_strategy() -> EndpointDiscoveryStrategy {
    EndpointDiscoveryStrategy {
        name: "unrealcopilot".to_string(),
        config_files: vec![
            "Config/DefaultEditorPerProjectUserSettings.ini".to_string(),
            "Saved/Config/WindowsEditor/EditorPerProjectUserSettings.ini".to_string(),
        ],
        section: "/Script/UnrealCopilot.UnrealCopilotSettings".to_string(),
        enable_key: None,
        host_key: Some("McpHost".to_string()),
        port_key: Some("McpPort".to_string()),
        path_key: Some("McpPath".to_string()),
        transport_key: Some("Transport".to_string()),
        auto_start_key: Some("bAutoStartMcpServer".to_string()),
        default_port: 19840,
    }
}

fn default_remote_mcp_strategy() -> EndpointDiscoveryStrategy {
    EndpointDiscoveryStrategy {
        name: "remote-mcp".to_string(),
        config_files: vec![
            "Config/DefaultEditorPerProjectUserSettings.ini".to_string(),
            "Saved/Config/WindowsEditor/EditorPerProjectUserSettings.ini".to_string(),
        ],
        section: "/Script/RemoteMCP.MCPSetting".to_string(),
        enable_key: Some("bEnable".to_string()),
        host_key: None,
        port_key: Some("Port".to_string()),
        path_key: None,
        transport_key: None,
        auto_start_key: Some("bAutoStart".to_string()),
        default_port: 8422,
    }
}

fn builtin_discovery_strategies() -> Vec<EndpointDiscoveryStrategy> {
    vec![
        default_unrealcopilot_strategy(),
        default_remote_mcp_strategy(),
    ]
}

#[derive(Debug)]
pub struct ConfigStore {
    path: PathBuf,
    data: AppConfig,
}

impl ConfigStore {
    pub fn load() -> Result<Self> {
        let path = config_path();
        let existed = path.is_file();
        let mut should_persist = !existed;
        let mut data = if existed {
            let raw = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            serde_json::from_str(&raw).unwrap_or_default()
        } else {
            AppConfig::default()
        };
        if merge_builtin_discovery_strategies(&mut data.discovery_strategies) {
            should_persist = true;
        }
        for entry in data.projects.values_mut() {
            entry.normalize();
        }
        let store = Self { path, data };
        if should_persist {
            store.save()?;
        }
        Ok(store)
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let raw = serde_json::to_string_pretty(&self.data)?;
        fs::write(&self.path, raw)
            .with_context(|| format!("failed to write {}", self.path.display()))
    }

    pub fn list_projects(&self) -> &BTreeMap<String, ProjectEntry> {
        &self.data.projects
    }

    pub fn active_project_name(&self) -> &str {
        &self.data.active_project
    }

    pub fn get_active_project(&self) -> Option<&ProjectEntry> {
        self.data.projects.get(&self.data.active_project)
    }

    pub fn save_project(
        &mut self,
        name: String,
        uproject_path: String,
        engine_root: String,
        engine_association: String,
        endpoint: ProjectMcpEndpoint,
        configured_at: String,
    ) -> Result<()> {
        let entry = self.data.projects.entry(name.clone()).or_default();
        entry.uproject_path = uproject_path;
        entry.engine_root = engine_root;
        entry.engine_association = engine_association;
        entry.configured_at = configured_at;
        match entry
            .endpoints
            .iter_mut()
            .find(|existing| existing.endpoint_id == endpoint.endpoint_id)
        {
            Some(existing) => *existing = endpoint.clone(),
            None => entry.endpoints.push(endpoint.clone()),
        }
        if entry.active_endpoint.is_empty() {
            entry.active_endpoint = endpoint.endpoint_id;
        }
        entry.normalize();
        if self.data.active_project.is_empty() {
            self.data.active_project = name;
        }
        self.save()
    }

    pub fn set_active_project(&mut self, name: &str) -> Result<bool> {
        if !self.data.projects.contains_key(name) {
            return Ok(false);
        }
        if self.data.active_project == name {
            return Ok(true);
        }
        self.data.active_project = name.to_string();
        self.save()?;
        Ok(true)
    }

    pub fn set_active_endpoint(&mut self, project_name: &str, endpoint_id: &str) -> Result<bool> {
        let Some(project) = self.data.projects.get_mut(project_name) else {
            return Ok(false);
        };
        if !project
            .endpoints
            .iter()
            .any(|endpoint| endpoint.endpoint_id == endpoint_id)
        {
            return Ok(false);
        }
        if project.active_endpoint == endpoint_id {
            return Ok(true);
        }
        project.active_endpoint = endpoint_id.to_string();
        project.normalize();
        self.save()?;
        Ok(true)
    }

    pub fn save_project_endpoint(
        &mut self,
        project_name: &str,
        endpoint: ProjectMcpEndpoint,
        activate: bool,
    ) -> Result<bool> {
        let Some(project) = self.data.projects.get_mut(project_name) else {
            return Ok(false);
        };
        match project
            .endpoints
            .iter_mut()
            .find(|existing| existing.endpoint_id == endpoint.endpoint_id)
        {
            Some(existing) => *existing = endpoint.clone(),
            None => project.endpoints.push(endpoint.clone()),
        }
        if activate || project.active_endpoint.is_empty() {
            project.active_endpoint = endpoint.endpoint_id.clone();
        }
        project.normalize();
        self.save()?;
        Ok(true)
    }

    pub fn plugin_source_local(&self) -> &str {
        &self.data.plugin_source_local
    }

    pub fn plugin_source_repo(&self) -> &str {
        &self.data.plugin_source_repo
    }

    pub fn set_plugin_source_local(&mut self, path: &str) -> Result<()> {
        self.data.plugin_source_local = path.to_string();
        self.save()
    }

    pub fn set_plugin_source_repo(&mut self, url: &str) -> Result<()> {
        self.data.plugin_source_repo = url.to_string();
        self.save()
    }

    pub fn discovery_strategies(&self) -> &[EndpointDiscoveryStrategy] {
        &self.data.discovery_strategies
    }
}

fn merge_builtin_discovery_strategies(strategies: &mut Vec<EndpointDiscoveryStrategy>) -> bool {
    let mut changed = false;
    for builtin in builtin_discovery_strategies() {
        if strategies
            .iter()
            .any(|existing| existing.name == builtin.name)
        {
            continue;
        }
        strategies.push(builtin);
        changed = true;
    }
    changed
}

pub fn base_dir() -> PathBuf {
    let mut dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    dir.push(".unreal-mcphub");
    dir
}

pub fn config_path() -> PathBuf {
    let mut path = base_dir();
    path.push("config.json");
    path
}
