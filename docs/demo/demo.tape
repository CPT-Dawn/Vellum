# The README demo, rendered with VHS (https://github.com/charmbracelet/vhs).
#
#   cargo build --release
#   vhs docs/demo/demo.tape    # run from the repo root; writes assets/demo.mp4
#   ~/.claude/skills/readme/scripts/readme-assets gif assets/demo.mp4 assets/demo.gif
#
# It drives the real TUI against a throwaway headless Sway with two Monitors
# and a seeded HOME (docs/demo/sandbox.sh), never your own session.
# Needs vhs (with ttyd and ffmpeg), sway and ImageMagick.

Output assets/demo.mp4

Require sway
Require magick

Set Shell "bash"
Set FontFamily "JetBrainsMono Nerd Font Mono"
Set FontSize 32
Set LineHeight 1.2
Set Width 2560
Set Height 1600
Set Padding 36
Set Margin 56
Set MarginFill "#090A10"
Set BorderRadius 16
Set WindowBar Colorful
Set WindowBarSize 48
Set Framerate 30
Set TypingSpeed 90ms
Set CursorBlink false
Set Theme { "name": "Midnight Bloom", "background": "#0B0D14", "foreground": "#C0CAF5", "cursor": "#8DE3DC", "selection": "#1A1B2E", "black": "#15162A", "red": "#F7768E", "green": "#9ECE6A", "yellow": "#FF9E64", "blue": "#7AA2F7", "magenta": "#BB9AF7", "cyan": "#8DE3DC", "white": "#A9B1D6", "brightBlack": "#565F89", "brightRed": "#F7768E", "brightGreen": "#9ECE6A", "brightYellow": "#FF9E64", "brightBlue": "#7AA2F7", "brightMagenta": "#BB9AF7", "brightCyan": "#8DE3DC", "brightWhite": "#C0CAF5" }

# Launch off camera so the first frame is the populated TUI. VHS's terminal
# advertises sixel but records only text, so previews use halfblocks.
Hide
Type "source docs/demo/sandbox.sh && export COLORTERM=truecolor VELLUM_GRAPHICS=halfblocks && vellum set dusk.jpg --transition none >/dev/null && clear && vellum"
Enter
Sleep 14s
Show

# Beat 1: the Library and a true-to-monitor preview.
Sleep 2s
Type@400ms "jj"
Sleep 1.5s

# Beat 2: the thumbnail grid.
Type "g"
Sleep 1s
Type@400ms "ll"
Sleep 2s
Type "g"
Sleep 1s

# Beat 3: fuzzy search, then apply to the targeted monitor.
Type "/"
Sleep 500ms
Type@150ms "glac"
Sleep 1.2s
Enter
Sleep 600ms
Enter
Sleep 2s

# Beat 4: the portrait monitor gets its own wallpaper.
Type "m"
Sleep 1s
Escape
Sleep 500ms
Type "/"
Sleep 300ms
Type@150ms "ember"
Sleep 800ms
Enter
Sleep 600ms
Enter
Sleep 2s

# Beat 5: start Rotation from its panel.
Type "s"
Sleep 1.5s
Space
Sleep 1.5s
Escape
Sleep 600ms

# Close on the full list, where it opened, so the loop is seamless.
Escape
Type "m"
Sleep 400ms
Type "k"
Sleep 2s

Hide
Type "q"
Sleep 300ms
Type "vellum rotation stop; demo_stop"
Enter
Sleep 1s
