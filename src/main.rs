mod cli;
mod config;
mod json_args;
mod orchestrator;
mod paths;
mod process;
mod server;
mod state;
mod submodule;
mod ue_client;
mod watcher;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    cli::run().await
}
