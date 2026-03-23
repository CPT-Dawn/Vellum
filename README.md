# Vellum

Wayland-native wallpaper manager with a daemon/client architecture.

## Architecture

- `vellumd`: daemon process that receives IPC commands over a Unix socket.
- `vellum-tui`: terminal UI client with image browser, monitor-aware preview, and daemon actions.
- `vellum-ipc`: shared request/response protocol types.

## Current Feature Set

- Interactive TUI with vim-style navigation.
- Terminal image preview with monitor-ratio frame.
- IPC commands for ping, monitor query, wallpaper set, and daemon shutdown.
- Multi-monitor targeting support (`SetWallpaper` can target one output or all outputs).
- Per-assignment scaling modes (`fit`, `fill`, `crop`) across CLI and TUI.
- Assignment introspection (`assignments` command and in-UI summary state).
- Assignment persistence across daemon restarts (stored in XDG state path by default).

The daemon is designed to be native and self-sustained within this project.

## Project Docs

- docs/STATUS.md: delivery status and remaining completion items.
- docs/REPO_STRUCTURE.md: current and target professional module layout.

## Important Status Note

The project is functional, but the fully self-sustained native Wayland renderer is not yet completed.
The daemon now accepts native wallpaper assignments internally. The full SCTK + layer-shell
rendering path is being completed incrementally.

Current visible output path in daemon:

- On Wayland sessions, `vellumd` now manages `swaybg` per output to present wallpapers.
- Ensure `swaybg` is installed on your system for wallpaper changes to be visible.

## Quick Start

### 1) Build and verify

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
```

## CI

GitHub Actions runs these checks on pushes and pull requests:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

Tagged pushes like `v0.1.0` trigger a release workflow that:

- `vellumd`
- `vellum-tui`
- publishes `vellum-linux-x86_64.tar.gz` to GitHub Releases
- publishes `SHA256SUMS` for artifact verification

Local release/checksum helpers:

```bash
make release-checksum
make release-verify
```

### 2) Run daemon

```bash
cargo run -p vellumd
# optional explicit path:
# cargo run -p vellumd -- --state-file /tmp/vellum-state.json
```

### 3) Run TUI

```bash
cargo run -p vellum-tui
```

### 4) CLI actions

```bash
cargo run -p vellum-tui -- ping
cargo run -p vellum-tui -- monitors
cargo run -p vellum-tui -- assignments
cargo run -p vellum-tui -- clear
cargo run -p vellum-tui -- set /absolute/path/image.png
cargo run -p vellum-tui -- set /absolute/path/image.png --monitor DP-1
cargo run -p vellum-tui -- set /absolute/path/image.png --mode fill
cargo run -p vellum-tui -- kill
```

## TUI Keymap

- `j/k`, `Up/Down`, `h/l`: move selection
- `gg` / `G`: jump first/last
- `Ctrl-u` / `Ctrl-d`: page up/down
- `Enter` / `Space`: apply selected wallpaper
- `t`: cycle target output (all outputs or one monitor)
- `s`: cycle scaling mode (`fit` -> `fill` -> `crop`)
- `m`: refresh monitor list from daemon
- `a`: refresh and show assignment summary from daemon
- `x`: clear all daemon assignments (and persisted state)
- `r`: reload image directory
- `?`: toggle help
- `q` / `Esc`: quit

## Packaging

The repository includes:

- `Makefile` for `build`, `install`, `test`, `clippy`, and `fmt`
- `systemd/user/vellumd.service`

Install staging example:

```bash
make install DESTDIR=/tmp/vellum-pkg PREFIX=/usr
```

## Roadmap to Full Self-Sustained Competitor

1. Native SCTK + wlr-layer-shell renderer in daemon.
2. wl_shm buffer lifecycle per output with strict memory management.
3. Dynamic output hotplug handling with surface reallocation.
4. Per-monitor scaling modes (`fit`, `fill`, `crop`) in daemon and TUI.
5. Complete native output rendering to remove any external integration requirements.