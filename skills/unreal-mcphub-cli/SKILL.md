---
name: unreal-mcphub-cli
description: "Use when the user wants to operate UnrealMCPHub through its CLI: bind a project, manage configured MCPs, inspect status or health, launch or restart the editor, list tools, call tools, or sync the active MCP into bundled MCPHub."
---

# UnrealMCPHub CLI

Use this skill when the task should be completed by running `UnrealMCPHub` commands instead of editing code directly.

## Quick Rules

- Prefer the built binary at `UnrealMCPHub\target\debug\unreal-mcphub.exe`.
- If the binary is missing or stale after code changes, run `cargo build` in `UnrealMCPHub` first.
- When the current working directory is already inside an Unreal project tree, rely on UnrealMCPHub's auto-bind behavior before adding extra selectors; it now reuses an existing saved binding and only refreshes it when the detected engine path or discovered MCP endpoints changed.
- For the common single-project, single-MCP flow, omit `--project` and `--mcp`.
- When `call-tool` is used, pass `--arguments-json` as a JSON object string.
- In Windows PowerShell, prefer building non-empty `--arguments-json` values with `ConvertTo-Json -Compress`; this build also accepts the de-quoted object syntax PowerShell often forwards to native executables.
- `run_unreal_skill` now honors optional defaults correctly; pass only the fields your current mode uses.
- Prefer `verify-ue --summary` for interactive terminal checks, and `verify-ue --output <file>` when a full JSON report should be preserved.
- `--wait-seconds` only controls how long UnrealMCPHub waits for the embedded MCP endpoint to become healthy; live verification steps can continue after that window.
- On Windows, `stop` automatically falls back to a forced process-tree termination if a graceful stop leaves child processes behind.
- If the active Unreal plugin endpoint is discovered with `auto_start=false`, treat `launch` or `verify-ue` failures to reach the endpoint as a likely plugin configuration issue first: the editor can start successfully while the embedded MCP stays offline until the plugin starts its server. Recommend enabling the plugin's MCP auto-start setting or starting the MCP manually inside Unreal.
- For PowerShell quoting, temp-script execution, and large-payload guidance, reference `UnrealMCPHub/docs/POWERSHELL_BEST_PRACTICES.md` or `UnrealMCPHub/docs/POWERSHELL_BEST_PRACTICES.zh-CN.md` instead of duplicating those details here.

## Primary Commands

- Bind or refresh the current project:
  `target\debug\unreal-mcphub.exe setup "<path-to.uproject>"`
- Inspect overall state:
  `target\debug\unreal-mcphub.exe status`
- Launch the editor and wait for MCP readiness:
  `target\debug\unreal-mcphub.exe launch --wait-seconds 180`
- Run a full live UE verification pass with a concise terminal summary:
  `target\debug\unreal-mcphub.exe verify-ue --compile --wait-seconds 180 --summary`
- Run a full live UE verification pass and save the full report:
  `target\debug\unreal-mcphub.exe verify-ue --compile --wait-seconds 180 --output verify-ue-report.json`
- Discover configured Unreal instances:
  `target\debug\unreal-mcphub.exe discover`
- Inspect the active instance or one explicit instance:
  `target\debug\unreal-mcphub.exe health`
  `target\debug\unreal-mcphub.exe session --scope full --limit 20`
- Explicit instance keys use the shape `<project>:<mcp-id>:<port>` and are best copied from `discover`.
- Switch the active project or active MCP:
  `target\debug\unreal-mcphub.exe use-project <project-name>`
  `target\debug\unreal-mcphub.exe use-mcp <mcp-id>`
- Register another MCP under a project:
  `target\debug\unreal-mcphub.exe add-mcp <mcp-id> --host 127.0.0.1 --port 19841 --path /mcp --activate`
- Forward generic MCP operations:
  `target\debug\unreal-mcphub.exe list-tools`
  `target\debug\unreal-mcphub.exe call-tool <tool-name> --arguments-json "{}"`
- Inspect the current tool surface with schema details:
  `target\debug\unreal-mcphub.exe list-tools --json`
- Forward generic MCP operations with non-empty PowerShell arguments:
  `$args = @{ skill_name = "cpp_editor_api"; path = "docs/overview.md" } | ConvertTo-Json -Compress`
  `target\debug\unreal-mcphub.exe call-tool read_unreal_skill --arguments-json "$args"`
- Mirror the selected MCP into bundled MCPHub:
  `target\debug\unreal-mcphub.exe sync-mcphub`
- After `sync-mcphub`, inspect the full synced tool surface without exposing extra facade tools:
  `vendor\MCPHub\target\debug\mcphub.exe tool-info --all --json <mcp-id>`
- To zoom in on one synced tool only:
  `vendor\MCPHub\target\debug\mcphub.exe tool-info <mcp-id>/<tool-name> --json`
- Stop the active editor instance:
  `target\debug\unreal-mcphub.exe stop`

## Working Style

1. Confirm whether the task is lifecycle management, MCP inspection, or tool forwarding.
2. Choose the narrowest CLI command that solves it.
3. If selectors are required, prefer `--project` and `--mcp`; otherwise let the active defaults resolve them.
4. For interactive verification, prefer concise terminal output first; only emit the full JSON when the user explicitly needs raw report details.
5. Report the concrete project name, MCP id, instance key, URL, or tool name that was used so the result is easy to audit.
