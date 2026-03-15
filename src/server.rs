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
use crate::watcher::ProcessWatcher;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
enum ProjectRequest {
    Status,
    Setup {
        uproject_path: Option<String>,
        engine_root: Option<String>,
        name: Option<String>,
    },
    UseProject {
        project_name: String,
    },
    UseMcp {
        mcp_id: String,
    },
    SaveMcp {
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
    },
    SetPluginSource {
        local_path: Option<String>,
        repo_url: Option<String>,
    },
    InstallPlugin,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
enum EditorRequest {
    Compile {
        target: Option<String>,
        configuration: Option<String>,
    },
    Launch {
        #[serde(default = "default_wait_seconds")]
        wait_seconds: u64,
    },
    Stop {
        instance_key: Option<String>,
        #[serde(default)]
        force: bool,
    },
    Restart {
        #[serde(default = "default_wait_seconds")]
        wait_seconds: u64,
        #[serde(default)]
        force: bool,
    },
    Discover,
    Use {
        instance_key: String,
    },
    Health {
        instance_key: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
enum SessionRequest {
    Get {
        instance_key: Option<String>,
        scope: Option<String>,
        #[serde(default = "default_session_limit")]
        limit: usize,
    },
    AddNote {
        content: String,
    },
    CrashReport,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
enum McpRequest {
    ListTools {
        project: Option<String>,
        mcp: Option<String>,
    },
    CallTool {
        project: Option<String>,
        mcp: Option<String>,
        tool_name: String,
        #[serde(default)]
        arguments: Value,
    },
    Sync {
        project: Option<String>,
        mcp: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ActionResponse {
    action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
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
        name = "project",
        description = "Project and config control for UnrealMCPHub. Actions: status, setup, use_project, use_mcp, save_mcp, set_plugin_source, install_plugin."
    )]
    async fn project(
        &self,
        Parameters(request): Parameters<ProjectRequest>,
    ) -> Result<Json<ActionResponse>, String> {
        match request {
            ProjectRequest::Status => action_data("status", orchestrator::hub_status()),
            ProjectRequest::Setup {
                uproject_path,
                engine_root,
                name,
            } => {
                let summary = orchestrator::setup_project(
                    uproject_path.map(Into::into),
                    engine_root.map(Into::into),
                    name,
                )
                .await
                .map_err(to_tool_error)?;
                action_data("setup", Ok(summary))
            }
            ProjectRequest::UseProject { project_name } => {
                let switched = orchestrator::use_project(&project_name).map_err(to_tool_error)?;
                if switched {
                    Ok(action_message(
                        "use_project",
                        format!("active project switched to {}", project_name),
                    ))
                } else {
                    Err(format!("project '{}' not found", project_name))
                }
            }
            ProjectRequest::UseMcp { mcp_id } => {
                let switched = orchestrator::use_mcp(&mcp_id).map_err(to_tool_error)?;
                if switched {
                    Ok(action_message(
                        "use_mcp",
                        format!("active mcp switched to {}", mcp_id),
                    ))
                } else {
                    Err(format!("mcp '{}' not found", mcp_id))
                }
            }
            ProjectRequest::SaveMcp {
                project,
                mcp_id,
                name,
                host,
                port,
                path,
                transport,
                auto_start,
                activate,
            } => action_data(
                "save_mcp",
                orchestrator::add_project_mcp(
                    project.as_deref(),
                    &mcp_id,
                    name.as_deref(),
                    &host,
                    port,
                    &path,
                    &transport,
                    auto_start,
                    activate,
                ),
            ),
            ProjectRequest::SetPluginSource {
                local_path,
                repo_url,
            } => Ok(action_message(
                "set_plugin_source",
                orchestrator::set_plugin_source(local_path.as_deref(), repo_url.as_deref())
                    .map_err(to_tool_error)?,
            )),
            ProjectRequest::InstallPlugin => Ok(action_message(
                "install_plugin",
                orchestrator::install_plugin().map_err(to_tool_error)?,
            )),
        }
    }

    #[tool(
        name = "editor",
        description = "Editor lifecycle and instance control. Actions: compile, launch, stop, restart, discover, use, health."
    )]
    async fn editor(
        &self,
        Parameters(request): Parameters<EditorRequest>,
    ) -> Result<Json<ActionResponse>, String> {
        match request {
            EditorRequest::Compile {
                target,
                configuration,
            } => action_data(
                "compile",
                orchestrator::compile_project(target, configuration).await,
            ),
            EditorRequest::Launch { wait_seconds } => {
                action_data("launch", orchestrator::launch_editor(wait_seconds).await)
            }
            EditorRequest::Stop {
                instance_key,
                force,
            } => action_data(
                "stop",
                orchestrator::stop_editor(instance_key.as_deref(), force).await,
            ),
            EditorRequest::Restart {
                wait_seconds,
                force,
            } => action_data(
                "restart",
                orchestrator::restart_editor(wait_seconds, force).await,
            ),
            EditorRequest::Discover => {
                action_data("discover", orchestrator::discover_instances().await)
            }
            EditorRequest::Use { instance_key } => {
                let switched = orchestrator::use_editor(&instance_key).map_err(to_tool_error)?;
                if switched {
                    Ok(action_message(
                        "use",
                        format!("active editor switched to {}", instance_key),
                    ))
                } else {
                    Err(format!("instance '{}' not found", instance_key))
                }
            }
            EditorRequest::Health { instance_key } => action_data(
                "health",
                orchestrator::get_instance_health(instance_key.as_deref()).await,
            ),
        }
    }

    #[tool(
        name = "session",
        description = "Session notes and crash context for the active or selected Unreal instance. Actions: get, add_note, crash_report."
    )]
    async fn session(
        &self,
        Parameters(request): Parameters<SessionRequest>,
    ) -> Result<Json<ActionResponse>, String> {
        match request {
            SessionRequest::Get {
                instance_key,
                scope,
                limit,
            } => action_data(
                "get",
                orchestrator::get_session(instance_key.as_deref(), scope.as_deref(), limit),
            ),
            SessionRequest::AddNote { content } => {
                orchestrator::add_note(&content).map_err(to_tool_error)?;
                Ok(action_message("add_note", "note added"))
            }
            SessionRequest::CrashReport => {
                action_data("crash_report", orchestrator::get_crash_report())
            }
        }
    }

    #[tool(
        name = "mcp",
        description = "Forward or sync the active Unreal MCP target. Actions: list_tools, call_tool, sync."
    )]
    async fn mcp(
        &self,
        Parameters(request): Parameters<McpRequest>,
    ) -> Result<Json<ActionResponse>, String> {
        match request {
            McpRequest::ListTools { project, mcp } => action_data(
                "list_tools",
                orchestrator::list_tools(project.as_deref(), mcp.as_deref()).await,
            ),
            McpRequest::CallTool {
                project,
                mcp,
                tool_name,
                arguments,
            } => action_data(
                "call_tool",
                orchestrator::call_tool(
                    project.as_deref(),
                    mcp.as_deref(),
                    &tool_name,
                    as_object(arguments).map_err(to_tool_error)?,
                )
                .await,
            ),
            McpRequest::Sync { project, mcp } => Ok(action_message(
                "sync",
                orchestrator::sync_mcphub(project.as_deref(), mcp.as_deref())
                    .await
                    .map_err(to_tool_error)?,
            )),
        }
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

fn action_data<T>(action: &str, result: anyhow::Result<T>) -> Result<Json<ActionResponse>, String>
where
    T: Serialize,
{
    let data = serde_json::to_value(result.map_err(to_tool_error)?).map_err(to_tool_error)?;
    Ok(Json(ActionResponse {
        action: action.to_string(),
        message: None,
        data: Some(data),
    }))
}

fn action_message(action: &str, message: impl Into<String>) -> Json<ActionResponse> {
    Json(ActionResponse {
        action: action.to_string(),
        message: Some(message.into()),
        data: None,
    })
}

fn to_tool_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
