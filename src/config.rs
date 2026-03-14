use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectMcpEndpoint {
    pub name: String,
    #[serde(rename = "mcp_id", alias = "endpoint_id")]
    pub endpoint_id: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub transport: String,
    pub auto_start: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointDiscoveryStrategy {
    pub name: String,
    pub config_files: Vec<String>,
    pub section: String,
    pub host_key: String,
    pub port_key: String,
    pub path_key: String,
    pub transport_key: String,
    pub auto_start_key: String,
    pub default_host: String,
    pub default_port: u16,
    pub default_path: String,
    pub default_transport: String,
    pub default_auto_start: bool,
    pub always_include: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectEntry {
    pub uproject_path: String,
    pub engine_root: String,
    pub engine_association: String,
    #[serde(default, rename = "mcps", alias = "endpoints")]
    pub endpoints: Vec<ProjectMcpEndpoint>,
    #[serde(default, rename = "active_mcp", alias = "active_endpoint")]
    pub active_endpoint: String,
    pub configured_at: String,
    #[serde(default)]
    pub mcp_port: u16,
    #[serde(default)]
    pub mcp_host: String,
    #[serde(default)]
    pub mcp_path: String,
    #[serde(default, rename = "mcp_id", alias = "endpoint_id")]
    pub endpoint_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub projects: BTreeMap<String, ProjectEntry>,
    pub active_project: String,
    #[serde(default)]
    pub discovery_strategies: Vec<EndpointDiscoveryStrategy>,
    pub plugin_source_local: String,
    pub plugin_source_repo: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            projects: BTreeMap::new(),
            active_project: String::new(),
            discovery_strategies: vec![default_unrealcopilot_strategy()],
            plugin_source_local: String::new(),
            plugin_source_repo: String::new(),
        }
    }
}

impl ProjectEntry {
    pub fn normalize(&mut self) {
        if self.endpoints.is_empty()
            && (!self.endpoint_id.is_empty() || self.mcp_port > 0 || !self.mcp_host.is_empty())
        {
            self.endpoints.push(ProjectMcpEndpoint {
                name: if self.endpoint_id.is_empty() {
                    "default".to_string()
                } else {
                    self.endpoint_id.clone()
                },
                endpoint_id: self.endpoint_id.clone(),
                host: if self.mcp_host.is_empty() {
                    "127.0.0.1".to_string()
                } else {
                    self.mcp_host.clone()
                },
                port: self.mcp_port,
                path: if self.mcp_path.is_empty() {
                    "/mcp".to_string()
                } else {
                    self.mcp_path.clone()
                },
                transport: "http".to_string(),
                auto_start: false,
            });
        }

        if self.active_endpoint.is_empty() {
            self.active_endpoint = self
                .endpoints
                .first()
                .map(|endpoint| endpoint.endpoint_id.clone())
                .unwrap_or_default();
        }
        if let Some(active) = self.get_active_endpoint().cloned() {
            self.mcp_port = active.port;
            self.mcp_host = active.host;
            self.mcp_path = active.path;
            self.endpoint_id = active.endpoint_id;
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
        host_key: "McpHost".to_string(),
        port_key: "McpPort".to_string(),
        path_key: "McpPath".to_string(),
        transport_key: "Transport".to_string(),
        auto_start_key: "bAutoStartMcpServer".to_string(),
        default_host: "127.0.0.1".to_string(),
        default_port: 19840,
        default_path: "/mcp".to_string(),
        default_transport: "http".to_string(),
        default_auto_start: false,
        always_include: true,
    }
}

#[derive(Debug)]
pub struct ConfigStore {
    path: PathBuf,
    data: AppConfig,
}

impl ConfigStore {
    pub fn load() -> Result<Self> {
        let path = config_path();
        let mut data = if path.is_file() {
            let raw = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            serde_json::from_str(&raw).unwrap_or_default()
        } else {
            AppConfig::default()
        };
        if data.discovery_strategies.is_empty() {
            data.discovery_strategies.push(default_unrealcopilot_strategy());
        }
        for entry in data.projects.values_mut() {
            entry.normalize();
        }
        Ok(Self { path, data })
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
        entry.mcp_port = endpoint.port;
        entry.mcp_host = endpoint.host.clone();
        entry.mcp_path = endpoint.path.clone();
        entry.endpoint_id = endpoint.endpoint_id.clone();
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
