use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};

use crate::orchestrator;
use crate::server;

#[derive(Debug, Parser)]
#[command(name = "unreal-mcp-orchestrator")]
#[command(about = "Standalone Unreal-focused orchestration layer built on top of MCPHub")]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve(ServeArgs),
    Setup(SetupArgs),
    Status,
    Compile(CompileArgs),
    Launch(LaunchArgs),
    Discover,
    Health(HealthArgs),
    Session(SessionArgs),
    Stop(StopArgs),
    Restart(RestartArgs),
    UseProject(UseProjectArgs),
    UseEditor(UseEditorArgs),
    InstallPlugin,
    SetPluginSource(SetPluginSourceArgs),
    CrashReport,
    SyncMcphub,
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
struct SetPluginSourceArgs {
    #[arg(long)]
    local_path: Option<String>,
    #[arg(long)]
    repo_url: Option<String>,
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();
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
        Command::SyncMcphub => {
            println!("{}", orchestrator::sync_mcphub_endpoint()?);
            Ok(())
        }
    }
}
