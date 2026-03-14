# Feature Parity

This document tracks migration from the older Python Unreal hub into the new
standalone Rust `UnrealMCPHub`.

## Source of Truth

The parity target is the actual Python implementation, not only the old README.

Concrete old surfaces include:

- project onboarding and persisted config
- build and launch lifecycle
- plugin install/configuration
- instance discovery and active-instance switching
- logs and crash retrieval
- session notes and call history
- tool routing over embedded Unreal MCP servers

## Implemented Here

- project setup with engine autodetection
- persisted active-project config
- compile via `Build.bat`
- editor launch with MCP readiness wait
- best-effort current-directory project binding
- configuration-driven MCP discovery strategies with a default UnrealCopilot strategy
- multiple configured MCP targets per project
- active MCP switching inside one project
- instance discovery driven by configured project MCP targets
- active-instance switching
- local plugin source config and copy-based install
- latest crash directory summary lookup
- session notes
- persisted call history and `get_session` snapshots
- per-instance health inspection (`get_instance_health`)
- background watcher during `serve`, including crash counting and stale-instance cleanup
- HTTP serving mode for the outer hub
- editor stop / restart actions for recovery flow
- standard MCP forwarding through generic list/call surfaces
- stdio MCP facade
- bridge into bundled generic `MCPHub` submodule via `sync-mcphub`

## Partially Implemented

- plugin installation currently copies from a local source path instead of
  supporting zip download flows

## Not Yet Implemented

- cook/package build actions
- log tail and build log analysis parity
- richer plugin-specific discovery strategies
