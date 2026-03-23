# awww-tui

A modern, high-performance Wayland wallpaper terminal UI powered by `awww`.

## Phase 1 Status

Phase 1 includes:

- Rust project initialization with strict modular structure
- async main loop using `tokio`
- terminal setup/teardown with `crossterm`
- foundational `ratatui` layout with dynamic pane highlighting
- basic app state and Vim-style pane navigation (`h`/`l`, arrows, `q` to quit)

## Run

```bash
cargo run
```
