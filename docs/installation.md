# Installation

How to get `vellum` and `vellumd` onto your machine and start the Daemon
with your session. Vellum needs a Wayland compositor with wlr-layer-shell,
such as Hyprland, niri or Sway.

## Channels

| Channel | Command | What you get |
| --- | --- | --- |
| AUR, prebuilt | `paru -S vellum-bin` | The release binaries, installed in seconds |
| AUR, from source | `paru -S vellum` | A local build of the tagged release |
| GitHub Releases | [Download a tarball](https://github.com/CPT-Dawn/Vellum/releases/latest) | x86_64 or aarch64 Linux binaries |
| Source | See below | Any commit, with optional features |

Each release tarball holds `vellum`, `vellumd`, the systemd user unit
`vellumd.service`, shell completions, man pages and both license files. Put
the two binaries on your `PATH`.

## From source

Needs Rust 1.99 or newer.

```sh
git clone https://github.com/CPT-Dawn/Vellum && cd Vellum
cargo install --locked --path crates/vellumd
cargo install --locked --path crates/vellum
```

### AVIF and JXL

JPEG, PNG and WebP are built in. AVIF and JPEG XL are opt-in features of the
`vellum` crate (the Daemon decodes nothing, so it never needs them):

```sh
cargo install --locked --path crates/vellum --features avif,jxl
```

AVIF needs the system `dav1d` library.

## Start with your session

Any `vellum` command starts the Daemon when it isn't running, but your
wallpapers only come back at login when the Daemon starts with your session.
Add the line for your setup:

| Setup | File | Line |
| --- | --- | --- |
| Hyprland | `hyprland.conf` | `exec-once = vellumd` |
| niri | `config.kdl` | `spawn-at-startup "vellumd"` |
| Sway | `config` | `exec vellumd` |
| systemd, any compositor | | `systemctl --user enable --now vellumd` |

On Hyprland's Lua config (`hyprland.lua`), start it from the startup event:

```lua
hl.on("hyprland.start", function() hl.exec_cmd("vellumd") end)
```

### The systemd unit

The AUR packages and the release tarballs ship `vellumd.service`. From a
tarball, copy it to `~/.config/systemd/user/`. It needs a compositor that
imports `WAYLAND_DISPLAY` into the systemd user environment and starts
`graphical-session.target`: Hyprland through uwsm, niri, and Sway with its
systemd integration do.

## Shell completions

The AUR packages install them for bash, zsh and fish. Otherwise, write the
one for your shell where it loads it, for example:

```sh
vellum completions fish > ~/.config/fish/completions/vellum.fish
vellum completions bash > ~/.local/share/bash-completion/completions/vellum
```

For zsh, write `_vellum` into a directory on your `fpath`.

[Back to the README](../README.md)
