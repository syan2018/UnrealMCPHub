use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};

use crate::orchestrator;
use crate::server;

#[derive(Debug, Parser)]
#[command(name = "unreal-mcphub")]
#[command(about = "Standalone Unreal-focused hub built on top of MCPHub")]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve(ServeArgs),
    Setup(SetupArgs),
    Status,
    ListTools(ListToolsArgs),
    CallTool(CallToolArgs),
    Compile(CompileArgs),
    Launch(LaunchArgs),
    Discover,
    Health(HealthArgs),
    Session(SessionArgs),
    Stop(StopArgs),
    Restart(RestartArgs),
    UseProject(UseProjectArgs),
    UseMcp(UseMcpArgs),
    AddMcp(AddMcpArgs),
    UseEditor(UseEditorArgs),
    InstallPlugin,
    SetPluginSource(SetPluginSourceArgs),
    CrashReport,
    SyncMcphub(SyncMcphubArgs),
}

#[derive(Debug, Args)]
struct SetupArgs {
    #[arg()]
    path: Option<PathBuf>,
    #[arg(long)]
    engine: Option<PathBuf>,
    #[arg(long)]
    name: Option<String>,
}

#[derive(Debug, Args)]
struct ServeArgs {
    #[arg(long)]
    http: bool,
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 9422)]
    port: u16,
}

#[derive(Debug, Args)]
struct CompileArgs {
    #[arg(long)]
    target: Option<String>,
    #[arg(long)]
    configuration: Option<String>,
}

#[derive(Debug, Args)]
struct ListToolsArgs {
    #[arg(long)]
    project: Option<String>,
    #[arg(long)]
    mcp: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct CallToolArgs {
    tool_name: String,
    #[arg(long, default_value = "{}")]
    arguments_json: String,
    #[arg(long)]
    project: Option<String>,
    #[arg(long)]
    mcp: Option<String>,
}

#[derive(Debug, Args)]
struct LaunchArgs {
    #[arg(long, default_value_t = 180)]
    wait_seconds: u64,
}

#[derive(Debug, Args)]
struct HealthArgs {
    #[arg()]
    instance: Option<String>,
}

#[derive(Debug, Args)]
struct SessionArgs {
    #[arg()]
    instance: Option<String>,
    #[arg(long, default_value = "full")]
    scope: String,
    #[arg(long, default_value_t = 50)]
    limit: usize,
}

#[derive(Debug, Args)]
struct StopArgs {
    #[arg()]
    instance: Option<String>,
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
struct RestartArgs {
    #[arg(long, default_value_t = 180)]
    wait_seconds: u64,
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
struct UseEditorArgs {
    instance_key: String,
}

#[derive(Debug, Args)]
struct UseProjectArgs {
    project_name: String,
}

#[derive(Debug, Args)]
struct UseMcpArgs {
    mcp_id: String,
}

#[derive(Debug, Args)]
struct AddMcpArgs {
    mcp_id: String,
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    host: String,
    #[arg(long)]
    port: u16,
    #[arg(long, default_value = "/mcp")]
    path: String,
    #[arg(long, default_value = "http")]
    transport: String,
    #[arg(long)]
    project: Option<String>,
    #[arg(long)]
    auto_start: bool,
    #[arg(long)]
    activate: bool,
}

#[derive(Debug, Args)]
struct SetPluginSourceArgs {
    #[arg(long)]
    local_path: Option<String>,
    #[arg(long)]
    repo_url: Option<String>,
}

#[derive(Debug, Args)]
struct SyncMcphubArgs {
    #[arg(long)]
    project: Option<String>,
    #[arg(long)]
    mcp: Option<String>,
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    if let Err(error) = orchestrator::bind_project_from_current_dir().await {
        eprintln!("warning: failed to auto-bind current Unreal project: {error}");
    }
    match cli.command {
        Command::Serve(args) => {
            if args.http {
                server::serve_http(&args.host, args.port).await
            } else {
                server::serve_stdio().await
            }
        }
        Command::Setup(args) => {
            let summary = orchestrator::setup_project(args.path, args.engine, args.name).await?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Command::Status => {
            let status = orchestrator::hub_status()?;
            println!("{}", serde_json::to_string_pretty(&status)?);
            Ok(())
        }
        Command::ListTools(args) => {
            let tools = orchestrator::list_tools(args.project.as_deref(), args.mcp.as_deref()).await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&tools)?);
            } else {
                for tool in tools {
                    println!("{}", tool.name);
                }
            }
            Ok(())
        }
        Command::CallTool(args) => {
            let arguments = serde_json::from_str(&args.arguments_json)?;
            let output = orchestrator::call_tool(
                args.project.as_deref(),
                args.mcp.as_deref(),
                &args.tool_name,
                match arguments {
                    serde_json::Value::Object(map) => map,
                    serde_json::Value::Null => serde_json::Map::new(),
                    other => anyhow::bail!("expected JSON object for --arguments-json, got {other}"),
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&output)?);
            Ok(())
        }
        Command::Compile(args) => {
            let output = orchestrator::compile_project(args.target, args.configuration).await?;
            println!("{output}");
            Ok(())
        }
        Command::Launch(args) => {
            let result = orchestrator::launch_editor(args.wait_seconds).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Command::Discover => {
            let result = orchestrator::discover_instances().await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Command::Health(args) => {
            let report = orchestrator::get_instance_health(args.instance.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Command::Session(args) => {
            let report =
                orchestrator::get_session(args.instance.as_deref(), Some(&args.scope), args.limit)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Command::Stop(args) => {
            let report = orchestrator::stop_editor(args.instance.as_deref(), args.force).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Command::Restart(args) => {
            let report = orchestrator::restart_editor(args.wait_seconds, args.force).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Command::UseProject(args) => {
            let switched = orchestrator::use_project(&args.project_name)?;
            println!("{}", if switched { "switched" } else { "not-found" });
            Ok(())
        }
        Command::UseMcp(args) => {
            let switched = orchestrator::use_mcp(&args.mcp_id)?;
            println!("{}", if switched { "switched" } else { "not-found" });
            Ok(())
        }
        Command::AddMcp(args) => {
            let summary = orchestrator::add_project_mcp(
                args.project.as_deref(),
                &args.mcp_id,
                args.name.as_deref(),
                &args.host,
                args.port,
                &args.path,
                &args.transport,
                args.auto_start,
                args.activate,
            )?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Command::UseEditor(args) => {
            let switched = orchestrator::use_editor(&args.instance_key)?;
            println!("{}", if switched { "switched" } else { "not-found" });
            Ok(())
        }
        Command::InstallPlugin => {
            println!("{}", orchestrator::install_plugin()?);
            Ok(())
        }
        Command::SetPluginSource(args) => {
            println!(
                "{}",
                orchestrator::set_plugin_source(
                    args.local_path.as_deref(),
                    args.repo_url.as_deref()
                )?
            );
            Ok(())
        }
        Command::CrashReport => {
            println!(
                "{}",
                serde_json::to_string_pretty(&orchestrator::get_crash_report()?)?
            );
            Ok(())
        }
        Command::SyncMcphub(args) => {
            println!(
                "{}",
                orchestrator::sync_mcphub(
                    args.project.as_deref(),
                    args.mcp.as_deref()
                )?
            );
            Ok(())
        }
    }
}
