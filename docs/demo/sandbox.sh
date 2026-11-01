# Source this from bash at the repo root: it starts a throwaway headless Sway
# with two Monitors and points this shell at it, so the demo and screenshots
# never touch your own session, Config, State or cache.
#
#   source docs/demo/sandbox.sh   # then run vellum as usual
#   demo_stop                     # stop the Daemon and Sway
#
# Needs sway and ImageMagick (`magick`). Uses target/release binaries when
# they exist (`cargo build --release`), else whatever vellum is on PATH.

demo_repo=$(pwd)
demo_root=$(mktemp -d /tmp/vellum-demo.XXXXXX)
mkdir -p "$demo_root"/{config,state,cache,data,Pictures/Wallpapers}
cat > "$demo_root/sway.conf" <<'EOF'
default_border none
output HEADLESS-1 mode 1920x1080 position 0 0
output HEADLESS-2 mode 1080x1920 position 1920 0
EOF

[ -x "$demo_repo/target/release/vellum" ] && export PATH="$demo_repo/target/release:$PATH"
export HOME="$demo_root" XDG_RUNTIME_DIR="$demo_root"
export XDG_CONFIG_HOME="$demo_root/config" XDG_STATE_HOME="$demo_root/state"
export XDG_CACHE_HOME="$demo_root/cache" XDG_DATA_HOME="$demo_root/data"
unset WAYLAND_DISPLAY SWAYSOCK DISPLAY HYPRLAND_INSTANCE_SIGNATURE NIRI_SOCKET

WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
  WLR_HEADLESS_OUTPUTS=2 sway --config "$demo_root/sway.conf" \
  >"$demo_root/sway.log" 2>&1 &
demo_sway=$!

for _ in $(seq 100); do
  WAYLAND_DISPLAY=$(cd "$demo_root" && ls -d wayland-[0-9] 2>/dev/null | head -1 || true)
  SWAYSOCK=$(ls "$demo_root"/sway-ipc.*.sock 2>/dev/null | head -1 || true)
  [ -n "$WAYLAND_DISPLAY" ] && [ -n "$SWAYSOCK" ] && SWAYSOCK=$SWAYSOCK swaymsg -t get_outputs >/dev/null 2>&1 && break
  sleep 0.1
done
export WAYLAND_DISPLAY SWAYSOCK

# Smooth gradient wallpapers in the Midnight Bloom palette: a 3x3 grid of
# colors, upscaled with a cubic filter, plus a little grain.
demo_wallpaper() {
  local file=$1 size=$2 args=() i=0
  shift 2
  for c in "$@"; do
    args+=(-fill "$c" -draw "point $((i % 3)),$((i / 3))")
    i=$((i + 1))
  done
  magick -size 3x3 xc:black "${args[@]}" -filter Cubic -resize "$size!" \
    -attenuate 0.08 +noise Gaussian -quality 92 "$HOME/Pictures/Wallpapers/$file"
}
demo_wallpaper dusk.jpg 2560x1440 '#1A1B2E' '#5A3A6E' '#F7768E' '#15162A' '#7AA2F7' '#FF9E64' '#0B0D14' '#1A1B2E' '#5A3A6E'
demo_wallpaper ember-portrait.jpg 1440x2560 '#FF9E64' '#F7768E' '#5A3A6E' '#1A1B2E' '#5A3A6E' '#15162A' '#0B0D14' '#15162A' '#0E1019'
demo_wallpaper glacier.jpg 2560x1440 '#E6FFFC' '#8DE3DC' '#4E9EA2' '#8DE3DC' '#4E9EA2' '#1B4549' '#1B4549' '#0E1019' '#0B0D14'
demo_wallpaper harbor.jpg 2560x1440 '#1A1B2E' '#15162A' '#0E1019' '#4E9EA2' '#1B4549' '#15162A' '#FF9E64' '#5A3A6E' '#0B0D14'
demo_wallpaper indigo-portrait.jpg 1440x2560 '#0B0D14' '#7AA2F7' '#0B0D14' '#15162A' '#2A2F5A' '#15162A' '#0E1019' '#8DE3DC' '#0E1019'
demo_wallpaper lagoon.jpg 3840x2160 '#0B0D14' '#0E1019' '#1B4549' '#1B4549' '#4E9EA2' '#8DE3DC' '#0E1019' '#1B4549' '#4E9EA2'
demo_wallpaper meadow.jpg 2560x1440 '#090A10' '#0E1019' '#1B4549' '#1B4549' '#2E5A3A' '#9ECE6A' '#4E9EA2' '#9ECE6A' '#E0AF68'
demo_wallpaper moss.jpg 2560x1440 '#0B0D14' '#1B4549' '#9ECE6A' '#0E1019' '#2E5A3A' '#1B4549' '#090A10' '#0E1019' '#4E9EA2'
demo_wallpaper nightfall.jpg 2560x1440 '#15162A' '#0E1019' '#7AA2F7' '#0E1019' '#2A2F5A' '#15162A' '#090A10' '#0E1019' '#1A1B2E'
demo_wallpaper orchid.jpg 2560x1440 '#0B0D14' '#3A2A5E' '#7AA2F7' '#3A2A5E' '#BB9AF7' '#3A2A5E' '#7AA2F7' '#1A1B2E' '#0B0D14'
demo_wallpaper rosewater.jpg 2560x1440 '#F7768E' '#5A3A6E' '#15162A' '#5A3A6E' '#1A1B2E' '#0B0D14' '#15162A' '#0B0D14' '#090A10'
demo_wallpaper tidepool.jpg 2560x1440 '#0B0D14' '#1B4549' '#0E1019' '#1B4549' '#4E9EA2' '#15162A' '#0E1019' '#8DE3DC' '#1B4549'

demo_stop() {
  cd "$demo_repo" || return
  vellum kill >/dev/null 2>&1
  kill "$demo_sway" 2>/dev/null
  wait "$demo_sway" 2>/dev/null
  rm -rf "$demo_root"
}

cd "$HOME" || return
