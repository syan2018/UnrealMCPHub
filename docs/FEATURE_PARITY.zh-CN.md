# Feature 对齐状态

这份文档用于追踪旧 Python Unreal Hub 到新 Rust `UnrealMCPHub`
的迁移进度。

## 对齐基准

这里对齐的是旧仓库的真实实现，而不只是 README 文案。

旧实现的核心面包括：

- 工程初始化与持久化配置
- 编译与启动生命周期
- 插件安装与配置
- 实例发现与活动实例切换
- 日志与崩溃信息读取
- 会话备注与调用历史
- 面向 UE 内嵌 MCP server 的工具路由

## 当前已实现

- 工程 setup 与引擎自动探测
- active project 持久化配置
- 通过 `Build.bat` 编译
- 拉起 Editor 并等待 MCP ready
- 从当前工作目录 best-effort 自动绑定 project
- 配置驱动的 MCP discovery strategy，默认内置 UnrealCopilot 策略
- 单个 project 下可配置多个 MCP
- 支持 project 内 active MCP 切换
- 仅基于已配置 project 的 MCP 做实例发现
- active instance 切换
- 本地插件源配置与复制式安装
- 最新崩溃目录摘要读取
- session note
- 持久化调用历史与 `get_session` 快照
- `get_instance_health` 实例健康检查
- `serve` 生命周期内的后台 watcher，包括 crash 计数与陈旧实例清理
- 外层 hub 的 HTTP serving 模式
- stop / restart 编辑器恢复动作
- 通过通用 list/call 表面做标准 MCP 转发
- stdio MCP facade
- 通过 `sync-mcphub` 把当前 Unreal MCP 同步进 bundled 的通用 `MCPHub`

## 部分完成

- 插件安装目前只支持本地复制，还没有 zip/GitHub 下载链路

## 尚未完成

- cook/package 构建动作
- 日志 tail 与构建日志分析对齐
- 更丰富的插件专属 discovery strategy
