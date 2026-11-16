# Commands

Every `vellum` command and flag, as `vellum --help` prints them for 2.0.3.
`vellum help <command>` and `man vellum` say the same on your machine.

Run `vellum` with no command to open the TUI. Any command starts the daemon
first when it isn't running.

## Wallpapers

| Command | Does |
| --- | --- |
| `vellum set <FILE>` | Put a wallpaper on every monitor, or on one with `-m` |
| `vellum next` | Show the next wallpaper by the Rotation rules |
| `vellum prev` | Go back to the wallpaper shown before |
| `vellum random` | Show a random wallpaper from the Rotation's source |

`FILE` is an image path (JPEG, PNG or WebP) or a name in the Library, such as
`dusk.jpg`. All four commands take these flags:

| Flag | Values | Default |
| --- | --- | --- |
| `-m`, `--monitor <NAME>` | A connector name from `vellum monitors` | Every monitor |
| `--transition <KIND>` | `fade`, `wipe`, `slide`, `none` | From the Config, else `fade` |
| `--direction <DIRECTION>` | `left`, `right`, `up`, `down` (Wipe and Slide) | From the Config, else `right` |
| `--duration <MS>` | Milliseconds | From the Config, else `600` |

`vellum set` also takes `--fit <FIT>`, default from the Config, else `fill`:

| Fit | Does |
| --- | --- |
| `fill` | Cover the monitor and crop the overflow evenly from both sides |
| `fit` | Show the whole wallpaper, letterboxed with the letterbox color |
| `center` | Actual size (one wallpaper pixel per logical pixel), centered |
| `stretch` | Scale each axis independently to the monitor's size |

```sh
vellum set ~/Pictures/Wallpapers/dunes.jpg
vellum set dunes.jpg -m DP-1 --transition wipe --direction up
```

## Rotation and Favorites

| Command | Does |
| --- | --- |
| `vellum rotation start` | Start changing wallpapers on a timer; stays on across reboots |
| `vellum rotation stop` | Stop Rotation |
| `vellum rotation status` | Show each monitor's source, order, interval and next change |
| `vellum fav add [FILE]` | Add a wallpaper to Favorites (no file: the one on screen) |
| `vellum fav remove [FILE]` | Remove a wallpaper from Favorites (no file: the one on screen) |
| `vellum fav list` | List Favorites, skipping files that have gone missing |

`fav add` and `fav remove` take `-m <NAME>` to pick the monitor whose
wallpaper they mean. The interval, source and order of Rotation live in the
Config: see [Rotation](configuration.md#rotation).

## The daemon

| Command | Does |
| --- | --- |
| `vellum monitors` | List connected monitors with their size and scale |
| `vellum status` | Show the daemon, its memory use, and each monitor's wallpaper |
| `vellum reload` | Re-read the Config now and report any error in it |
| `vellum kill` | Stop the daemon |
| `vellum completions <SHELL>` | Print a completion script for `bash`, `zsh` or `fish` |

[Back to the README](../README.md)
