# The README demo GIF, recorded with VHS (https://github.com/charmbracelet/vhs).
#
#   cargo build --release && vhs docs/demo/demo.tape
#
# Run it from the repo root. It drives the real CLI and TUI against a
# throwaway headless Sway (docs/demo/sandbox.sh), never your own session.
# Needs vhs (with ttyd and ffmpeg), sway and ImageMagick.

Output assets/demo.gif

Require sway
Require magick

Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set FontSize 18
Set Width 1400
Set Height 820
Set Padding 28
Set Framerate 30
Set TypingSpeed 55ms
Set CursorBlink false
Set Theme { "name": "Midnight Bloom", "background": "#0B0D14", "foreground": "#C0CAF5", "cursor": "#8DE3DC", "selection": "#1A1B2E", "black": "#15162A", "red": "#F7768E", "green": "#9ECE6A", "yellow": "#FF9E64", "blue": "#7AA2F7", "magenta": "#BB9AF7", "cyan": "#8DE3DC", "white": "#A9B1D6", "brightBlack": "#565F89", "brightRed": "#F7768E", "brightGreen": "#9ECE6A", "brightYellow": "#FF9E64", "brightBlue": "#7AA2F7", "brightMagenta": "#BB9AF7", "brightCyan": "#8DE3DC", "brightWhite": "#C0CAF5" }

Hide
Type "source docs/demo/sandbox.sh && export COLORTERM=truecolor VELLUM_GRAPHICS=halfblocks PS1='\[\e[38;2;141;227;220m\]❯\[\e[0m\] ' && clear"
Enter
Sleep 4s
Show

# The CLI: two Monitors, one command, a Wipe.
Type "vellum monitors"
Sleep 400ms
Enter
Sleep 1.5s
Type "vellum set dusk.jpg --transition wipe --direction up"
Sleep 400ms
Enter
Sleep 1.5s
Type "vellum status"
Sleep 400ms
Enter
Sleep 3s

# The TUI. VHS's terminal advertises sixel but records only text, so the
# previews use halfblocks (set above). Wait out the probe before typing.
Type "clear && vellum"
Enter
Sleep 3s
Type@350ms "jjj"
Sleep 1.5s
Type "/"
Sleep 500ms
Type@120ms "glac"
Sleep 1.2s
Enter
Sleep 800ms
Enter
Sleep 2s

# Rotation panel: start it, then close.
Type "s"
Sleep 1.5s
Space
Sleep 1.2s
Escape
Sleep 1s

# Back to the full list, a random pick, and out.
Escape
Sleep 800ms
Type "r"
Sleep 2s
Type "q"
Sleep 500ms

Hide
Type "vellum rotation stop; demo_stop"
Enter
Sleep 1s
