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

## Phase 3 Status

Phase 3 includes:

- Full three-pane TUI layout foundation with clear pane separation and dynamic theming
- Dummy wallpaper browser pane with row selection and visual highlighting
- Dummy monitor preview pane with textual layout map and ratio bars
- Transition settings pane with editable dummy fields (type, step, fps)
- Vim navigation enhanced for pane/row movement and transition value editing

## Phase 4 Status

Phase 4 includes:

- Real filesystem wallpaper browser with recursive image discovery
- Fuzzy finding for wallpapers (`/` to enter search mode, `Backspace` to edit)
- Live Try-On while browsing (`j`/`k` updates preview via `awww`)
- Transition controls wired to backend apply (`type`, `step`, `fps`)
- Confirm and revert flow:
	- `Enter` confirms selected wallpaper
	- `c` cancels live preview and reverts
- Monitor targeting integrated with selected output from compositor monitor list

## Phase 5 Status

Phase 5 includes:

- Aspect Ratio Simulator in monitor pane with mode cycling (`m`: fit/fill/crop)
- Image metadata probing for source dimensions used by simulator
- Serde JSON persistence under config directory for profiles and playlists
- Quick profile workflow:
	- `p` saves current selection/transition/monitor/aspect mode
	- `o` reloads and reapplies the saved profile
- Playlist auto-cycling via background Tokio worker:
	- `g` toggles automated cycling
	- `]` applies next playlist entry immediately

## Phase 5.1 Status

Phase 5.1 includes:

- Non-blocking startup refresh for wallpaper indexing and monitor probing
- Bounded wallpaper discovery for large trees (safe cap to reduce memory spikes)
- Permission-tolerant filesystem traversal (skips denied/missing branches)
- Playlist hardening:
	- missing files are pruned from active playlist automatically
	- auto-cycle skips invalid paths safely and persists cleaned playlist state

## Run

```bash
cargo run
```
