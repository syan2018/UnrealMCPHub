# UnrealMCPHub

Additional documentation:

- [中文说明](README.zh-CN.md)
- [Feature Parity](docs/FEATURE_PARITY.md)
- [功能对齐状态](docs/FEATURE_PARITY.zh-CN.md)

Standalone Unreal-focused hub that vendors
[`MCPHub`](https://github.com/syan2018/MCPHub) as a git submodule and rebuilds
the earlier Python Unreal hub workflow around the current `UnrealCopilot`
plugin.

`UnrealMCPHub` is positioned as a lifecycle-aware Unreal hub:

- manage Unreal project lifecycle through either CLI or MCP tools
- discover configured MCP interfaces that are embedded into Unreal projects or
  plugins
- auto-bind the current project when launched inside an Unreal project tree
- reconnect and keep a stable control surface across Unreal editor restarts
- bridge the active Unreal MCP into the bundled generic `MCPHub`

## Design

This project separates concerns into two layers:

- `UnrealMCPHub`
  Unreal-aware lifecycle, project config, editor launch, plugin install, notes,
  crash lookup, MCP discovery, and MCP routing.
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
- persisted project config in `~/.unreal-mcphub/config.json`
- persisted instance/session state in `~/.unreal-mcphub/state.json`
- engine detection from `.uproject` and Windows registry
- best-effort auto-bind from the current working directory into the matching Unreal project
- configuration-driven MCP discovery strategies with a default UnrealCopilot strategy
- project setup, status, compile, launch, discover, use-project, use-editor
- multiple configured MCP targets per project
- active MCP switching inside one project
- auto-discovery of the default project MCP plus manual registration of extra MCP targets
- instance discovery driven by configured project MCP targets
- stable active-instance tracking across editor stop / restart cycles
- plugin source config and local plugin install flow
- crash report lookup from `Saved/Crashes`
- session notes plus persisted call history / session snapshots
- background watcher during `serve` to refresh instance status and track crashes
- per-instance health inspection for MCP reachability and process liveness
- stdio and HTTP MCP facade serving modes
- editor stop and restart flows for crash recovery
- standard MCP forwarding through `list-tools`, `call-tool`, and `sync-mcphub`
- stdio MCP facade with the orchestration tools above
- `sync-mcphub` bridge that mirrors the active Unreal MCP into bundled
  generic `MCPHub` via `register-http` + `discover`

Not implemented yet:

- richer plugin-specific discovery strategies beyond the default UnrealCopilot setup
- zip/GitHub plugin download pipeline

## Build

```powershell
cd UnrealMCPHub
cargo build
```

## Syncing Bundled MCPHub

`vendor/MCPHub` is a normal git submodule pointing at the upstream MCPHub
repository. The intended sync flow is to update that submodule from upstream
first, then commit the new submodule pointer in `UnrealMCPHub`.

```powershell
git submodule update --remote vendor/MCPHub
git add vendor/MCPHub
git commit -m "chore: bump bundled mcphub"
```

## CLI Quick Start

Configure the current project:

```powershell
target\debug\unreal-mcphub.exe setup "D:\Projects\Games\Unreal Projects\LyraStarterGame\LyraStarterGame.uproject"
```

When launched inside a directory that belongs to a UE project, UnrealMCPHub
will also try to bind that project automatically before running the command.

Add another MCP under the active project:

```powershell
target\debug\unreal-mcphub.exe add-mcp tools-secondary --host 127.0.0.1 --port 19841 --path /mcp --activate
```

Switch the active MCP inside the current project:

```powershell
target\debug\unreal-mcphub.exe use-mcp tools-secondary
```

List tools on the active MCP:

```powershell
target\debug\unreal-mcphub.exe list-tools
```

Call one tool on the active MCP:

```powershell
target\debug\unreal-mcphub.exe call-tool get_dispatch --arguments-json "{}"
```

Show hub state:

```powershell
target\debug\unreal-mcphub.exe status
```

Launch the editor and wait for MCP:

```powershell
target\debug\unreal-mcphub.exe launch --wait-seconds 180
```

Discover reachable instances:

```powershell
target\debug\unreal-mcphub.exe discover
```

Inspect one instance's health:

```powershell
target\debug\unreal-mcphub.exe health
target\debug\unreal-mcphub.exe health LyraStarterGame:19840
```

Inspect the persisted session snapshot:

```powershell
target\debug\unreal-mcphub.exe session --scope full --limit 20
target\debug\unreal-mcphub.exe session LyraStarterGame:19840 --scope history --limit 50
```

Mirror the active Unreal MCP into bundled `MCPHub`:

```powershell
target\debug\unreal-mcphub.exe sync-mcphub
```

## MCP Server

Run UnrealMCPHub itself as a stdio MCP server:

```powershell
target\debug\unreal-mcphub.exe serve
```

Run UnrealMCPHub as an HTTP MCP server:

```powershell
target\debug\unreal-mcphub.exe serve --http --host 127.0.0.1 --port 9422
```

Current MCP tools:

- `setup_project`
- `get_project_config`
- `hub_status`
- `use_project`
- `use_mcp`
- `add_project_mcp`
- `list_tools`
- `call_tool`
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
- `sync_mcphub`

## Verified Lyra Flow

This repository was smoke-tested against:

- project:
  `D:\Projects\Games\Unreal Projects\LyraStarterGame\LyraStarterGame.uproject`
- engine:
  `D:\Epic Games\UE_5.7`
- UnrealCopilot MCP:
  `http://127.0.0.1:19840/mcp`

Verified commands:

```powershell
target\debug\unreal-mcphub.exe setup "D:\Projects\Games\Unreal Projects\LyraStarterGame\LyraStarterGame.uproject"
target\debug\unreal-mcphub.exe launch --wait-seconds 180
target\debug\unreal-mcphub.exe status
target\debug\unreal-mcphub.exe sync-mcphub
```
