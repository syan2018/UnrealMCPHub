# UnrealMCPHub

更多文档：

- [English README](README.md)
- [Feature Parity](docs/FEATURE_PARITY.md)
- [功能对齐状态](docs/FEATURE_PARITY.zh-CN.md)

`UnrealMCPHub` 是一个面向 Unreal 的生命周期 Hub。它 vendored 了通用
[`MCPHub`](https://github.com/syan2018/MCPHub) 作为 git submodule，并在此之上
实现 Unreal 项目、Editor 进程和内嵌 MCP 接口的统一管理。

这个项目的核心目标不是只做一个启动器，也不是只做一个代理，而是提供一个
稳定的控制中枢：

- 通过 CLI 或 MCP tools 管理 Unreal 项目的生命周期
- 根据配置识别和发现嵌入在 Unreal 工程或插件中的 MCP 接口
- 在 Unreal 工程目录中启动时，自动尝试绑定当前 project
- 在 Unreal Editor 重启、崩溃恢复、活动实例切换时维持稳定连接语义
- 把当前活动的 Unreal MCP 同步到 bundled 的通用 `MCPHub`

## 架构分层

这个仓库现在明确分成两层：

- `UnrealMCPHub`
  Unreal-aware 的上层 Hub，负责项目配置、Editor 生命周期、实例发现、会话状态、
  崩溃恢复、MCP 发现与路由，以及对外 MCP/CLI 表面。
- `vendor/MCPHub`
  通用的 MCP registry、discovery、invoke 与 facade 能力，作为可复用底座存在。

这意味着边界是清晰的：

- 通用 MCP 能力放在 `MCPHub`
- Unreal 专属能力放在 `UnrealMCPHub`

## 当前已实现

- 独立 Rust 仓库，内含 `MCPHub` submodule
- 持久化项目配置：`~/.unreal-mcphub/config.json`
- 持久化实例与会话状态：`~/.unreal-mcphub/state.json`
- 从 `.uproject` 与 Windows 注册表自动探测引擎
- 从当前工作目录 best-effort 自动绑定匹配的 Unreal project
- 配置驱动的 MCP discovery strategy，默认内置 UnrealCopilot 策略
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
- stdio MCP facade
- HTTP MCP facade
- `stop_editor` / `restart_editor`
- 通过 `list-tools`、`call-tool`、`sync-mcphub` 提供标准 MCP 转发
- `sync-mcphub`，将当前活动 Unreal MCP 同步到 bundled `MCPHub`

## 仍未完成

- 更丰富的插件专属 discovery strategy，而不只是一条默认 UnrealCopilot 策略
- zip / GitHub 插件下载链路
- cook / package 流程
- 日志 tail 与构建日志分析

## 构建

```powershell
cd UnrealMCPHub
cargo build
```

## 同步 bundled MCPHub

`vendor/MCPHub` 是标准 git submodule，应该以远端 `MCPHub` 仓库为真源同步：

```powershell
git submodule update --remote vendor/MCPHub
git add vendor/MCPHub
git commit -m "chore: bump bundled mcphub"
```

## CLI 快速示例

配置当前项目：

```powershell
target\debug\unreal-mcphub.exe setup "D:\Projects\Games\Unreal Projects\LyraStarterGame\LyraStarterGame.uproject"
```

如果在某个 Unreal 工程目录内启动，UnrealMCPHub 也会在执行命令前自动尝试绑定
当前 project。
在 Windows PowerShell 下，`call-tool --arguments-json` 现在同时兼容严格 JSON
和 PowerShell 传给原生 exe 时常见的“去引号对象”形式，但传非空参数时仍建议优先
使用 `ConvertTo-Json -Compress`，可读性和稳定性都更好。

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

调用当前 active MCP 上的一个工具：

```powershell
target\debug\unreal-mcphub.exe call-tool get_dispatch --arguments-json "{}"
```

在 PowerShell 中调用带参数的工具：

```powershell
$args = @{ skill_name = "cpp_editor_api"; path = "docs/overview.md" } | ConvertTo-Json -Compress
target\debug\unreal-mcphub.exe call-tool read_unreal_skill --arguments-json "$args"
```

`run_unreal_skill` 目前即使只执行 inline Python，也仍要求显式提供
`skill_name`、`script`、`args` 这几个字段，所以建议这样传：

```powershell
$args = @{
  skill_name = $null
  script = $null
  args = @{}
  python = "RESULT = {'ok': True, 'source': 'manual-cli-smoke'}"
} | ConvertTo-Json -Compress
target\debug\unreal-mcphub.exe call-tool run_unreal_skill --arguments-json "$args"
```

查看当前 Hub 状态：

```powershell
target\debug\unreal-mcphub.exe status
```

启动 Editor 并等待 MCP ready：

```powershell
target\debug\unreal-mcphub.exe launch --wait-seconds 180
```

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

把当前活动 Unreal MCP 同步到 bundled `MCPHub`：

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
