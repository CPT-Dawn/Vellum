# Vellum

Vellum is a Wayland wallpaper stack with:

- `vellum-daemon`: the wallpaper daemon that talks to Wayland and applies images.
- `vellum`: the TUI client for browsing images, configuring playlist behavior, and controlling runtime.

## Install

### Arch Linux / AUR

```bash
paru -S vellum
```

## Reboot-Friendly Startup

Vellum is designed to be session-safe and reboot-friendly:

- The daemon restores last applied wallpapers from cache on startup.
- Favorites and playlist settings are persisted in XDG state.
- Launching `vellum` without a TTY (for example from compositor startup) runs a lightweight bootstrap path that ensures daemon + playlist worker startup.

### Hyprland (simple setup)

Add this to your Hyprland config:

```ini
exec-once = vellum
```

This is enough for startup bootstrap on login. Open `vellum` from a terminal when you want the interactive TUI.

### systemd user service (recommended for packaged installs)

Install the unit file from:

- `packaging/systemd/user/vellum-daemon.service`

Then enable it:

```bash
systemctl --user enable --now vellum-daemon.service
```

This keeps `vellum-daemon` alive across your user session and auto-restarts on failure.

### XDG autostart (non-systemd sessions)

Install:

- `packaging/autostart/vellum.desktop`

to:

- `~/.config/autostart/` (per-user), or
- `/etc/xdg/autostart/` (system-wide).

## Persistence Model

### Wallpaper restore

`vellum-daemon` restores previous per-output wallpapers from cache by default.

### User state

Vellum stores state in XDG locations:

- `XDG_STATE_HOME/vellum/playlist-state-v1.txt`
- `XDG_STATE_HOME/vellum/favorites-v1.txt`

Fallback path when `XDG_STATE_HOME` is not set:

- `~/.local/state/vellum/`

Cache path:

- `XDG_CACHE_HOME/vellum/` (or `~/.cache/vellum/`)

## Daily Usage

1. Run `vellum` in a terminal to open the TUI.
2. Use `r` to restart/reload daemon handling from the TUI when needed.
3. Apply wallpapers and playlist settings; state and cache are reused after reboot.
