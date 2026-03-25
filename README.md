# Vellum

Vellum ships two binaries:

- `vellum-daemon`, which applies wallpapers on the Wayland session.
- `vellum`, which is the TUI client used to browse images and send wallpaper requests.

## Start On Login

Preferred setup on Arch or AUR installs:

1. Install the systemd user unit from `packaging/systemd/user/vellum-daemon.service` into your user unit directory or package it into `/usr/lib/systemd/user/`.
2. Enable it with `systemctl --user enable --now vellum-daemon.service`.

This keeps the daemon running for the whole Wayland session and lets systemd restart it if it fails during compositor startup.

If your desktop environment uses XDG autostart instead of a user service, install `packaging/autostart/vellum-daemon.desktop` into `~/.config/autostart/` or `/etc/xdg/autostart/`.

## TUI Daemon Keybind

In the TUI, `s` now starts the daemon if it is stopped and refreshes its status if it is already running. It no longer stops the daemon.
