# Vellum

Vellum is a native Wayland wallpaper runtime with a foreground TUI and a
background daemon that owns the wallpaper surfaces.

This repository has been simplified to keep only the core runtime crates used
by the migration:

- common
- daemon (published as vellum-core)

The TUI starts a daemon subprocess on demand for manual use, and the package
also ships session-start integration so the daemon can start automatically in a
Wayland desktop session.

## Requirements

- A Wayland compositor that supports wlr-layer-shell
- lz4 development headers
- Rust 1.94.0 or newer

## Build

Run from repository root:

cargo check --workspace

or

cargo build --workspace

## Workspace Layout

- Cargo workspace root: [Cargo.toml](Cargo.toml)
- Shared runtime support code: [common/src/lib.rs](common/src/lib.rs)
- Daemon/runtime core API: [daemon/src/lib.rs](daemon/src/lib.rs)

## Notes

- The daemon crate package name is vellum-core.
- The library crate name is vellum_core.
- Protocol XML is kept in [protocols/wlr-layer-shell-unstable-v1.xml](protocols/wlr-layer-shell-unstable-v1.xml).

## TUI Features

Current integrated TUI capabilities in `vellum` include:

- Native backend control (integrated daemon runtime)
- Filesystem image browser with fuzzy filtering
- Transition controls (duration, fps, easing, effect)
- Aspect ratio simulator (fit/fill/stretch) against selected monitor
- Auto-cycling playlist controls
- Profile save/load in JSON format

Key controls:

- `Tab`/`Shift+Tab`: move between panes
- `Arrow keys`: navigate/edit
- `Enter`: open folder or apply selected image
- `F5`: toggle playlist auto-cycle
- `F6`: add selected image to playlist
- `F7`: clear playlist
- `F8`: save default profile
- `F9`: load default profile

## Session Startup

The packaged daemon is intended to run only inside a Wayland session.
It should not be launched from a plain TTY.

For automatic startup, install the session autostart entry or enable the
user service shipped in packaging.

The foreground app binary is `vellum`.
The background daemon process is `vellum-daemon`.
The internal IPC namespace defaults to `vellum`.
