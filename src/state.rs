use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::base_dir;

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct Note {
    pub timestamp: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct ToolCallRecord {
    pub timestamp: String,
    pub tool_name: String,
    pub success: bool,
    pub duration_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct InstanceState {
    pub key: String,
    pub project_name: String,
    pub project_path: String,
    pub engine_root: String,
    pub host: String,
    pub port: u16,
    pub url: String,
    pub pid: Option<u32>,
    pub status: String,
    pub last_seen: String,
    pub crash_count: u32,
    pub notes: Vec<Note>,
    pub call_history: Vec<ToolCallRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppState {
    pub instances: BTreeMap<String, InstanceState>,
    pub active_instance: String,
}

#[derive(Debug)]
pub struct StateStore {
    path: PathBuf,
    data: AppState,
}

impl StateStore {
    pub fn load() -> Result<Self> {
        let path = state_path();
        let data = if path.is_file() {
            let raw = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            serde_json::from_str(&raw).unwrap_or_default()
        } else {
            AppState::default()
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

    pub fn list_instances(&self) -> Vec<&InstanceState> {
        self.data.instances.values().collect()
    }

    pub fn get_instance(&self, identifier: &str) -> Option<&InstanceState> {
        self.resolve_instance_key(identifier)
            .and_then(|key| self.data.instances.get(&key))
    }

    pub fn get_active_instance(&self) -> Option<&InstanceState> {
        if let Some(active) = self.data.instances.get(&self.data.active_instance) {
            if active.status == "online" {
                return Some(active);
            }
        }

        self.data
            .instances
            .values()
            .find(|instance| instance.status == "online")
            .or_else(|| self.data.instances.get(&self.data.active_instance))
    }

    pub fn upsert_instance(&mut self, instance: InstanceState) -> Result<()> {
        let key = if instance.key.is_empty() {
            make_instance_key(&instance.project_name, instance.port)
        } else {
            instance.key.clone()
        };
        let merged = match self.data.instances.get(&key).cloned() {
            Some(existing) => merge_instance(existing, instance, &key),
            None => {
                let mut created = instance;
                created.key = key.clone();
                if created.last_seen.is_empty() {
                    created.last_seen = now_timestamp();
                }
                created
            }
        };
        let should_activate = self.data.active_instance.is_empty()
            || self
                .data
                .instances
                .get(&self.data.active_instance)
                .map(|instance| instance.status != "online")
                .unwrap_or(true);
        self.data.instances.insert(key.clone(), merged);
        if should_activate {
            self.data.active_instance = key;
        }
        self.save()
    }

    pub fn set_active_instance(&mut self, key: &str) -> Result<bool> {
        let Some(resolved) = self.resolve_instance_key(key) else {
            return Ok(false);
        };
        self.data.active_instance = resolved;
        self.save()?;
        Ok(true)
    }

    pub fn record_note(&mut self, key: &str, note: Note) -> Result<()> {
        let mut changed = false;
        if let Some(resolved) = self.resolve_instance_key(key) {
            if let Some(instance) = self.data.instances.get_mut(&resolved) {
                instance.notes.push(note);
                instance.last_seen = now_timestamp();
                changed = true;
            }
        }
        if changed {
            self.save()?;
        }
        Ok(())
    }

    pub fn record_call(&mut self, key: &str, record: ToolCallRecord) -> Result<()> {
        let mut changed = false;
        if let Some(resolved) = self.resolve_instance_key(key) {
            if let Some(instance) = self.data.instances.get_mut(&resolved) {
                instance.call_history.push(record);
                instance.last_seen = now_timestamp();
                changed = true;
            }
        }
        if changed {
            self.save()?;
        }
        Ok(())
    }

    pub fn get_call_history(&self, key: &str, limit: usize) -> Vec<ToolCallRecord> {
        self.get_instance(key)
            .map(|instance| {
                let keep = limit.max(1);
                let len = instance.call_history.len();
                let start = len.saturating_sub(keep);
                instance.call_history[start..].to_vec()
            })
            .unwrap_or_default()
    }

    pub fn update_instance_status(
        &mut self,
        identifier: &str,
        status: &str,
        pid: Option<u32>,
    ) -> Result<()> {
        let mut changed = false;
        if let Some(resolved) = self.resolve_instance_key(identifier) {
            if let Some(instance) = self.data.instances.get_mut(&resolved) {
                instance.status = status.to_string();
                instance.last_seen = now_timestamp();
                if pid.is_some() {
                    instance.pid = pid;
                }
                changed = true;
            }
        }
        if changed {
            self.save()?;
        }
        Ok(())
    }

    pub fn mark_crashed(&mut self, identifier: &str) -> Result<()> {
        let mut changed = false;
        if let Some(resolved) = self.resolve_instance_key(identifier) {
            if let Some(instance) = self.data.instances.get_mut(&resolved) {
                instance.status = "crashed".to_string();
                instance.crash_count = instance.crash_count.saturating_add(1);
                instance.last_seen = now_timestamp();
                changed = true;
            }
        }
        if changed {
            self.save()?;
        }
        Ok(())
    }

    pub fn cleanup(&mut self, max_age_hours: f64) -> Result<Vec<String>> {
        let max_age_secs = (max_age_hours.max(0.0) * 3600.0) as u64;
        let now = current_epoch_secs();
        let mut removed = Vec::new();
        self.data.instances.retain(|key, instance| {
            if instance.status == "online" || instance.status == "starting" {
                return true;
            }
            let Some(last_seen) = parse_timestamp_secs(&instance.last_seen) else {
                return true;
            };
            let keep = now.saturating_sub(last_seen) <= max_age_secs;
            if !keep {
                removed.push(key.clone());
            }
            keep
        });

        if removed.iter().any(|key| key == &self.data.active_instance) {
            self.data.active_instance = self
                .data
                .instances
                .values()
                .find(|instance| instance.status == "online")
                .map(|instance| instance.key.clone())
                .or_else(|| self.data.instances.keys().next().cloned())
                .unwrap_or_default();
        }

        if !removed.is_empty() {
            self.save()?;
        }
        Ok(removed)
    }

    fn resolve_instance_key(&self, identifier: &str) -> Option<String> {
        if identifier.is_empty() {
            return None;
        }
        if self.data.instances.contains_key(identifier) {
            return Some(identifier.to_string());
        }
        if let Ok(port) = identifier.parse::<u16>() {
            if let Some((key, _)) = self
                .data
                .instances
                .iter()
                .find(|(_, instance)| instance.port == port)
            {
                return Some(key.clone());
            }
        }
        let lowered = identifier.to_ascii_lowercase();
        self.data.instances.iter().find_map(|(key, instance)| {
            (instance.project_name.to_ascii_lowercase() == lowered).then(|| key.clone())
        })
    }
}

fn merge_instance(
    existing: InstanceState,
    mut incoming: InstanceState,
    key: &str,
) -> InstanceState {
    incoming.key = key.to_string();
    if incoming.project_name.is_empty() {
        incoming.project_name = existing.project_name;
    }
    if incoming.project_path.is_empty() {
        incoming.project_path = existing.project_path;
    }
    if incoming.engine_root.is_empty() {
        incoming.engine_root = existing.engine_root;
    }
    if incoming.host.is_empty() {
        incoming.host = existing.host;
    }
    if incoming.url.is_empty() {
        incoming.url = existing.url;
    }
    if incoming.pid.is_none() {
        incoming.pid = existing.pid;
    }
    if incoming.last_seen.is_empty() {
        incoming.last_seen = now_timestamp();
    }
    if incoming.notes.is_empty() {
        incoming.notes = existing.notes;
    }
    if incoming.call_history.is_empty() {
        incoming.call_history = existing.call_history;
    }
    incoming.crash_count = incoming.crash_count.max(existing.crash_count);
    incoming
}

fn now_timestamp() -> String {
    current_epoch_secs().to_string()
}

fn current_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn parse_timestamp_secs(value: &str) -> Option<u64> {
    value.parse::<u64>().ok()
}

pub fn make_instance_key(project_name: &str, port: u16) -> String {
    let name = project_name.trim();
    if name.is_empty() {
        format!("unknown:{port}")
    } else {
        format!("{name}:{port}")
    }
}

pub fn state_path() -> PathBuf {
    let mut path = base_dir();
    path.push("state.json");
    path
}
