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
theme = "midnight"
# on_change = "matugen image {path}"  # no hook by default

[transition]
type = "fade"
direction = "right"
duration = 600
easing = "ease-out"

[rotation]
interval = "30m"
source = "library"
order = "shuffle"
sync = false

# [rotation.monitor.DP-2]   # Overrides: none by default
# interval = "2h"

# [keymap]                  # TUI keys: see "Keys" below
# apply_all = "A"
[preview]
cache_mb = 200
```

TOML needs top-level keys (`library`, `fit`, `letterbox`, `theme`,
`on_change`) to come before the first `[section]`.

## Changes apply live

Save the file and the change takes effect. You do not need to restart
anything. The Daemon watches the Config directory and asks `vellum` to
re-read the file whenever it changes. `vellum status` shows the Transition
and Rotation the Daemon is using now.

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

## Theme

### `theme`

The TUI's colors (run `vellum` with no command to open it).

- `"midnight"`: Midnight Bloom, Vellum's own palette. This is the default.
  It needs a truecolor terminal (`COLORTERM=truecolor` or `24bit`); other
  terminals get the `"terminal"` look automatically.
- `"terminal"`: your terminal's own 16 colors and background, so the TUI
  matches your rice.

```toml
theme = "terminal"
```

If `NO_COLOR` is set, the TUI draws no color at all, whatever the theme,
and marks the selection with reverse video and bold.

## Previews

### `preview.cache_mb`

The TUI keeps the previews and grid thumbnails it renders on disk, in
`~/.cache/vellum/thumbs/` (or `$XDG_CACHE_HOME/vellum/thumbs/`), so the
next time you open it they show up at once. This caps how much space the
cache may use, in MiB. When it is full, the previews you looked at least
recently are deleted first. `0` turns the disk cache off.

- Type: a whole number of MiB.
- Default: `200`

```toml
[preview]
cache_mb = 500
```

An entry belongs to one file at one size, Monitor and Fit mode. Editing or
replacing a wallpaper makes its old entries unused, and they age out. You
can delete the folder at any time.

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

## Rotation

Rotation changes the wallpaper on a timer. It is off until you start it,
and once started it stays on across reboots until you stop it:

```sh
vellum rotation start
vellum rotation status   # source, order, interval and time to the next change
vellum rotation stop
```

The `[rotation]` section says how it rotates. Every key is optional:

```toml
[rotation]
interval = "30m"
source = "library"
order = "shuffle"
sync = false
```

### `rotation.interval`

How long each wallpaper stays up. Write it with units, such as `"30m"`,
`"1h30m"`, `"90s"` or `"250ms"`, or as a whole number of minutes (`15`).
The default is `"30m"`. A shorter interval takes effect at once.

### `rotation.source`

- `"library"`: every wallpaper in the Library. This is the default.
- `"favorites"`: only your Favorites (see below).

### `rotation.order`

- `"shuffle"`: a shuffled deck. No wallpaper comes back until every one
  has been shown, even across a reshuffle. This is the default.
- `"sequential"`: Library order (or the order you added Favorites). It
  remembers its position across restarts.

### `rotation.sync`

With `false` (the default), each monitor gets a different wallpaper on
each change. With `true`, they all show the same one and change together.

### Per-monitor Overrides

A `[rotation.monitor.<name>]` section gives one monitor its own
`interval`, `source` and `order`. Any key it leaves out comes from
`[rotation]`. A monitor with an Override rotates on its own and is never
part of `sync`. Run `vellum monitors` to see the names.

```toml
[rotation.monitor.DP-2]   # the portrait monitor
interval = "2h"
source = "favorites"
```

### Stepping by hand

`vellum next`, `vellum prev` and `vellum random` (each with an optional
`-m <monitor>`) change the wallpaper now, which makes them handy for
keybinds. `next` follows the Rotation rules above, `prev` goes back to the
wallpaper shown before, and `random` picks any wallpaper from the
Rotation's source. They work whether or not Rotation is running.

Any wallpaper you apply by hand, including `vellum set`, restarts that
monitor's timer, so it stays up for a full interval.

A file that cannot be decoded is skipped with a warning, and the next one
is tried, so Rotation never gets stuck.

## Favorites

Favorites are the wallpapers you star:

```sh
vellum fav add dunes.jpg   # a path or a Library name
vellum fav add             # the wallpaper on screen now (-m to pick a monitor)
vellum fav remove dunes.jpg
vellum fav list
```

They are kept in `~/.local/share/vellum/favorites` (or
`$XDG_DATA_HOME/vellum/favorites`), one path per line. A Favorite whose
file has gone missing, for example on an unplugged drive, is skipped
without an error and comes back when the file does.

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

In the TUI the warning shows as a toast instead.

## Keys

### `[keymap]`

Every key in the TUI can be changed. The footer hints, the `?` help
overlay and the `:` command palette are generated from the same keymap,
so they always show your keys.

- Type: a table of Action names, each set to a key or a list of keys
- Default: the keys in the table below

Setting an Action replaces all of its default keys. A key you give to one
Action is taken away from any other Action that had it by default. An
empty list leaves an Action with no key (it stays in the palette).

```toml
[keymap]
down = ["n", "down"]
up = ["e", "up"]
apply_all = "A"
help = "f1"
top = "g g"       # a sequence: g, then g again
quit = []         # Ctrl-C still quits
```

A key is a character (`"j"`, `"G"`, `"/"`), or one of `enter`, `esc`,
`tab`, `backspace`, `space`, `up`, `down`, `left`, `right`, `home`, `end`,
`pageup`, `pagedown` and `f1` to `f24`, optionally after `ctrl-` or `alt-`.
Write uppercase letters as themselves (`"G"`, not `"shift-g"`). Keys
separated by spaces form a sequence. If a key is both bound alone and the
start of a sequence, it waits 400 ms for the rest. By default `g` toggles
the grid and `g g` jumps to the top, so a lone `g` takes effect after that
pause, or at once if you press another key.

Browsing the list:

| Action | Default | Does |
|---|---|---|
| `down`, `up` | `j` `down`, `k` `up` | Move (a row of tiles in the grid) |
| `left`, `right` | `h` `left`, `l` `right` | Move between tiles in the grid |
| `grid` | `g` | Switch between the list and the thumbnail grid |
| `top`, `bottom` | `g g` `home`, `G` `end` | Jump to either end |
| `page_down`, `page_up` | `ctrl-d` `pagedown`, `ctrl-u` `pageup` | Half a page |
| `apply` | `enter` | Apply to the targeted monitor |
| `apply_all` | `a` | Apply to every monitor |
| `random` | `r` | Apply a random wallpaper from the list |
| `monitor` | `m` | Target the next monitor |
| `favorite` | `f` | Favorite or unfavorite |
| `favorites_only` | `F` | Show only Favorites |
| `search` | `/` | Fuzzy search the Library |
| `help` | `?` | Show every key |
| `palette` | `:` `ctrl-p` | Run any Action by name |
| `back` | `esc` | Clear the search and filter, or quit |
| `quit` | `q` | Quit |

In search, help and the palette (other keys type into the prompt):

| Action | Default | Does |
|---|---|---|
| `next`, `prev` | `down` `ctrl-n`, `up` `ctrl-p` | Next or previous row |
| `confirm` | `enter` | Keep the search, run the command, close help |
| `cancel` | `esc` | Clear and close |

`ctrl-c` always quits. The mouse works too: click to select a wallpaper, a
monitor in the header or a footer hint; double-click to apply; scroll with
the wheel.

An unknown Action, a key Vellum can't read, or one key given to two Actions
is an error, reported like any other Config mistake.
