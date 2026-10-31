# Configuration

Vellum works without any Config. Every setting has a sensible default, so
you only write down what you want to change.

The Config lives at `~/.config/vellum/config.toml`, or at
`$XDG_CONFIG_HOME/vellum/config.toml` if `XDG_CONFIG_HOME` is set. It is a
[TOML](https://toml.io) file you can keep in your dotfiles; a symlinked
file works too.

## A complete example

Every key, set to its default value:

```toml
library = ["~/Pictures/Wallpapers"]
fit = "fill"
letterbox = "#000000"
# on_change = "matugen image {path}"  # no hook by default

[transition]
type = "fade"
direction = "right"
duration = 600
easing = "ease-out"
```

TOML needs top-level keys (`library`, `fit`, `letterbox`, `on_change`) to
come before the first `[section]`.

## Changes apply live

Save the file and the change takes effect. You do not need to restart
anything. The Daemon watches the Config directory and asks `vellum` to
re-read the file whenever it changes. `vellum status` shows the Transition
the Daemon is using now.

To re-read the Config by hand and see any error in it, run:

```sh
vellum reload
```

## Mistakes are safe

If the Config has a mistake, Vellum tells you the line, the key and what it
expected, and keeps using the last good Config until you fix it:

```text
vellum: /home/you/.config/vellum/config.toml: line 6, `transition.duration`: invalid type: string "slow", expected a whole number of milliseconds, like 600
6 | duration = "slow"
               ^^^^^^
```

`vellum reload` prints the error and exits with status 1. `vellum set`
prints it as a warning and carries on with the last good values. When the
Daemon notices a broken edit by itself, the error goes to its log
(`~/.local/state/vellum/vellumd.log`, or `journalctl --user -u vellumd`).

Unknown keys are mistakes too, so a typo such as `typ = "wipe"` is reported
instead of being silently ignored.

## Library

### `library`

The folders Vellum searches for wallpapers. It searches each folder
recursively and follows symlinks. It skips hidden files and folders.
Folders that don't exist are skipped.

- Type: a list of paths. Each path is absolute or starts with `~/`.
- Default: `["~/Pictures/Wallpapers"]`

```toml
library = ["~/Pictures/Wallpapers", "~/Art", "/srv/shared/wallpapers"]
```

`vellum set` accepts a name from the Library as well as a path. If no file
matches the argument as given, Vellum looks for a Library wallpaper whose
path ends with it:

```sh
vellum set dunes.jpg          # finds ~/Pictures/Wallpapers/desert/dunes.jpg
vellum set desert/dunes.jpg   # more of the path, if two folders have a dunes.jpg
```

Wallpapers are JPEG, PNG and WebP files, plus AVIF and JXL when Vellum is
built with those features.

## Fit mode and letterbox

### `fit`

How a wallpaper is sized to a monitor when `vellum set` is run without
`--fit`.

- `"fill"`: cover the whole monitor, cropping what overflows. This is the default.
- `"fit"`: show the whole wallpaper, with bars in the `letterbox` color.
- `"center"`: actual size, centered, one wallpaper pixel per logical pixel.
- `"stretch"`: fill the monitor, ignoring the aspect ratio.

```toml
fit = "fit"
```

### `letterbox`

The color of the bars around a wallpaper in Fit mode `fit`, and behind
transparent parts of any wallpaper.

- Type: a color string, `"#rrggbb"`
- Default: `"#000000"`

```toml
letterbox = "#1a1b2e"
```

Changing it repaints the bars on monitors that already show a wallpaper in
Fit mode `fit`.

## Transition

The `[transition]` section sets the Transition that `vellum set` plays.
You can still override it per command with `--transition`, `--direction`
and `--duration`.

```toml
[transition]
type = "wipe"
direction = "up"
duration = 400
easing = "ease-in-out"
```

### `transition.type`

- `"fade"`: the new wallpaper fades in over the old one. This is the default.
- `"wipe"`: a moving edge reveals the new wallpaper in place.
- `"slide"`: the new wallpaper slides in over the old one.
- `"none"`: an instant swap.

### `transition.direction`

Which way a Wipe's edge or a Slide's wallpaper moves: `"left"`, `"right"`
(the default), `"up"` or `"down"`. Fade and none ignore it.

### `transition.duration`

How long the Transition takes, in whole milliseconds. The default is `600`.
`0` swaps instantly.

### `transition.easing`

The timing curve: `"linear"`, `"ease-in"`, `"ease-out"` (the default) or
`"ease-in-out"`.

## Change hook

### `on_change`

A command Vellum runs after every new wallpaper it puts on a monitor:
`vellum set`, `next`, `prev` and `random`, and each Rotation tick. Use it to
regenerate your color scheme so it follows your wallpaper.

- Type: a command line, as a string
- Default: none

In the command, `{path}` is replaced by the wallpaper's absolute path and
`{monitor}` by the monitor's connector name (see `vellum monitors`). The
hook runs once per monitor, so `vellum set` on two monitors runs it twice,
one after the other.

```toml
# matugen: Material You colors for GTK, Qt and anything with a template
on_change = "matugen image {path}"

# pywal: -n leaves the wallpaper to Vellum
on_change = "wal -n -i {path}"

# wallust
on_change = "wallust run {path}"
```

#### File names are safe

Vellum splits the command into words the way a shell would, honoring
quotes and backslashes, and then runs it directly, without a shell. `{path}`
always stays inside the word you wrote it in, so a wallpaper named
`$(rm -rf ~).png` or `it's here.jpg` reaches your tool as one plain
argument. Quoting the placeholder (`"{path}"`) is allowed and changes
nothing. A word starting with `~/` is expanded to your home folder.

Because there is no shell, pipes, `&&` and `$VARIABLES` don't work in the
command itself. To run several commands, point `on_change` at a script:

```toml
on_change = "~/.config/vellum/on-change.sh {path} {monitor}"
```

```sh
#!/bin/sh
matugen image "$1"
pkill -USR2 waybar
```

The hook also gets the values as environment variables, `VELLUM_PATH` and
`VELLUM_MONITOR`, so an inline shell works too. The single quotes keep the
script one word for `sh -c`:

```toml
on_change = "sh -c 'wallust run \"$VELLUM_PATH\" && pkill -USR2 waybar'"
```

Never write `{path}` inside an `sh -c` script: there it would be pasted into
shell code. Use `"$VELLUM_PATH"` instead.

#### When it runs, and when it fails

The hook runs after the Daemon has accepted the new wallpaper, so it never
delays or blocks the change. It is run by `vellum`, never by the Daemon.
`vellum set` waits for the hook to finish before it exits.

It does not run when Vellum only brings back a wallpaper you already had:
restoring wallpapers at login, after a monitor is plugged in or after a
scale change, or repainting the bars after a `letterbox` change.

If the hook cannot start or exits with a non-zero status, Vellum prints a
warning and nothing else changes: the wallpaper stays, and `vellum set`
still succeeds. For Rotation ticks the warning goes to the Daemon's log.

```text
vellum: the on_change hook `matugen` failed on DP-1 (exit status: 1)
```
