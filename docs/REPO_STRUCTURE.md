# Repository Structure

This repository now follows a more modular Rust layout and is prepared for upcoming renderer work.

## Current Structure

- Cargo.toml
- README.md
- Makefile
- systemd/user/vellumd.service
- docs/
  - STATUS.md
  - REPO_STRUCTURE.md
- crates/
  - vellum-ipc/
    - src/lib.rs
    - src/protocol.rs
    - src/envelope.rs
  - vellumd/
    - src/main.rs
    - src/cli.rs
    - src/paths.rs
    - src/monitor.rs
    - src/state.rs
    - src/ipc/
      - mod.rs
      - server.rs
      - handlers.rs
    - src/renderer/
      - mod.rs
      - backend.rs
      - command_queue.rs
      - output_registry.rs
  - vellum-tui/
    - src/main.rs
    - src/cli.rs
    - src/daemon_client.rs
    - src/app/
      - mod.rs
      - state.rs
      - input.rs
      - ui.rs
    - src/display.rs
    - src/images.rs

## Target Professional Structure (Near-Term)

- crates/
  - vellum-ipc/
    - src/lib.rs
    - src/protocol.rs
    - src/envelope.rs
  - vellumd/
    - src/main.rs
    - src/cli.rs
    - src/paths.rs
    - src/monitor.rs
    - src/state.rs
    - src/renderer/
      - mod.rs
      - output_registry.rs
      - command_queue.rs
      - layer_shell.rs
      - shm_pool.rs
      - image_pipeline.rs
    - src/ipc/
      - mod.rs
      - server.rs
      - handlers.rs
  - vellum-tui/
    - src/main.rs
    - src/cli.rs
    - src/daemon_client.rs
    - src/app/
      - mod.rs
      - state.rs
      - actions.rs
    - src/ui/
      - mod.rs
      - layout.rs
      - widgets.rs
      - theme.rs
    - src/daemon/
      - mod.rs
      - client.rs
      - protocol.rs

## Guiding Rules

- Keep main.rs as thin entrypoints.
- Move domain logic into focused modules.
- Keep shared wire types only in vellum-ipc.
- Keep daemon Wayland rendering internals isolated from IPC transport logic.
- Keep TUI rendering concerns separate from daemon communication concerns.
