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
