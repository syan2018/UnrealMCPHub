# Feature Parity

This document tracks migration from the older Python `UnrealMCPHub` into the
new standalone Rust `UnrealMCPOrchestrator`.

## Source of Truth

The parity target is the actual Python implementation, not only the old README.

Concrete old surfaces include:

- project onboarding and persisted config
- build and launch lifecycle
- plugin install/configuration
- instance discovery and active-instance switching
- logs and crash retrieval
- session notes and call history
- UE proxy calls across dispatch-style and inventory-style MCP servers

## Implemented Here

- project setup with engine autodetection
- persisted active-project config
- compile via `Build.bat`
- editor launch with MCP readiness wait
- instance discovery against UnrealCopilot HTTP endpoints
- dynamic discovery seeded by configured projects, known instances, and scan ports
- active-instance switching
- local plugin source config and copy-based install
- latest crash directory summary lookup
- session notes
- persisted call history and `get_session` snapshots
- per-instance health inspection (`get_instance_health`)
- background watcher during `serve`, including crash counting and stale-instance cleanup
- HTTP serving mode for the outer orchestrator
- editor stop / restart actions for recovery flow
- UE proxy calls for current UnrealCopilot tools
- stdio MCP facade
- bridge into bundled generic `MCPHub` submodule via `sync-mcphub`

## Partially Implemented

- multi-instance support now reuses configured projects, known instances, and
  configured scan ports, but still works best once a project has been set up
- plugin installation currently copies from a local source path instead of
  supporting zip download flows

## Not Yet Implemented

- cook/package build actions
- log tail and build log analysis parity
- richer proxy compatibility adapters beyond the current UnrealCopilot-oriented
  flows
- discovery of completely unrelated UE projects that are running but were never
  configured in the orchestrator

## Notes About Naming Drift

The old README drifted from the actual Python tool names. This new repository
currently follows the README-oriented names where they are clearer:

- `compile_project` instead of Python's `build_project`
- `use_editor` as a direct tool instead of the older `manage_instance(action="use")`
- `get_crash_report` as a direct tool rather than tunneling through `get_log(source="crash")`

The intention is to preserve user experience while keeping the implementation
and docs aligned.
