#!/usr/bin/env bash
# Regenerates the README screenshots in assets/screenshots/ from a headless
# Sway (never your own session): grim captures of two Monitors, and the TUI
# running in kitty with its graphics protocol.
#
#   cargo build --release && docs/demo/screenshots.sh
#
# Needs sway, grim, kitty (software GL is fine), ImageMagick and the
# JetBrainsMono Nerd Font.
set -euo pipefail
cd "$(dirname "$0")/../.."
# shellcheck source=sandbox.sh
source docs/demo/sandbox.sh
trap demo_stop EXIT
out="$demo_repo/assets/screenshots"
raw="$demo_root/raw"
mkdir -p "$out" "$raw"

# 1. The desktop: a landscape and a portrait Monitor, each with its own wallpaper.
echo "== desktop"
vellum set harbor.jpg -m HEADLESS-1 --transition none
vellum set indigo-portrait.jpg -m HEADLESS-2 --transition none
grim -o HEADLESS-1 "$raw/h1.png"
grim -o HEADLESS-2 "$raw/h2.png"

# 2. The TUI in kitty, full screen on HEADLESS-1, driven over kitty remote control.
echo "== TUI"
swaymsg focus output HEADLESS-1 >/dev/null
LIBGL_ALWAYS_SOFTWARE=1 kitty --config NONE --listen-on "unix:$demo_root/kitty.sock" \
  -o allow_remote_control=yes -o font_family="JetBrainsMono Nerd Font Mono" \
  -o font_size=21 -o background=#0B0D14 -o foreground=#C0CAF5 \
  -o window_padding_width=12 -o cursor_blink_interval=0 vellum \
  >"$demo_root/kitty.log" 2>&1 &
kitty_pid=$!
send() { kitten @ --to "unix:$demo_root/kitty.sock" "$@"; }
for _ in $(seq 100); do [ -S "$demo_root/kitty.sock" ] && break; sleep 0.1; done
sleep 4                                  # Library scan and thumbnails
send send-text lll && sleep 2            # move to the fourth tile
grim -o HEADLESS-1 "$raw/tui-grid.png"
send send-text g && sleep 1              # back to the list and preview
send send-text k && sleep 1               # glacier.jpg
send send-key enter && sleep 1.5         # apply, with its toast
grim -o HEADLESS-1 "$raw/tui-list.png"
send send-text q
wait "$kitty_pid"

# 3. Compose: Monitors laid out as Sway has them, TUI shots scaled for GitHub.
echo "== compose"
magick -size 1640x1060 xc:'#0B0D14' \
  \( "$raw/h1.png" -resize 960x540 -bordercolor '#565F89' -border 2 \) -geometry +40+260 -composite \
  \( "$raw/h2.png" -resize 540x960 -bordercolor '#565F89' -border 2 \) -geometry +1060+50 -composite \
  -quality 88 "$out/monitors.webp"
magick "$raw/tui-grid.png" -resize 1440x -quality 90 "$out/tui-grid.webp"
magick "$raw/tui-list.png" -resize 1440x -quality 90 "$out/tui-list.webp"
ls -l "$out"
