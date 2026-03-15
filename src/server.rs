use anyhow::{Result, anyhow};
use axum::Router;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager,
    tower::{StreamableHttpServerConfig, StreamableHttpService},
};
use rmcp::{
    Json, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::orchestrator;
use crate::state::Note;
use crate::ue_client::ToolDescriptor;
use crate::watcher::ProcessWatcher;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct SetupProjectRequest {
    uproject_path: Option<String>,
    engine_root: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct CompileProjectRequest {
    target: Option<String>,
    configuration: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct LaunchEditorRequest {
    #[serde(default = "default_wait_seconds")]
    wait_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct UseEditorRequest {
    instance_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct UseProjectRequest {
    project_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct UseMcpRequest {
    mcp_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct AddMcpRequest {
    project: Option<String>,
    mcp_id: String,
    name: Option<String>,
    host: String,
    port: u16,
    #[serde(default = "default_mcp_path")]
    path: String,
    #[serde(default = "default_mcp_transport")]
    transport: String,
    #[serde(default)]
    auto_start: bool,
    #[serde(default)]
    activate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct AddNoteRequest {
    content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct SetPluginSourceRequest {
    local_path: Option<String>,
    repo_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct McpSelectorRequest {
    project: Option<String>,
    mcp: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct McpCallRequest {
    project: Option<String>,
    mcp: Option<String>,
    tool_name: String,
    #[serde(default)]
    arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct InstanceHealthRequest {
    instance_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct SessionRequest {
    instance_key: Option<String>,
    scope: Option<String>,
    #[serde(default = "default_session_limit")]
    limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct StopEditorRequest {
    instance_key: Option<String>,
    #[serde(default)]
    force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct RestartEditorRequest {
    #[serde(default = "default_wait_seconds")]
    wait_seconds: u64,
    #[serde(default)]
    force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct NotesResponse {
    notes: Vec<Note>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ToolListResponse {
    tools: Vec<ToolDescriptor>,
}

#[derive(Debug, Clone)]
pub struct UnrealFacade {
    tool_router: ToolRouter<Self>,
}

impl UnrealFacade {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

impl Default for UnrealFacade {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for UnrealFacade {}

#[tool_router(router = tool_router)]
impl UnrealFacade {
    #[tool(
        name = "setup_project",
        description = "Configure a UE project and persist it as the active project."
    )]
    async fn setup_project(
        &self,
        Parameters(request): Parameters<SetupProjectRequest>,
    ) -> Result<Json<orchestrator::ProjectSummary>, String> {
        let summary = orchestrator::setup_project(
            request.uproject_path.map(Into::into),
            request.engine_root.map(Into::into),
            request.name,
        )
        .await
        .map_err(to_tool_error)?;
        Ok(Json(summary))
    }

    #[tool(
        name = "get_project_config",
        description = "List configured Unreal projects."
    )]
    async fn get_project_config(&self) -> Result<Json<Vec<orchestrator::ProjectSummary>>, String> {
        Ok(Json(
            orchestrator::get_project_config().map_err(to_tool_error)?,
        ))
    }

    #[tool(
        name = "hub_status",
        description = "Get one-stop status for configured projects, instances, plugin source, and MCPHub submodule."
    )]
    async fn hub_status(&self) -> Result<Json<orchestrator::HubStatus>, String> {
        Ok(Json(orchestrator::hub_status().map_err(to_tool_error)?))
    }

    #[tool(
        name = "use_project",
        description = "Switch the active configured Unreal project."
    )]
    async fn use_project(
        &self,
        Parameters(request): Parameters<UseProjectRequest>,
    ) -> Result<String, String> {
        let switched = orchestrator::use_project(&request.project_name).map_err(to_tool_error)?;
        if switched {
            Ok(format!(
                "active project switched to {}",
                request.project_name
            ))
        } else {
            Err(format!("project '{}' not found", request.project_name))
        }
    }

    #[tool(
        name = "use_mcp",
        description = "Switch the active MCP target inside the active Unreal project."
    )]
    async fn use_mcp(
        &self,
        Parameters(request): Parameters<UseMcpRequest>,
    ) -> Result<String, String> {
        let switched = orchestrator::use_mcp(&request.mcp_id).map_err(to_tool_error)?;
        if switched {
            Ok(format!("active mcp switched to {}", request.mcp_id))
        } else {
            Err(format!("mcp '{}' not found", request.mcp_id))
        }
    }

    #[tool(
        name = "add_project_mcp",
        description = "Add or update one MCP target under a configured Unreal project."
    )]
    async fn add_project_mcp(
        &self,
        Parameters(request): Parameters<AddMcpRequest>,
    ) -> Result<Json<orchestrator::ProjectSummary>, String> {
        Ok(Json(
            orchestrator::add_project_mcp(
                request.project.as_deref(),
                &request.mcp_id,
                request.name.as_deref(),
                &request.host,
                request.port,
                &request.path,
                &request.transport,
                request.auto_start,
                request.activate,
            )
            .map_err(to_tool_error)?,
        ))
    }

    #[tool(
        name = "compile_project",
        description = "Compile the active Unreal project via UBT."
    )]
    async fn compile_project(
        &self,
        Parameters(request): Parameters<CompileProjectRequest>,
    ) -> Result<String, String> {
        orchestrator::compile_project(request.target, request.configuration)
            .await
            .map_err(to_tool_error)
    }

    #[tool(
        name = "launch_editor",
        description = "Launch UnrealEditor for the active project and optionally wait for MCP readiness."
    )]
    async fn launch_editor(
        &self,
        Parameters(request): Parameters<LaunchEditorRequest>,
    ) -> Result<Json<orchestrator::LaunchResult>, String> {
        Ok(Json(
            orchestrator::launch_editor(request.wait_seconds)
                .await
                .map_err(to_tool_error)?,
        ))
    }

    #[tool(
        name = "stop_editor",
        description = "Stop one UnrealEditor process by tracked instance or the active instance."
    )]
    async fn stop_editor(
        &self,
        Parameters(request): Parameters<StopEditorRequest>,
    ) -> Result<Json<orchestrator::StopEditorResult>, String> {
        Ok(Json(
            orchestrator::stop_editor(request.instance_key.as_deref(), request.force)
                .await
                .map_err(to_tool_error)?,
        ))
    }

    #[tool(
        name = "restart_editor",
        description = "Restart the active UnrealEditor and preserve crash/session context."
    )]
    async fn restart_editor(
        &self,
        Parameters(request): Parameters<RestartEditorRequest>,
    ) -> Result<Json<orchestrator::RestartResult>, String> {
        Ok(Json(
            orchestrator::restart_editor(request.wait_seconds, request.force)
                .await
                .map_err(to_tool_error)?,
        ))
    }

    #[tool(
        name = "discover_instances",
        description = "Probe configured MCP ports and register reachable Unreal instances."
    )]
    async fn discover_instances(&self) -> Result<Json<orchestrator::DiscoveryResult>, String> {
        Ok(Json(
            orchestrator::discover_instances()
                .await
                .map_err(to_tool_error)?,
        ))
    }

    #[tool(
        name = "use_editor",
        description = "Switch the active Unreal instance by instance key."
    )]
    async fn use_editor(
        &self,
        Parameters(request): Parameters<UseEditorRequest>,
    ) -> Result<String, String> {
        let switched = orchestrator::use_editor(&request.instance_key).map_err(to_tool_error)?;
        if switched {
            Ok(format!(
                "active editor switched to {}",
                request.instance_key
            ))
        } else {
            Err(format!("instance '{}' not found", request.instance_key))
        }
    }

    #[tool(
        name = "add_note",
        description = "Attach a session note to the active Unreal instance."
    )]
    async fn add_note(
        &self,
        Parameters(request): Parameters<AddNoteRequest>,
    ) -> Result<String, String> {
        orchestrator::add_note(&request.content).map_err(to_tool_error)?;
        Ok("note added".to_string())
    }

    #[tool(
        name = "get_notes",
        description = "List notes attached to the active Unreal instance."
    )]
    async fn get_notes(&self) -> Result<Json<NotesResponse>, String> {
        Ok(Json(NotesResponse {
            notes: orchestrator::get_notes().map_err(to_tool_error)?,
        }))
    }

    #[tool(
        name = "get_session",
        description = "Return notes and call history for one Unreal instance or the active instance."
    )]
    async fn get_session(
        &self,
        Parameters(request): Parameters<SessionRequest>,
    ) -> Result<Json<orchestrator::SessionReport>, String> {
        Ok(Json(
            orchestrator::get_session(
                request.instance_key.as_deref(),
                request.scope.as_deref(),
                request.limit,
            )
            .map_err(to_tool_error)?,
        ))
    }

    #[tool(
        name = "set_plugin_source",
        description = "Configure a local path or repo URL for the UnrealCopilot plugin source."
    )]
    async fn set_plugin_source(
        &self,
        Parameters(request): Parameters<SetPluginSourceRequest>,
    ) -> Result<String, String> {
        orchestrator::set_plugin_source(request.local_path.as_deref(), request.repo_url.as_deref())
            .map_err(to_tool_error)
    }

    #[tool(
        name = "install_plugin",
        description = "Install UnrealCopilot into the active project's Plugins directory."
    )]
    async fn install_plugin(&self) -> Result<String, String> {
        orchestrator::install_plugin().map_err(to_tool_error)
    }

    #[tool(
        name = "get_crash_report",
        description = "Read the latest crash report from the active project's Saved/Crashes directory."
    )]
    async fn get_crash_report(&self) -> Result<Json<Option<orchestrator::CrashReport>>, String> {
        Ok(Json(
            orchestrator::get_crash_report().map_err(to_tool_error)?,
        ))
    }

    #[tool(
        name = "get_instance_health",
        description = "Inspect process and MCP health for one Unreal instance or the active instance."
    )]
    async fn get_instance_health(
        &self,
        Parameters(request): Parameters<InstanceHealthRequest>,
    ) -> Result<Json<orchestrator::InstanceHealthReport>, String> {
        Ok(Json(
            orchestrator::get_instance_health(request.instance_key.as_deref())
                .await
                .map_err(to_tool_error)?,
        ))
    }

    #[tool(
        name = "list_tools",
        description = "List tools exposed by one configured MCP target, defaulting to the active project and mcp."
    )]
    async fn list_tools(
        &self,
        Parameters(request): Parameters<McpSelectorRequest>,
    ) -> Result<Json<ToolListResponse>, String> {
        Ok(Json(ToolListResponse {
            tools: orchestrator::list_tools(request.project.as_deref(), request.mcp.as_deref())
                .await
                .map_err(to_tool_error)?,
        }))
    }

    #[tool(
        name = "call_tool",
        description = "Call a tool on one configured MCP target, defaulting to the active project and mcp."
    )]
    async fn call_tool(
        &self,
        Parameters(request): Parameters<McpCallRequest>,
    ) -> Result<Json<orchestrator::EndpointToolEnvelope>, String> {
        Ok(Json(
            orchestrator::call_tool(
                request.project.as_deref(),
                request.mcp.as_deref(),
                &request.tool_name,
                as_object(request.arguments).map_err(to_tool_error)?,
            )
            .await
            .map_err(to_tool_error)?,
        ))
    }

    #[tool(
        name = "sync_mcphub",
        description = "Mirror one configured MCP target into the bundled generic MCPHub registry and refresh its catalog."
    )]
    async fn sync_mcphub(
        &self,
        Parameters(request): Parameters<McpSelectorRequest>,
    ) -> Result<String, String> {
        orchestrator::sync_mcphub(request.project.as_deref(), request.mcp.as_deref())
            .map_err(to_tool_error)
    }
}

pub async fn serve_stdio() -> Result<()> {
    let server = UnrealFacade::new();
    let transport = rmcp::transport::stdio();
    let running = server
        .serve(transport)
        .await
        .map_err(|error| anyhow!("failed to start UnrealMCPHub server: {error}"))?;
    let watcher = ProcessWatcher::spawn();
    let result = running.waiting().await;
    watcher.stop().await;
    result?;
    Ok(())
}

pub async fn serve_http(host: &str, port: u16) -> Result<()> {
    let cancellation = CancellationToken::new();
    let config = StreamableHttpServerConfig {
        stateful_mode: true,
        sse_keep_alive: None,
        cancellation_token: cancellation.child_token(),
        ..Default::default()
    };
    let service: StreamableHttpService<UnrealFacade, LocalSessionManager> =
        StreamableHttpService::new(|| Ok(UnrealFacade::new()), Default::default(), config);
    let app = Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(format!("{host}:{port}"))
        .await
        .map_err(|error| anyhow!("failed to bind HTTP listener on {host}:{port}: {error}"))?;
    let watcher = ProcessWatcher::spawn();
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            cancellation.cancel();
        })
        .await;
    watcher.stop().await;
    result.map_err(|error| anyhow!("failed to serve HTTP facade: {error}"))?;
    Ok(())
}

fn default_wait_seconds() -> u64 {
    180
}

fn default_session_limit() -> usize {
    50
}

fn default_mcp_path() -> String {
    "/mcp".to_string()
}

fn default_mcp_transport() -> String {
    "http".to_string()
}

fn as_object(value: Value) -> Result<Map<String, Value>> {
    match value {
        Value::Object(map) => Ok(map),
        Value::Null => Ok(Map::new()),
        other => Err(anyhow!(
            "expected an object for tool arguments, got {other}"
        )),
    }
}

fn to_tool_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
