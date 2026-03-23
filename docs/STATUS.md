# Vellum Status

## Completed

- Rust workspace with three crates: vellum-ipc, vellumd, vellum-tui.
- Versioned IPC protocol envelopes and request/response schema.
- TUI image browser with preview, monitor target selection, and scale mode selection.
- Daemon assignment tracking with persisted state file.
- Assignment introspection and clear controls from CLI and TUI.
- Baseline protocol and persistence unit tests.
- Daemon renderer scaffold with command queue and output registry.
- TUI extracted CLI and daemon transport modules.

## In Progress

- Native renderer backend implementation in daemon (SCTK + wlr-layer-shell + wl_shm).

## Remaining Before Project Completion

- Implement real Wayland render surfaces per output in daemon.
- Implement wl_output hotplug lifecycle (add/remove/reconfigure outputs).
- Implement shared-memory buffer lifecycle and reclamation strategy.
- Add integration tests for daemon/client socket interactions.
- Add renderer-focused performance checks and memory profiling benchmarks.
- Add packaging/CI pipeline for release artifacts and lint/test gates.

## Recommended Next Slices

1. Add daemon IPC handler module split (`ipc/server.rs`, `ipc/handlers.rs`).
2. Move TUI app state/actions/layout into dedicated modules.
3. Add integration test harness that boots daemon and exercises IPC flows.
4. Implement first renderer backend milestone for one output.
