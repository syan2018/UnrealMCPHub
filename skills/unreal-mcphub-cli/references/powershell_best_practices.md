# PowerShell Best Practices

Use this reference when calling `unreal-mcphub.exe` or Unreal MCP tools from
Windows PowerShell.

## Preferred Patterns

- Build non-empty `--arguments-json` payloads with `ConvertTo-Json -Compress`.
- Keep the final payload in a variable before invoking the exe.
- Use `@{}` for object payloads and `@()` for arrays before JSON conversion.
- For empty arguments, pass `"{}"` directly.
- For `run_unreal_skill`, send only the fields used by the chosen mode.
- For long inline Python, prefer a temporary `.py` file plus the `script` field
  over one giant shell string.

## Reliable Examples

Empty object:

```powershell
target\debug\unreal-mcphub.exe call-tool list_unreal_skill --arguments-json "{}"
```

Structured object:

```powershell
$args = @{
  skill_name = "cpp_editor_api"
  path = "docs/overview.md"
} | ConvertTo-Json -Compress

target\debug\unreal-mcphub.exe call-tool read_unreal_skill --arguments-json "$args"
```

Inline Python:

```powershell
$args = @{
  python = "RESULT = {'ok': True, 'source': 'powershell-inline'}"
} | ConvertTo-Json -Compress

target\debug\unreal-mcphub.exe call-tool run_unreal_skill --arguments-json "$args"
```

Script file:

```powershell
$scriptPath = Join-Path $env:TEMP "mcphub-smoke.py"
@'
RESULT = {"ok": True, "source": "temp-script"}
'@ | Set-Content -Path $scriptPath -Encoding utf8

$args = @{
  script = $scriptPath
} | ConvertTo-Json -Compress

target\debug\unreal-mcphub.exe call-tool run_unreal_skill --arguments-json "$args"
```

## Avoid

- Do not hand-write large nested JSON strings when PowerShell can build them.
- Do not rely on extra shell-wrapper fallbacks.
- Do not send unused `run_unreal_skill` fields "just in case".
- Do not embed multiline Python directly in the command line when a temp file is
  simpler.
