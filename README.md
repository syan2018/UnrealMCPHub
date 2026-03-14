# UnrealMCPOrchestrator

Standalone Unreal-focused orchestration layer that vendors
[`MCPHub`](https://github.com/syan2018/MCPHub) as a git submodule and rebuilds
the older `UnrealMCPHub` workflow around the current `UnrealCopilot` plugin.

## Design

This project separates concerns into two layers:

- `UnrealMCPOrchestrator`
  Unreal-aware lifecycle, project config, editor launch, plugin install, notes,
  crash lookup, and UE proxy tools.
- `vendor/MCPHub`
  Generic MCP registry, discovery, and reusable upstream hub logic, consumed as
  a git submodule.

The goal is to preserve the old UnrealHub user experience while moving the
generic MCP substrate into the reusable Rust `MCPHub` project.

## Repository Layout

- `src/`
  Standalone Rust binary and MCP server.
- `vendor/MCPHub/`
  Git submodule pointing at the published `MCPHub` repository.
- `docs/FEATURE_PARITY.md`
  Migration and parity tracking against the older Python hub.

## Current Status

Implemented in this first standalone slice:

- dedicated git repository with `MCPHub` as a submodule
- persisted project config in `~/.unreal-mcp-orchestrator/config.json`
- persisted instance/session state in `~/.unreal-mcp-orchestrator/state.json`
- engine detection from `.uproject` and Windows registry
- UnrealCopilot transport discovery from project config
- project setup, status, compile, launch, discover, use-project, use-editor
- dynamic discovery seeded by configured projects, known instances, and scan ports
- plugin source config and local plugin install flow
- crash report lookup from `Saved/Crashes`
- session notes plus persisted call history / session snapshots
- background watcher during `serve` to refresh instance status and track crashes
- per-instance health inspection for MCP endpoint reachability and process liveness
- stdio and HTTP MCP facade serving modes
- editor stop and restart flows for crash recovery
- UE proxy calls:
  - `ue_status`
  - `ue_list_tools`
  - `ue_call`
  - `ue_run_python`
  - `ue_get_dispatch`
  - `ue_call_dispatch`
- stdio MCP facade with the orchestration tools above
- `sync-mcphub` bridge that mirrors the active UE endpoint into bundled
  generic `MCPHub` via `register-http` + `discover`

Not implemented yet:

- richer process discovery for unrelated UE projects that are not yet configured
- zip/GitHub plugin download pipeline

## Build

```powershell
cd UnrealMCPOrchestrator
cargo build
```

## CLI Quick Start

Configure the current project:

```powershell
target\debug\unreal-mcp-orchestrator.exe setup "D:\Projects\Games\Unreal Projects\LyraStarterGame\LyraStarterGame.uproject"
```

Show orchestrator state:

```powershell
target\debug\unreal-mcp-orchestrator.exe status
```

Launch the editor and wait for MCP:

```powershell
target\debug\unreal-mcp-orchestrator.exe launch --wait-seconds 180
```

Discover reachable instances:

```powershell
target\debug\unreal-mcp-orchestrator.exe discover
```

Inspect one instance's health:

```powershell
target\debug\unreal-mcp-orchestrator.exe health
target\debug\unreal-mcp-orchestrator.exe health LyraStarterGame:19840
```

Inspect the persisted session snapshot:

```powershell
target\debug\unreal-mcp-orchestrator.exe session --scope full --limit 20
target\debug\unreal-mcp-orchestrator.exe session LyraStarterGame:19840 --scope history --limit 50
```

Mirror the active project into bundled `MCPHub`:

```powershell
target\debug\unreal-mcp-orchestrator.exe sync-mcphub
```

## MCP Server

Run the orchestrator itself as a stdio MCP server:

```powershell
target\debug\unreal-mcp-orchestrator.exe serve
```

Run the orchestrator as an HTTP MCP server:

```powershell
target\debug\unreal-mcp-orchestrator.exe serve --http --host 127.0.0.1 --port 9422
```

Current MCP tools:

- `setup_project`
- `get_project_config`
- `hub_status`
- `use_project`
- `compile_project`
- `launch_editor`
- `stop_editor`
- `restart_editor`
- `discover_instances`
- `use_editor`
- `add_note`
- `get_notes`
- `get_session`
- `set_plugin_source`
- `install_plugin`
- `get_crash_report`
- `get_instance_health`
- `ue_status`
- `ue_list_tools`
- `ue_call`
- `ue_run_python`
- `ue_get_dispatch`
- `ue_call_dispatch`
- `sync_mcphub_endpoint`

## Verified Lyra Flow

This repository was smoke-tested against:

- project:
  `D:\Projects\Games\Unreal Projects\LyraStarterGame\LyraStarterGame.uproject`
- engine:
  `D:\Epic Games\UE_5.7`
- UnrealCopilot endpoint:
  `http://127.0.0.1:19840/mcp`

Verified commands:

```powershell
target\debug\unreal-mcp-orchestrator.exe setup "D:\Projects\Games\Unreal Projects\LyraStarterGame\LyraStarterGame.uproject"
target\debug\unreal-mcp-orchestrator.exe launch --wait-seconds 180
target\debug\unreal-mcp-orchestrator.exe status
target\debug\unreal-mcp-orchestrator.exe sync-mcphub
```
