# Vellum Status

## Completed

- Rust workspace with three crates: vellum-ipc, vellumd, vellum-tui.
- Versioned IPC protocol envelopes and request/response schema.
- TUI image browser with preview, monitor target selection, and scale mode selection.
- Daemon assignment tracking with persisted state file.
- Assignment introspection and clear controls from CLI and TUI.
- Baseline protocol and persistence unit tests.

## In Progress

- Native renderer integration in daemon (SCTK + wlr-layer-shell + wl_shm).

## Remaining Before Project Completion

- Implement real Wayland render surfaces per output in daemon.
- Implement wl_output hotplug lifecycle (add/remove/reconfigure outputs).
- Implement shared-memory buffer lifecycle and reclamation strategy.
- Add integration tests for daemon/client socket interactions.
- Add renderer-focused performance checks and memory profiling benchmarks.
- Add packaging/CI pipeline for release artifacts and lint/test gates.

## Recommended Next Slices

1. Introduce renderer module scaffold with explicit state machine.
2. Add output registry and hotplug event handling API.
3. Add render command queue and per-output assignment application.
4. Add integration test harness that boots daemon and exercises IPC flows.
