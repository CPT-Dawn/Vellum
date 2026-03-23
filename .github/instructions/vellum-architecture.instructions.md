---
description: "Use when implementing Rust code, Wayland daemon/client architecture, TUI features, IPC, or documentation for the Vellum wallpaper manager project."
applyTo:
  - "**/*.rs"
  - "Cargo.toml"
  - "README.md"
---

# Project Context: Vellum
Vellum is a high-performance, Wayland-native wallpaper manager built in Rust, specifically targeting wlroots-based compositors (like Hyprland) and Niri.

It strictly follows a Daemon/Client architecture:
1. `vellumd` (The Daemon): A long-running background process that interacts with Wayland (`wlr-layer-shell`, `wl_shm`) to render images efficiently at the bottom layer of the compositor. It listens for commands via Unix domain sockets.
2. `vellum-tui` (The Client): A modern, terminal-based user interface that communicates with the daemon. It provides a file browser, live image previews inside the terminal, and wallpaper management controls.

## Core Directives for AI Assistant

### 1. Rust Coding Standards (Memory Safety & Speed)
- Idiomatic and Safe Rust: Write strict, idiomatic Rust. Maximize the use of the borrow checker to ensure memory safety. Do not use `unsafe` blocks unless interfacing directly with a C library where it is unavoidable, and then wrap it safely.
- Zero-Cost Abstractions: Leverage Rust's zero-cost abstractions to keep the binary lightweight and fast.
- Error Handling: Use `anyhow` for application-level binaries and `thiserror` for library-level code. Never use `.unwrap()` or `.expect()` in production code; handle errors gracefully and bubble them up to the TUI or daemon logs.
- Educational and Documented: Add clear inline comments for complex Wayland protocol interactions or Rust lifecycle management.
- Resource Efficiency: The daemon must have near-zero idle CPU usage. Optimize image decoding and buffer sharing to perform well on high-end hardware without unnecessary battery drain.

### 2. Wayland and Daemon Architecture
- Stack: Use `smithay-client-toolkit` (SCTK) for Wayland interactions. Avoid raw FFI bindings if SCTK provides a safe abstraction.
- Layer Shell: Wallpapers must be rendered using `wlr-layer-shell` on the `background` layer.
- Monitor Hotplugging: `vellumd` must dynamically handle `wl_output` events. If a monitor connects or disconnects, the daemon should re-allocate or drop surfaces without crashing.
- Memory Management (`wl_shm`): Be precise with shared memory pools. Ensure old image buffers are dropped and memory is freed when a new wallpaper is applied to prevent leaks.

### 3. TUI and Aesthetics (Modern and Beautiful)
- Stack: Use `ratatui` for the interface, `crossterm` for backend/events, and `ratatui-image` for terminal image rendering.
- Image Previews: Support high-fidelity image previews that bypass standard text cells. Implement Kitty Graphics Protocol first, with Sixel fallback.
- Color Palette: Default to modern, high-contrast dark styling (for example, Tokyo Night or One Dark Pro inspired palettes) with deep blues, soft purples, and vibrant accents.
- Responsiveness: The TUI layout must dynamically resize without panicking. Use `ratatui` `Constraint` effectively to manage a 3-panel layout (Browser, Metadata, Preview).

### 4. Inter-Process Communication (IPC)
- Use Unix domain sockets for communication between `vellum-tui` and `vellumd`.
- Structure IPC payloads with `serde_json`. Keep the API minimal (for example: `SetWallpaper`, `GetMonitors`, `KillDaemon`).
- Ensure the TUI handles daemon connection timeouts gracefully and prompts when the daemon is not running.

### 5. Review and Generation Rules
- When generating code, output only the requested changes or complete files. Avoid truncating essential logic.
- If a requested feature violates Wayland design principles or Rust memory safety, flag it and propose an architectural alternative.
