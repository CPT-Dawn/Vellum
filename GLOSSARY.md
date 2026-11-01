# Vellum

Vellum is a lightweight wallpaper manager for Wayland compositors that support layer-shell. It puts still images on the desktop background, transitions smoothly between them, and rotates them automatically.

## Language

**Wallpaper**:
A still image file shown as the background of one or more monitors.
_Avoid_: Background, image (when the domain meaning is intended)

**Monitor**:
A physical display that shows its own wallpaper, identified by its connector name (e.g. `DP-1`).
_Avoid_: Output, screen, display

**Fit mode**:
How a wallpaper is sized to a monitor: Fill (cover and crop), Fit (letterbox), Center (actual size), or Stretch.
_Avoid_: Scaling mode, resize strategy

**Library**:
The set of folders Vellum searches recursively for wallpapers.
_Avoid_: Workspace, collection, gallery

**Favorite**:
A wallpaper the user has starred so it is easy to find again or to rotate through.
_Avoid_: Bookmark, starred item

**Rotation**:
Automatically changing the wallpaper on a fixed interval, in shuffled or sequential order, drawing from the Library or from Favorites.
_Avoid_: Playlist, slideshow, cycle

**Override**:
A per-monitor Rotation setting that replaces the global Rotation for that monitor only.
_Avoid_: Exception, per-monitor playlist

**Transition**:
The visual effect that plays while one wallpaper replaces another.
Vellum offers Fade, Wipe, Slide and None.
_Avoid_: Animation, effect

**Daemon**:
The single always-running Vellum process that keeps wallpapers on screen and runs Rotation.
_Avoid_: Server, backend, worker

**Config**:
The user's optional, hand-editable preferences. Every setting has a sensible default.
_Avoid_: Settings file, preferences

**State**:
What Vellum remembers on its own, such as which wallpaper each monitor shows, so it can restore them after a reboot.
_Avoid_: Cache, session
