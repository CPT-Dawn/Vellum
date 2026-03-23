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
- Tagged release workflow that builds and archives Linux binaries.
- Renderer command queue now updates an internal backend assignment state.
- Tagged release workflow now publishes GitHub Release assets with SHA256 checksums.
- TUI extracted display and image utility modules.
- Renderer-facing handler tests validate set/clear command effects.
- Makefile now includes release checksum generation and verification helpers.
- TUI app state moved into dedicated app state module.
- TUI key input handling and frame rendering split into dedicated modules.
- TUI non-UI command dispatch extracted into a dedicated module.
- Renderer now performs image-path preflight diagnostics before applying queued assignments.
- Renderer now routes output refresh/apply/clear through a dedicated layer-shell session scaffold boundary.
- Renderer now tracks per-output surface state with dynamic output add/remove lifecycle handling.
- Renderer now includes shared-memory buffer pool allocation/reuse/reclaim lifecycle management.
- Renderer now includes stress and latency checks for apply/clear flow and buffer boundedness.
- Renderer now drives visible output via per-monitor swaybg presenter management on Wayland sessions.
- IPC set/clear now fail fast if renderer presentation fails, avoiding false success responses.
- Renderer now composes native per-output RGBA frame buffers with fit/fill/crop logic before presentation.
- Renderer now uploads composed native frame bytes into shared-memory pool entries per output.
- Renderer now emits native commit descriptors (output, stride, buffer id, source) from layer-shell session flow.
- Renderer now forwards native commit descriptors into a wl_shm bridge stage that tracks latest per-output commits.
- wl_shm bridge now executes native commit plans through a commit executor boundary with validation and error propagation back to renderer apply.
- Renderer backend assignment state now advances transactionally only after successful wl_shm commit execution.
- Daemon now runs a continuous monitor refresh loop and re-syncs renderer output surfaces when monitor snapshots change.
- Daemon startup now replays persisted assignments into renderer state so restored assignments are applied immediately after boot.
- Daemon now binds its IPC socket before bootstrap replay work, improving startup responsiveness and restart reliability.
- Set-wallpaper daemon state rollback now restores the previous assignment on renderer failure, preserving transactional correctness.
- Daemon monitor refresh loop and IPC handlers now share a synchronized monitor snapshot for monitor queries and target validation.

## In Progress

- Integrating live Wayland protocol event loop bindings into layer-shell session internals (replacing CLI-based monitor probing path).

## Remaining Before Project Completion

- Add compositor-backed integration tests in a Wayland test harness environment.

## Recommended Next Slices

1. Finish wiring wl_output registry and lifecycle callbacks into layer-shell session state.
2. Add compositor-backed integration tests in a Wayland test harness environment.
