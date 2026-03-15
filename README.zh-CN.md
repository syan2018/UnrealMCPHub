# UnrealMCPHub

更多文档：

- [English README](README.md)

`UnrealMCPHub` 是一个面向 Unreal 的生命周期 Hub。它 vendored 了通用
[`MCPHub`](https://github.com/syan2018/MCPHub) 作为 git submodule，并在此之上
实现 Unreal 项目、Editor 进程和内嵌 MCP 接口的统一管理。

这个项目的核心目标不是只做一个启动器，也不是只做一个代理，而是提供一个
稳定的控制中枢：

- 通过 CLI 或 MCP tools 管理 Unreal 项目的生命周期
- 根据配置识别和发现嵌入在 Unreal 工程或插件中的 MCP 接口
- 在 Unreal 工程目录中启动时，自动尝试绑定当前 project
- 在 Unreal Editor 重启、崩溃恢复、活动实例切换时维持稳定连接语义
- 把当前活动的 Unreal MCP 同步到 UnrealMCPHub 内部使用的
  MCPHub catalog / runtime

## 架构分层

这个仓库现在明确分成两层：

- `UnrealMCPHub`
  Unreal-aware 的上层 Hub，负责项目配置、Editor 生命周期、实例发现、会话状态、
  崩溃恢复、MCP 发现与路由，以及对外 MCP/CLI 表面。
- `vendor/MCPHub`
  通用的 MCP registry、discovery、invoke 与 runtime/library 能力，作为可复用
  底座存在。

这意味着边界是清晰的：

- 通用 MCP 能力放在 `MCPHub`
- Unreal 专属能力放在 `UnrealMCPHub`

## 能力概览

- 独立 Rust 仓库，内含 `MCPHub` submodule
- 持久化项目配置：`~/.unreal-mcphub/config.json`
- 持久化实例与会话状态：`~/.unreal-mcphub/state.json`
- 从 `.uproject` 与 Windows 注册表自动探测引擎
- 从当前工作目录 best-effort 自动绑定匹配的 Unreal project
- 配置驱动的 MCP discovery strategy
- `setup`、`status`、`compile`、`launch`、`discover`、`use-project`、`use-editor`
- 单个 project 下可配置多个 MCP
- 支持 project 内 active MCP 切换
- 自动读取 project 默认 MCP，并允许手工补充额外 MCP
- 仅基于已配置 project 的 MCP 进行实例发现
- 跨 Editor 停止 / 重启的 active instance 追踪
- 插件源配置与本地复制式安装
- `Saved/Crashes` 崩溃摘要读取
- session notes、调用历史、session snapshot
- `serve` 生命周期内的 watcher，用于刷新实例状态、跟踪 crash、清理 stale instance
- 实例健康检查，覆盖 MCP reachability 与 process liveness
- stdio MCP server
- HTTP MCP server
- `stop_editor` / `restart_editor`
- 通过 `list-tools`、`call-tool`、`sync-mcphub` 提供标准 MCP 转发
- `sync-mcphub`，刷新 `list-tools` 与 `call-tool` 使用的内部
  MCPHub catalog

配置与状态文件只支持当前这套规范字段名，不提供旧字段别名。

## Discovery Strategies

`discovery_strategies` 是有意暴露给用户的配置扩展面。现在默认配置会把内置
strategies 直接写入 `~/.unreal-mcphub/config.json`，方便用户照着改，接入自己的
MCP 插件。

这里的设计原则是：

- project 记录保持通用，只保存 `mcps`、`active_mcp` 以及 endpoint 的基础信息
- 插件相关的发现逻辑放进 `discovery_strategies`

这样扩展新插件时，不需要把 UnrealCopilot 专属字段塞进每个 project 配置里。

示例：

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

如果要接入别的插件 MCP，就新增一个 strategy block，指向它自己的配置文件和键名。
这些字段一般表示：

- `config_files`: 按顺序扫描的 project 相对路径配置文件
- `section`: 包含 MCP 设置的 INI section
- `enable_key`: 可选的布尔开关键；如果存在且为 false，则该 endpoint 不参与发现
- `*_key`: 可选的字段名，用于从该 section 读取值
- `default_port`: 在没读到插件自定义端口前使用的默认端口

Host 默认就是 `127.0.0.1`，路径默认就是 `/mcp`，transport 默认就是 `http`，
auto-start 默认就是 `false`。所以这些默认值不需要再单独加配置项，只有插件真的暴露了
可读取的覆盖字段时，才需要在 strategy 里写对应的 `*_key`。

## 构建

```powershell
cd UnrealMCPHub
cargo build
```

## 更新 vendored MCPHub

`vendor/MCPHub` 是标准 git submodule，应该以远端 `MCPHub` 仓库为真源同步：

```powershell
git submodule update --remote vendor/MCPHub
git add vendor/MCPHub
git commit -m "chore: bump vendored mcphub"
```

## CLI 快速示例

配置当前项目：

```powershell
target\debug\unreal-mcphub.exe setup "D:\Projects\Games\Unreal Projects\LyraStarterGame\LyraStarterGame.uproject"
```

如果在某个 Unreal 工程目录内启动，UnrealMCPHub 也会在执行命令前自动尝试绑定
当前 project。如果这个 project 已经存在于 `~/.unreal-mcphub/config.json` 中，
现在会优先复用已保存的绑定信息，只有在探测到引擎路径或 MCP endpoint 配置发生变化时
才会刷新配置。
在 Windows PowerShell 下，`call-tool --arguments-json` 现在同时兼容严格 JSON
和 PowerShell 传给原生 exe 时常见的“去引号对象”形式，但传非空参数时仍建议优先
使用 `ConvertTo-Json -Compress`，可读性和稳定性都更好。

对于嵌入在 Unreal 插件里的 MCP，`launch` 和 `verify-ue` 只能在 endpoint 实际启动后
才能连通。如果发现到的 endpoint 显示 `auto_start=false`，那就表示 UnrealMCPHub
虽然可以拉起编辑器，但不能替插件把 MCP 服务自动开起来。这种情况下，需要在 Unreal
里开启插件 MCP 的自动启动，或者在 Editor 打开后手动启动 MCP 服务。

给当前 active project 增加一个额外的 MCP：

```powershell
target\debug\unreal-mcphub.exe add-mcp tools-secondary --host 127.0.0.1 --port 19841 --path /mcp --activate
```

切换当前 project 的 active MCP：

```powershell
target\debug\unreal-mcphub.exe use-mcp tools-secondary
```

列出当前 active MCP 暴露的工具：

```powershell
target\debug\unreal-mcphub.exe list-tools
```

列出工具，同时查看描述和输入 schema：

```powershell
target\debug\unreal-mcphub.exe list-tools --json
```

调用当前 active MCP 上的一个工具：

```powershell
target\debug\unreal-mcphub.exe call-tool get_dispatch --arguments-json "{}"
```

在 PowerShell 中调用带参数的工具：

```powershell
$args = @{ skill_name = "cpp_editor_api"; path = "docs/overview.md" } | ConvertTo-Json -Compress
target\debug\unreal-mcphub.exe call-tool read_unreal_skill --arguments-json "$args"
```

`run_unreal_skill` 现在已经会正确遵守 optional 默认值，只需要传你当前实际会
用到的字段。执行 inline Python 时，最简单可以只传 `python`：

```powershell
$args = @{
  python = "RESULT = {'ok': True, 'source': 'manual-cli-smoke'}"
} | ConvertTo-Json -Compress
target\debug\unreal-mcphub.exe call-tool run_unreal_skill --arguments-json "$args"
```

如果插件更新后工具缓存看起来还是旧的，重新执行一次 `discover` 或
`sync-mcphub` 刷新工具目录即可。

日常使用只需要 `unreal-mcphub.exe`。vendored 的 `MCPHub` 仓库只是内部实现
依赖，不是需要你手动调用的 companion 可执行文件。

```powershell
target\debug\unreal-mcphub.exe sync-mcphub
target\debug\unreal-mcphub.exe list-tools --json
```

这样会先刷新同步后的工具目录，再查看 UnrealMCPHub 自己暴露出来的当前工具面。

查看当前 Hub 状态：

```powershell
target\debug\unreal-mcphub.exe status
```

启动 Editor 并等待 MCP ready：

```powershell
target\debug\unreal-mcphub.exe launch --wait-seconds 180
```

如果当前 active project 关联的 Unreal Editor 进程已经存活，`launch` 现在会优先复用
这个已跟踪实例，不会再为同一个 project / MCP target 额外拉起第二个 Editor。

如果返回的 JSON 里出现 `health: null`，或者 note 中提示 `auto_start=false`，
表示 Editor 已经启动，但在等待窗口内内嵌 endpoint 始终没有变成可连接状态。常见原因
通常是插件未启用、MCP 服务未设置为自动启动，或者需要在 Editor 内手动打开。

对当前 active project 执行一次真实 UE 回归验证：

```powershell
target\debug\unreal-mcphub.exe verify-ue --compile --wait-seconds 180
```

这个命令会直接连接真实的 Unreal Editor，等待内嵌 MCP ready，然后一次性检查
暴露工具表面，并实际调用代表性的 C++、Blueprint、Asset、skill、session 与
`sync-mcphub` 流程，输出结构化验证报告。`--wait-seconds` 只控制“等待编辑器内
MCP 变为 healthy”的阶段，不是整条验证命令的总超时；后续真实工具调用仍会继续
执行。Windows 下如果希望稳定落盘报告，优先使用：

```powershell
target\debug\unreal-mcphub.exe verify-ue --compile --wait-seconds 180 --output verify-ue-report.json
```

如果你只想在终端里看简洁结果，不想刷出完整 JSON，可以使用：

```powershell
target\debug\unreal-mcphub.exe verify-ue --compile --wait-seconds 180 --summary
```

`--summary` 现在会直接写 stdout，适合在 PowerShell 里交互查看；如果你需要持久化
报告文件，或者后面还要详细排查，`--output` 仍然是更合适的选择。

发现可达实例：

```powershell
target\debug\unreal-mcphub.exe discover
```

查看实例健康：

```powershell
target\debug\unreal-mcphub.exe health
target\debug\unreal-mcphub.exe health <project>:<mcp-id>:<port>
```

查看 session 快照：

```powershell
target\debug\unreal-mcphub.exe session --scope full --limit 20
target\debug\unreal-mcphub.exe session <project>:<mcp-id>:<port> --scope history --limit 50
```

刷新当前活动 Unreal MCP 对应的内部同步 catalog：

```powershell
target\debug\unreal-mcphub.exe sync-mcphub
```

停止当前活动 Unreal Editor：

```powershell
target\debug\unreal-mcphub.exe stop
```

在 Windows 下，`stop` 会先尝试正常结束进程树；如果子进程阻止了优雅退出，会自动
回退到强制终止，并同步刷新保存的实例状态，避免 Editor 已经退出但旧 PID 还被显示为
在线。

## MCP Server

把 UnrealMCPHub 自己作为 stdio MCP server 启动：

```powershell
target\debug\unreal-mcphub.exe serve
```

把 UnrealMCPHub 作为 HTTP MCP server 启动：

```powershell
target\debug\unreal-mcphub.exe serve --http --host 127.0.0.1 --port 9422
```

当前暴露的 MCP tools：

- `project`
  Actions: `status`、`setup`、`use_project`、`use_mcp`、`save_mcp`、
  `set_plugin_source`、`install_plugin`
- `editor`
  Actions: `compile`、`launch`、`stop`、`restart`、`discover`、`use`、`health`
- `session`
  Actions: `get`、`add_note`、`crash_report`
- `mcp`
  Actions: `list_tools`、`call_tool`、`sync`
