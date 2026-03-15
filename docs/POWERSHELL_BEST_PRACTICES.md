# PowerShell Best Practices

`UnrealMCPHub` works well from Windows PowerShell, but native executable
argument passing gets fragile once JSON, nested objects, or inline Python are
involved. This guide documents the patterns that have been the most reliable in
real Unreal editor sessions.

## Core Rule

When a command expects `--arguments-json`, build a PowerShell object first and
let `ConvertTo-Json -Compress` produce the final string.

```powershell
$args = @{ skill_name = "cpp_editor_api"; path = "docs/overview.md" } |
  ConvertTo-Json -Compress
target\debug\unreal-mcphub.exe call-tool read_unreal_skill --arguments-json "$args"
```

Avoid hand-writing JSON in double-quoted PowerShell strings unless the payload
is trivial.

## Always Use `-Depth` For Nested Objects

`ConvertTo-Json` defaults to a shallow depth. Nested `args` payloads will be
truncated unless you raise it.

```powershell
$args = @{
  skill_name = "cpp_blueprint_write_api"
  script = "create_blueprint_with_components.py"
  args = @{
    asset_path = "/Game/MCPHubSmoke/BP_MyActor"
    components = @(
      @{ class = "StaticMeshComponent"; name = "Mesh" }
    )
  }
} | ConvertTo-Json -Compress -Depth 8
```

If the resulting JSON unexpectedly contains `"..."`, increase `-Depth`.

## Prefer Structured JSON Over Inline Escaping

This is the most readable and least error-prone pattern:

```powershell
$payload = @{
  foo = "bar"
  count = 3
} | ConvertTo-Json -Compress

target\debug\unreal-mcphub.exe call-tool some_tool --arguments-json "$payload"
```

This is harder to maintain and easier to break:

```powershell
target\debug\unreal-mcphub.exe call-tool some_tool --arguments-json "{\"foo\":\"bar\",\"count\":3}"
```

## `run_unreal_skill`: Send Only What You Use

`run_unreal_skill` now respects its optional defaults correctly, so unused
fields can be omitted.

Inline Python can be as small as:

```powershell
$args = @{
  python = "RESULT = @{ ok = $true }"
} | ConvertTo-Json -Compress -Depth 6

target\debug\unreal-mcphub.exe call-tool run_unreal_skill --arguments-json "$args"
```

Skill-script execution can omit `python` entirely:

```powershell
$args = @{
  skill_name = "cpp_blueprint_write_api"
  script = "create_blueprint_with_components.py"
  args = @{
    asset_path = "/Game/MCPHubSmoke/BP_MyActor"
  }
} | ConvertTo-Json -Compress -Depth 8
```

If a client still shows an older cached schema where all fields look required,
refresh the tool catalog with `discover` or `sync-mcphub`.

## For Longer Inline Python, Use A Temporary File

Passing multi-line Python directly through PowerShell into a native executable
is fragile. The most reliable workaround is:

1. Write a temporary `.py` file.
2. Save it as UTF-8 without BOM.
3. Pass a short inline loader through `python`.

Example:

```powershell
$scriptPath = Join-Path $env:TEMP "mcphub-inline.py"
$code = @'
import unreal

actor_subsystem = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
RESULT = {"ok": bool(actor_subsystem)}
'@
[System.IO.File]::WriteAllText(
  $scriptPath,
  $code,
  New-Object System.Text.UTF8Encoding($false)
)

$payload = @{
  args = @{ script_path = $scriptPath }
  python = "exec(open(ARGS['script_path'], encoding='utf-8').read())"
} | ConvertTo-Json -Compress -Depth 8

target\debug\unreal-mcphub.exe call-tool run_unreal_skill --arguments-json "$payload"
```

## UTF-8 Without BOM Matters

If the temporary Python file is written with a BOM, Unreal-side Python can fail
with:

```text
SyntaxError: invalid non-printable character U+FEFF
```

Use:

```powershell
New-Object System.Text.UTF8Encoding($false)
```

That `$false` disables the BOM.

## Use `cmd /s /c` As A Fallback For Awkward Payloads

When PowerShell native argument passing keeps splitting or re-quoting a long
payload, running the final command through `cmd /s /c` is often more stable.

```powershell
$payload = @{ args = @{}; python = "RESULT = {'ok': True}" } |
  ConvertTo-Json -Compress -Depth 6

cmd /s /c "target\debug\unreal-mcphub.exe call-tool run_unreal_skill --arguments-json $payload"
```

Use this as a fallback, not as the default for every command.

## Prefer `--summary` Or `--output` For Large Results

Large JSON responses are awkward to inspect in an interactive PowerShell
session. Two patterns are more practical:

Interactive summary:

```powershell
target\debug\unreal-mcphub.exe verify-ue --wait-seconds 180 --summary
```

Write the full report to a file:

```powershell
target\debug\unreal-mcphub.exe verify-ue --wait-seconds 180 --output verify-ue-report.json
```

This is usually easier than piping large JSON blobs around PowerShell.

## Recommended Command Patterns

Read a skill:

```powershell
$args = @{ skill_name = "cpp_editor_api"; path = "SKILL.md" } |
  ConvertTo-Json -Compress
target\debug\unreal-mcphub.exe call-tool read_unreal_skill --arguments-json "$args"
```

Run a skill script:

```powershell
$args = @{
  skill_name = "cpp_blueprint_write_api"
  script = "create_blueprint_with_components.py"
  args = @{
    asset_path = "/Game/MCPHubSmoke/BP_MyActor"
  }
} | ConvertTo-Json -Compress -Depth 8

target\debug\unreal-mcphub.exe call-tool run_unreal_skill --arguments-json "$args"
```

Run inline Python:

```powershell
$args = @{
  python = "RESULT = {'ok': True, 'source': 'manual-cli-smoke'}"
} | ConvertTo-Json -Compress -Depth 6

target\debug\unreal-mcphub.exe call-tool run_unreal_skill --arguments-json "$args"
```

## Quick Checklist

- Build `--arguments-json` with `ConvertTo-Json -Compress`
- Add `-Depth` whenever nested objects or arrays are involved
- Prefer PowerShell objects over hand-escaped JSON strings
- For `run_unreal_skill`, send only the fields your current mode actually uses
- For longer Python, write a temp file and `exec(...)` it
- Save temp Python files as UTF-8 without BOM
- Use `cmd /s /c` only when native PowerShell argv behavior becomes unreliable
- Prefer `--summary` or `--output` for large verification reports
