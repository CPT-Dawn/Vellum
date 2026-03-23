# awww-tui

A modern, high-performance Wayland wallpaper terminal UI powered by `awww`.

## Phase 1 Status

Phase 1 includes:

- Rust project initialization with strict modular structure
- async main loop using `tokio`
- terminal setup/teardown with `crossterm`
- foundational `ratatui` layout with dynamic pane highlighting
- basic app state and Vim-style pane navigation (`h`/`l`, arrows, `q` to quit)

## Phase 2 Status

Phase 2 includes:

- Wayland monitor discovery with backend fallback order: `hyprctl -j monitors` then `wlr-randr`
- Parser normalization into a shared `MonitorInfo` model (name, geometry, refresh rate)
- Async command wrapper for `awww` and `awww-daemon` using `tokio::process::Command`
- Typed transition controls (`fade`, `wipe`, `grow`) for future transition lab integration
- Unit tests for both monitor parser backends

## Run

```bash
cargo run
```
