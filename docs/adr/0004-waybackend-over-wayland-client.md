# waybackend instead of wayland-client for the Daemon

The 2.0 spec picked `wayland-client` with its pure-Rust backend for `vellumd`. It also said the choice would go back to the maintainer if `wayland-client` turned out materially larger than `waybackend`. It did. We built the same minimal Daemon on each crate: bind the globals, create a background layer surface per Monitor, turn a memfd into a `wl_shm` pool, destroy the buffer on release, and run a rustix `poll` loop. Built with rustc 1.99.1 and a release profile of LTO, one codegen unit, strip and `panic = "abort"`:

| Build | waybackend 0.11 | wayland-client 0.31 | Difference |
|---|---:|---:|---:|
| `opt-level = 3` | 327,144 B | 485,672 B | +158,528 B (+48.5%) |
| `opt-level = "s"` | 313,648 B | 450,576 B | +136,928 B (+43.7%) |
| Idle RSS after one frame | 2,456 kB | 2,584 kB | +128 kB |

The full measurement is on issue #2. The maintainer chose `waybackend`, which supersedes the spec's `wayland-client` line. The first real `vellumd` built on it (Monitors, IPC, present) is 354,536 B stripped.

## Consequences

- We vendor the protocol XMLs in `crates/vellumd/protocols/`. `waybackend-scanner` generates the bindings at build time. To add a protocol, drop in its XML, list it in `build.rs`, add an `Obj` variant and handle its events.
- Object tracking is manual. Every Wayland object id gets an `Obj` variant in `ObjectManager`, and every received event has to be decoded even if we ignore it, because decoding is what advances the read cursor.
- Fds passed in requests, such as a frame's memfd, are sent on `flush()`, not when the request is queued. Keep the fd open until the flush.
- `Anchor::all()` in the generated bitflags sets every bit, and compositors reject that. Spell out `TOP | BOTTOM | LEFT | RIGHT`.
- `waybackend` is a small crate with a single maintainer. Depending on it does not copy any awww code, so ADR 0001 holds. If it is ever abandoned, the wire-level surface we use is small enough to fork or replace.
