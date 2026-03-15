# UnrealMCPHub

Additional documentation:

- [中文说明](README.zh-CN.md)

Standalone Unreal-focused hub that vendors
[`MCPHub`](https://github.com/syan2018/MCPHub) as a git submodule and provides
project lifecycle management, MCP discovery, and stable routing for the current
`UnrealCopilot` plugin.

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

## Repository Layout

- `src/`
  Standalone Rust binary and MCP server.
- `vendor/MCPHub/`
  Git submodule pointing at the published `MCPHub` repository.

## Capabilities

- dedicated git repository with `MCPHub` as a submodule
- persisted project config in `~/.unreal-mcphub/config.json`
- persisted instance/session state in `~/.unreal-mcphub/state.json`
- engine detection from `.uproject` and Windows registry
- best-effort auto-bind from the current working directory into the matching Unreal project
- configuration-driven MCP discovery strategies
- project setup, status, compile, launch, discover, use-project, and use-editor
- multiple configured MCP targets per project
- active MCP switching inside one project
- auto-discovery of the default project MCP plus manual registration of extra MCP targets
- instance discovery driven by configured project MCP targets
- stable active-instance tracking across editor stop and restart cycles
- plugin source config and local plugin install flow
- crash report lookup from `Saved/Crashes`
- session notes plus persisted call history and session snapshots
- background watcher during `serve` to refresh instance status and track crashes
- per-instance health inspection for MCP reachability and process liveness
- stdio and HTTP MCP facade serving modes
- editor stop and restart flows for recovery
- standard MCP forwarding through `list-tools`, `call-tool`, and `sync-mcphub`
- `sync-mcphub` bridge that mirrors the active Unreal MCP into bundled generic `MCPHub`
Config and state use the canonical field names only. Old field aliases are not
supported.

## Discovery Strategies

`discovery_strategies` is part of the intended user-facing config surface. The
default config now writes the built-in strategies into
`~/.unreal-mcphub/config.json` so users can copy and adapt them for their own
MCP plugins.

The extensibility rule is:

- keep project entries generic (`mcps`, `active_mcp`, endpoint host/port/path)
- keep plugin-specific discovery details inside `discovery_strategies`

That keeps `UnrealMCPHub` extensible without baking UnrealCopilot-specific
fields into every project record.

Example:

```json
{
  "projects": {},
  "active_project": "",
  "discovery_strategies": [
    {
      "name": "unrealcopilot",
      "config_files": [
        "Config/DefaultEditorPerProjectUserSettings.ini",
        "Saved/Config/WindowsEditor/EditorPerProjectUserSettings.ini"
      ],
      "section": "/Script/UnrealCopilot.UnrealCopilotSettings",
      "host_key": "McpHost",
      "port_key": "McpPort",
      "path_key": "McpPath",
      "transport_key": "Transport",
      "auto_start_key": "bAutoStartMcpServer",
      "default_port": 19840
    },
    {
      "name": "remote-mcp",
      "config_files": [
        "Config/DefaultEditorPerProjectUserSettings.ini",
        "Saved/Config/WindowsEditor/EditorPerProjectUserSettings.ini"
      ],
      "section": "/Script/RemoteMCP.MCPSetting",
      "enable_key": "bEnable",
      "port_key": "Port",
      "auto_start_key": "bAutoStart",
      "default_port": 8422
    }
  ]
}
```

To integrate another plugin MCP, add another strategy block that points at that
plugin's config files and keys. In most cases the fields mean:

- `config_files`: project-relative config files to scan in order
- `section`: INI section name that contains the MCP settings
- `enable_key`: optional boolean gate; when present and false, the endpoint is
  ignored
- `*_key`: optional field names to read from that section
- `default_port`: fallback port before a plugin-specific port override is found

Host defaults to `127.0.0.1`, path defaults to `/mcp`, transport defaults to
`http`, and auto-start defaults to `false`, so those do not need extra config
keys unless a plugin exposes overrides that UnrealMCPHub should read.

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
If that project is already present in `~/.unreal-mcphub/config.json`,
UnrealMCPHub now reuses the saved binding and only refreshes it when the
detected engine path or discovered MCP endpoints actually changed.
On Windows PowerShell, `call-tool --arguments-json` now accepts both strict
JSON and the de-quoted object syntax PowerShell often forwards to native
executables, but `ConvertTo-Json -Compress` is still the clearest way to pass
non-empty arguments.

For embedded Unreal plugin MCPs, `launch` and `verify-ue` can only connect once
the endpoint is actually running. If the discovered endpoint shows
`auto_start=false`, UnrealMCPHub can still launch the editor, but it cannot
make the plugin start its MCP server on your behalf. In that case, enable the
plugin's MCP auto-start setting in Unreal or start the MCP server manually
after the editor opens.

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

List tools with descriptions and input schemas:

```powershell
target\debug\unreal-mcphub.exe list-tools --json
```

Call one tool on the active MCP:

```powershell
target\debug\unreal-mcphub.exe call-tool get_dispatch --arguments-json "{}"
```

Call one tool with non-empty arguments from PowerShell:

```powershell
$args = @{ skill_name = "cpp_editor_api"; path = "docs/overview.md" } | ConvertTo-Json -Compress
target\debug\unreal-mcphub.exe call-tool read_unreal_skill --arguments-json "$args"
```

`run_unreal_skill` now honors its optional defaults correctly, so you only need
to send the fields you are actually using. Inline Python can be called with
just `python`:

```powershell
$args = @{
  python = "RESULT = {'ok': True, 'source': 'manual-cli-smoke'}"
} | ConvertTo-Json -Compress
target\debug\unreal-mcphub.exe call-tool run_unreal_skill --arguments-json "$args"
```

If the tool cache looks stale after a plugin update, refresh the catalog with
`discover` or `sync-mcphub`.

If you want the full synced tool surface with cached schemas and starter input
templates, prefer the bundled `mcphub.exe` CLI directly instead of exposing
another MCP facade layer:

```powershell
vendor\MCPHub\target\debug\mcphub.exe tool-info --all --json lyrastartergame-local
```

And if you only want one tool:

```powershell
vendor\MCPHub\target\debug\mcphub.exe tool-info lyrastartergame-local/run_unreal_skill --json
```

Show hub state:

```powershell
target\debug\unreal-mcphub.exe status
```

Launch the editor and wait for MCP:

```powershell
target\debug\unreal-mcphub.exe launch --wait-seconds 180
```

If the active project's tracked Unreal Editor process is already alive,
`launch` now reuses that instance instead of spawning a second editor for the
same project/MCP target.

If the returned JSON includes `health: null` or a note about `auto_start=false`,
the editor launched but the embedded endpoint never became reachable during the
wait window. That usually means the plugin is disabled, its MCP server is not
set to auto-start, or the server needs to be started manually inside the
editor.

Run a live Unreal verification pass against the active project:

```powershell
target\debug\unreal-mcphub.exe verify-ue --compile --wait-seconds 180
```

This uses a real Unreal Editor instance, waits for the embedded MCP to become
healthy, verifies the exposed tool surface, and exercises representative C++,
Blueprint, asset, skill, session, and `sync-mcphub` flows in one report.
`--wait-seconds` only applies to the "wait until the embedded MCP is healthy"
phase; the full verification can still take longer while live tool calls run.
On Windows, prefer writing the report directly instead of piping large JSON
through PowerShell:

```powershell
target\debug\unreal-mcphub.exe verify-ue --compile --wait-seconds 180 --output verify-ue-report.json
```

If you only want a concise terminal summary instead of the full JSON payload, use:

```powershell
target\debug\unreal-mcphub.exe verify-ue --compile --wait-seconds 180 --summary
```

`--summary` writes directly to stdout and is safe to use interactively in
PowerShell. `--output` remains the better option when you want a durable report
artifact or need to inspect the full JSON later.

Discover reachable instances:

```powershell
target\debug\unreal-mcphub.exe discover
```

Inspect one instance's health:

```powershell
target\debug\unreal-mcphub.exe health
target\debug\unreal-mcphub.exe health <project>:<mcp-id>:<port>
```

Inspect the persisted session snapshot:

```powershell
target\debug\unreal-mcphub.exe session --scope full --limit 20
target\debug\unreal-mcphub.exe session <project>:<mcp-id>:<port> --scope history --limit 50
```

Mirror the active Unreal MCP into bundled `MCPHub`:

```powershell
target\debug\unreal-mcphub.exe sync-mcphub
```

Stop the active Unreal Editor instance:

```powershell
target\debug\unreal-mcphub.exe stop
```

On Windows, `stop` first attempts a normal tree shutdown and automatically
falls back to a forced termination when child processes prevent a clean exit.
The saved instance state is also refreshed so stale PIDs do not continue to
appear as live after the editor is gone.

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

- `project`
  Actions: `status`, `setup`, `use_project`, `use_mcp`, `save_mcp`,
  `set_plugin_source`, `install_plugin`
- `editor`
  Actions: `compile`, `launch`, `stop`, `restart`, `discover`, `use`, `health`
- `session`
  Actions: `get`, `add_note`, `crash_report`
- `mcp`
  Actions: `list_tools`, `call_tool`, `sync`

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
target\debug\unreal-mcphub.exe verify-ue --compile --wait-seconds 180 --summary
```
