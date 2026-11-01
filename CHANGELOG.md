# Changelog

All notable changes to Vellum are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Vellum uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [2.0.0] - Unreleased

Vellum 2.0 is a clean-room rewrite. No code is shared with 1.x or with
awww, and the license changes to MIT OR Apache-2.0
([ADR 0001](docs/adr/0001-clean-room-rewrite-and-relicense.md)).

### Added

- `vellumd`, a single-threaded Daemon that links no image decoders and keeps
  no pixels ([ADR 0003](docs/adr/0003-decoder-free-pixel-free-daemon.md)).
  It idles at about 2.5 MiB RSS with zero wakeups.
- Fade, Wipe and Slide Transitions with a direction, duration and easing,
  paced by each Monitor's frame callbacks. Fade runs on the compositor's
  GPU through alpha-modifier where available, with a short CPU fallback.
- Four Fit modes (Fill, Fit, Center, Stretch) and a configurable letterbox
  color, rendered at the Monitor's exact physical resolution, fractional
  scaling included.
- Wallpapers restored at startup, on hotplug and after a scale change,
  without a flash of black. Until its first frame, a Monitor shows the
  letterbox color as a solid fill (single-pixel-buffer-v1, where the
  compositor has it).
- Rotation from the Library or Favorites, shuffled or sequential, with
  per-monitor Overrides and an optional `sync`. It survives reboots.
- Favorites (`vellum fav`) and stepping by hand with `vellum next`, `prev`
  and `random`.
- An optional, live-reloading Config at `~/.config/vellum/config.toml`.
  Mistakes are reported with the line and key, and the last good Config
  stays in effect.
- The `on_change` hook, run without a shell, for matugen, pywal and
  wallust.
- A new TUI: list and true-to-monitor preview, a thumbnail grid on kitty
  graphics, fuzzy search, Favorites, a command palette, a help overlay,
  a Rotation panel that saves to the Config, a first-run welcome screen,
  mouse support, remappable keys, and the Midnight Bloom theme with ANSI
  and `NO_COLOR` fallbacks.
- `VELLUM_GRAPHICS` forces the TUI's preview protocol (`halfblocks`,
  `kitty`, `sixel` or `iterm2`) for terminals that misreport what they draw.
- Any `vellum` command starts the Daemon when it isn't running.
- A `Type=notify` systemd user unit, `vellumd.service`.
- Shell completions (`vellum completions`) and man pages.
- Release tarballs for x86_64 and aarch64 Linux.
- Optional AVIF and JXL decoding behind the `avif` and `jxl` build
  features.
- A benchmark script and CI gates for memory, wakeups and binary size.

### Changed

- The Daemon binary is now `vellumd` (was `vellum-daemon`), and its user
  unit is `vellumd.service` (was `vellum-daemon.service`). Disable the old
  unit before upgrading:
  `systemctl --user disable --now vellum-daemon.service`.
- Startup is `exec-once = vellumd` (or the systemd unit). `vellum` no longer
  has a bootstrap mode, and there is no XDG autostart file.
- State, Favorites and the cache moved to new files under the XDG folders.
  Nothing is migrated from 1.x.

### Removed

- Animated wallpapers (GIF, animated WebP, APNG) and video
  ([ADR 0002](docs/adr/0002-static-wallpapers-only.md)).
- The 1.x playlists, aspect simulator profiles and tokio worker.

## 1.x

Vellum 1.0.0 to 1.1.0 (April 2026) were a fork of awww under GPL-3.0. Their
notes are on [GitHub Releases](https://github.com/CPT-Dawn/Vellum/releases).

[2.0.0]: https://github.com/CPT-Dawn/Vellum/compare/v1.1.0...v2.0.0
