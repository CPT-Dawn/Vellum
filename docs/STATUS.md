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
- Daemon integration tests that spawn vellumd and verify IPC/persistence flows.
- CI workflow for fmt, clippy, and workspace tests.
- Daemon IPC split into server and handler modules.

## In Progress

- Native renderer backend implementation in daemon (SCTK + wlr-layer-shell + wl_shm).

## Remaining Before Project Completion

- Implement real Wayland render surfaces per output in daemon.
- Implement wl_output hotplug lifecycle (add/remove/reconfigure outputs).
- Implement shared-memory buffer lifecycle and reclamation strategy.
- Add renderer-focused performance checks and memory profiling benchmarks.
- Add packaging/release pipeline for publishable artifacts.

## Recommended Next Slices

1. Move TUI app state/actions/layout into dedicated modules.
2. Implement first renderer backend milestone for one output.
3. Add renderer-facing integration tests (command queue to backend behavior).
4. Add release packaging workflow for tagged builds.
