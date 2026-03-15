# PowerShell 最佳实践

`UnrealMCPHub` 在 Windows PowerShell 下可以很好地工作，但一旦命令参数里出现
JSON、嵌套对象或者 inline Python，原生 exe 的参数传递就会开始变脆。这里把当前
最稳定、最值得复用的写法整理出来，尽量避免再踩同一类坑。

## 核心原则

凡是命令需要 `--arguments-json`，优先先在 PowerShell 里构造对象，再交给
`ConvertTo-Json -Compress` 生成最终字符串。

```powershell
$args = @{ skill_name = "cpp_editor_api"; path = "docs/overview.md" } |
  ConvertTo-Json -Compress
target\debug\unreal-mcphub.exe call-tool read_unreal_skill --arguments-json "$args"
```

除非 payload 非常短，否则不要手写带转义的 JSON 字符串。

## 嵌套对象一定要配 `-Depth`

`ConvertTo-Json` 默认深度很浅。只要参数里再嵌套一层 `args`，就可能被截断。

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

如果生成的 JSON 里出现 `"..."`，通常就是 `-Depth` 不够。

## 尽量传结构化 JSON，不要硬写转义串

推荐：

```powershell
$payload = @{
  foo = "bar"
  count = 3
} | ConvertTo-Json -Compress

target\debug\unreal-mcphub.exe call-tool some_tool --arguments-json "$payload"
```

不推荐：

```powershell
target\debug\unreal-mcphub.exe call-tool some_tool --arguments-json "{\"foo\":\"bar\",\"count\":3}"
```

后者很容易在引号、转义和阅读性上同时出问题。

## `run_unreal_skill` 现在按需传字段就行

`run_unreal_skill` 现在已经会正确遵守 optional 默认值，不需要再为了凑 schema
额外补一堆没用的 `null` 字段。

如果只是跑 inline Python，最简单可以这样传：

```powershell
$args = @{
  python = "RESULT = @{ ok = $true }"
} | ConvertTo-Json -Compress -Depth 6

target\debug\unreal-mcphub.exe call-tool run_unreal_skill --arguments-json "$args"
```

如果只是跑 skill script，也可以直接省掉 `python`：

```powershell
$args = @{
  skill_name = "cpp_blueprint_write_api"
  script = "create_blueprint_with_components.py"
  args = @{
    asset_path = "/Game/MCPHubSmoke/BP_MyActor"
  }
} | ConvertTo-Json -Compress -Depth 8
```

如果插件更新后工具缓存看起来还是旧的，重新执行一次 `discover` 或
`sync-mcphub` 刷新工具目录即可。

## 长一点的 inline Python，改用临时文件

把多行 Python 直接塞进 PowerShell 再传给原生 exe，很容易被引号和参数拆分坑到。
当前最可靠的方式是：

1. 先写一个临时 `.py` 文件
2. 用 UTF-8 无 BOM 保存
3. 在 `python` 字段里只传一个很短的 loader

示例：

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

## UTF-8 无 BOM 很重要

如果临时 Python 文件带了 BOM，Unreal 侧 Python 可能会直接报：

```text
SyntaxError: invalid non-printable character U+FEFF
```

因此写文件时要明确使用：

```powershell
New-Object System.Text.UTF8Encoding($false)
```

这里的 `$false` 就是关闭 BOM。

## 复杂 payload 可以退回 `cmd /s /c`

当 PowerShell 原生参数传递一直在拆字符串、重写引号、或者把长 payload 传坏时，
`cmd /s /c` 往往会更稳定。

```powershell
$payload = @{ args = @{}; python = "RESULT = {'ok': True}" } |
  ConvertTo-Json -Compress -Depth 6

cmd /s /c "target\debug\unreal-mcphub.exe call-tool run_unreal_skill --arguments-json $payload"
```

它适合作为兜底方案，不建议一开始所有命令都这么写。

## 大结果优先用 `--summary` 或 `--output`

很长的 JSON 直接在 PowerShell 终端里看并不舒服，更推荐两种方式：

终端里看摘要：

```powershell
target\debug\unreal-mcphub.exe verify-ue --wait-seconds 180 --summary
```

完整报告直接落盘：

```powershell
target\debug\unreal-mcphub.exe verify-ue --wait-seconds 180 --output verify-ue-report.json
```

通常比在 PowerShell 里折腾大段 JSON 更省心。

## 推荐命令模板

读取 skill：

```powershell
$args = @{ skill_name = "cpp_editor_api"; path = "SKILL.md" } |
  ConvertTo-Json -Compress
target\debug\unreal-mcphub.exe call-tool read_unreal_skill --arguments-json "$args"
```

运行 skill script：

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

运行 inline Python：

```powershell
$args = @{
  python = "RESULT = {'ok': True, 'source': 'manual-cli-smoke'}"
} | ConvertTo-Json -Compress -Depth 6

target\debug\unreal-mcphub.exe call-tool run_unreal_skill --arguments-json "$args"
```

## 速查清单

- `--arguments-json` 用 `ConvertTo-Json -Compress` 生成
- 只要有嵌套对象或数组，就加 `-Depth`
- 优先传 PowerShell 对象，不要手写一大串转义 JSON
- `run_unreal_skill` 按你当前模式需要的字段来传，不用再补无意义的 `null`
- 长 Python 代码先写临时文件，再用 `exec(...)` loader
- 临时 Python 文件一定要用 UTF-8 无 BOM
- PowerShell 原生 argv 不稳定时，再退回 `cmd /s /c`
- 大报告优先用 `--summary` 或 `--output`
