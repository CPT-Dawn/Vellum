# The Daemon holds no image decoders and keeps no pixels

The `vellumd` daemon links no image-decoding code and drops each frame as soon as the compositor has taken it. All decoding and scaling happens in the `vellum` binary: either the interactive process (`vellum set`, the TUI), or a short-lived `vellum render` process that the daemon starts whenever it needs a new frame (restore at login, a monitor plugged in, a scale change, a Rotation tick). The finished frame reaches the daemon as a memfd over the IPC socket, the daemon commits it, and frees it on `wl_buffer.release`. That keeps the always-running process at a few MB with zero wakeups when idle, in exchange for a process spawn whenever a new frame is needed.

## Consequences

- `vellumd` depends on `vellum` being installed next to it. They always ship together.
- On Smithay-based compositors (niri, COSMIC) the compositor keeps the memfd pages until the next commit. The daemon's own memory use drops, but total system memory does not.
- Do not "optimize" by caching decoded frames in the daemon. Doing so undoes the core design goal.
