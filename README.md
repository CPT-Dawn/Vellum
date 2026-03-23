# Vellum

Vellum is a native Wayland wallpaper runtime in active migration from its
predecessor codebase.

This repository has been simplified to keep only the core runtime crates used
by the migration:

- common
- daemon (published as vellum-core)

Legacy packaging, shell completion, docs generation, script examples, and
standalone client/test folders were removed to keep the workspace focused.

## Requirements

- A compositor that supports wlr-layer-shell
- lz4 development headers
- Rust 1.89.0 or newer

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

Current integrated TUI capabilities in `vellum-tui` include:

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

## AUR Packaging Outline

A starter Arch packaging template for the combined binary is provided at:

- [packaging/PKGBUILD.template](packaging/PKGBUILD.template)
