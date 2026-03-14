use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectEntry {
    pub uproject_path: String,
    pub engine_root: String,
    pub engine_association: String,
    pub mcp_port: u16,
    pub mcp_host: String,
    pub mcp_path: String,
    pub endpoint_id: String,
    pub configured_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub projects: BTreeMap<String, ProjectEntry>,
    pub active_project: String,
    pub scan_ports: Vec<u16>,
    pub plugin_source_local: String,
    pub plugin_source_repo: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            projects: BTreeMap::new(),
            active_project: String::new(),
            scan_ports: vec![19840, 8422, 8423, 8424, 8425],
            plugin_source_local: String::new(),
            plugin_source_repo: String::new(),
        }
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
        let data = if path.is_file() {
            let raw = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            serde_json::from_str(&raw).unwrap_or_default()
        } else {
            AppConfig::default()
        };
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

    pub fn save_project(&mut self, name: String, entry: ProjectEntry) -> Result<()> {
        self.data.projects.insert(name.clone(), entry);
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

    pub fn scan_ports(&self) -> &[u16] {
        &self.data.scan_ports
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
