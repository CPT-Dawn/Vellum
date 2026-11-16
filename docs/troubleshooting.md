# Troubleshooting

The questions people ask most, each with its answer. For anything else,
[open an issue](https://github.com/CPT-Dawn/Vellum/issues/new/choose) with
the Daemon's log attached.

## Does it work on GNOME?

No. GNOME's compositor, Mutter, doesn't implement wlr-layer-shell, the
protocol Vellum uses to put a surface on the desktop background, and there
is no other way for an app to draw the wallpaper there. Use GNOME's own
background settings. X11 isn't supported either.

## Can I use animated wallpapers or videos?

No, and that is on purpose. Animation needs per-frame decoding and constant
wakeups, which would end the Daemon's zero-CPU idle
([ADR 0002](adr/0002-static-wallpapers-only.md)). For motion, use a
dedicated tool such as [mpvpaper](https://github.com/GhostNaN/mpvpaper).

## Can one image span several Monitors?

Not yet. Each Monitor shows its own wallpaper. Spanning can arrive later as
another Fit mode without changing the design.

## My wallpaper is gone after I log in

The Daemon isn't starting with your session. Add it to your compositor's
autostart: see [Start with your session](installation.md#start-with-your-session).

## My Config change did nothing

Run `vellum reload`. It re-reads the Config and prints any mistake with its
line and key; the last good Config stays in effect until you fix it. See
[Mistakes are safe](configuration.md#mistakes-are-safe).

## How do I recolor my desktop when the wallpaper changes?

Set `on_change` in the Config. It runs after every new wallpaper, with
`{path}` and `{monitor}` filled in, and works with matugen, pywal and
wallust. See [Change hook](configuration.md#change-hook).

## Where does Vellum keep things?

| What | Where |
| --- | --- |
| The Config | `~/.config/vellum/config.toml` |
| What's on screen, the Rotation position | `~/.local/state/vellum/` |
| Favorites | `~/.local/share/vellum/favorites` |
| TUI thumbnails | `~/.cache/vellum/thumbs/` |
| The Daemon's log, when a `vellum` command started it | `~/.local/state/vellum/vellumd.log` |

The `XDG_*_HOME` variables move them. Under systemd the log goes to the
journal: `journalctl --user -u vellumd`.

[Back to the README](../README.md)
