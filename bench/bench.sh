#!/usr/bin/env bash
# Measures vellumd against swaybg, hyprpaper and awww. Method: docs/benchmarks.md.
#
#   bench/bench.sh                         headless Sway, 2560x1440 + 1920x1080
#   bench/bench.sh --headless 3840x2160    headless Sway with your own Monitors
#   bench/bench.sh --live                  the running compositor (e.g. Hyprland)
#
# Options: --idle SECS (default 20), --settle SECS (default 3),
#          --image WxH (default 3840x2160), --out FILE (also write the table).
# Binaries come from PATH; override with SWAYBG, HYPRPAPER, AWWW, AWWW_DAEMON,
# and VELLUM_BIN_DIR (default: target/release, built first).
# BENCH_DEBUG=1 logs each measured process. Missing daemons are skipped.
# Nothing outside a temp dir is touched: every daemon gets a private
# XDG_RUNTIME_DIR, HOME, config, state and cache.
set -euo pipefail

mode=headless monitors="2560x1440,1920x1080" idle=20 settle=3 image=3840x2160 out=""
while (($#)); do
    case $1 in
    --headless) mode=headless; [[ ${2:-} =~ ^[0-9] ]] && { monitors=$2; shift; } ;;
    --live) mode=live ;;
    --idle) idle=$2; shift ;;
    --settle) settle=$2; shift ;;
    --image) image=$2; shift ;;
    --out) out=$2; shift ;;
    -h | --help) sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

repo=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/vellum-bench.XXXXXX")
run=$work/run
mkdir -p "$run" "$work/home" -m 700
sway_pid=""
cleanup() {
    [[ -n $sway_pid ]] && kill "$sway_pid" 2>/dev/null && wait "$sway_pid" 2>/dev/null
    rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

log() { printf '%s\n' "$*" >&2; }

# --- the compositor ---------------------------------------------------------

if [[ $mode == headless ]]; then
    command -v sway >/dev/null || { log "sway is not installed"; exit 1; }
    IFS=, read -ra sizes <<<"$monitors"
    : >"$work/sway.conf"
    for i in "${!sizes[@]}"; do
        echo "output HEADLESS-$((i + 1)) mode ${sizes[i]} position $((i * 10000)) 0" >>"$work/sway.conf"
    done
    env -i PATH="$PATH" HOME="$work/home" XDG_RUNTIME_DIR="$run" \
        WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
        WLR_HEADLESS_OUTPUTS=${#sizes[@]} \
        sway --config "$work/sway.conf" >/dev/null 2>"$work/sway.log" &
    sway_pid=$!
    for _ in $(seq 100); do
        display=$(find "$run" -maxdepth 1 -name 'wayland-*' ! -name '*.lock' -printf '%f\n' | head -1)
        [[ -n $display ]] && break
        sleep 0.1
    done
    [[ -n ${display:-} ]] || { log "sway did not start:"; cat "$work/sway.log" >&2; exit 1; }
    sleep 1 # outputs come up after the socket
    compositor="headless Sway $(sway --version | awk '{print $3}') (pixman), Monitors $monitors"
else
    [[ -n ${WAYLAND_DISPLAY:-} && -n ${XDG_RUNTIME_DIR:-} ]] || { log "--live needs a Wayland session"; exit 1; }
    display=${WAYLAND_DISPLAY##*/}
    real_socket=$WAYLAND_DISPLAY
    [[ $real_socket == /* ]] || real_socket=$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY
    ln -s "$real_socket" "$run/$display"
    if [[ -n ${HYPRLAND_INSTANCE_SIGNATURE:-} ]]; then
        # Hyprland's own IPC sockets, so hyprpaper finds its compositor.
        mkdir -p "$run/hypr/$HYPRLAND_INSTANCE_SIGNATURE"
        for s in "$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE"/.socket*.sock; do
            [[ -S $s ]] && ln -s "$s" "$run/hypr/$HYPRLAND_INSTANCE_SIGNATURE/"
        done
        compositor="Hyprland $(hyprctl version -j 2>/dev/null | sed -n 's/.*"tag": *"\([^"]*\)".*/\1/p' | head -1)"
    else
        compositor="${XDG_CURRENT_DESKTOP:-unknown compositor}"
    fi
    log "Live mode: each daemon briefly draws a test wallpaper on your Monitors."
    log "Your own wallpaper daemon keeps running; it is back once the run ends."
fi

# Every daemon and client runs with this environment and nothing else. An
# array, not only a function, so a backgrounded daemon's $! is the daemon.
sandbox=(env -i PATH="$PATH" HOME="$work/home" XDG_RUNTIME_DIR="$run" WAYLAND_DISPLAY="$display"
    XDG_CONFIG_HOME="$work/home/.config" XDG_STATE_HOME="$work/home/.local/state"
    XDG_CACHE_HOME="$work/home/.cache" XDG_DATA_HOME="$work/home/.local/share")
if [[ -n ${HYPRLAND_INSTANCE_SIGNATURE:-} ]]; then
    sandbox+=(HYPRLAND_INSTANCE_SIGNATURE="$HYPRLAND_INSTANCE_SIGNATURE")
fi
sandbox() { "${sandbox[@]}" "$@"; }

# --- wallpapers -------------------------------------------------------------

# Two vertical gradients at --image size. Content doesn't matter: every daemon
# here keeps decoded or scaled pixels, never the PNG.
python3 - "$work" "${image%x*}" "${image#*x}" <<'EOF'
import struct, sys, zlib
out, w, h = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
def png(path, top, bottom):
    rows = bytearray()
    for y in range(h):
        t = y / max(h - 1, 1)
        px = bytes(round(a + (b - a) * t) for a, b in zip(top, bottom))
        rows += b"\0" + px * w
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(bytes(rows), 1)) + chunk(b"IEND", b""))
png(f"{out}/a.png", (0x0B, 0x0D, 0x14), (0x8D, 0xE3, 0xDC))
png(f"{out}/b.png", (0x7A, 0xA2, 0xF7), (0xF7, 0x76, 0x8E))
EOF
A=$work/a.png B=$work/b.png

# --- measuring --------------------------------------------------------------

# The daemon's pid plus its children, if any.
tree() { echo "$1"; pgrep -P "$1" || true; }
debug_tree() { for p in $(tree "$1"); do log "   pid $p: $(tr "\0" " " </proc/$p/cmdline) $(grep ^Rss /proc/$p/smaps_rollup)"; done; }

# "RSS PSS PRIVATE" in KiB, summed over the process tree (smaps_rollup).
memory() {
    local files=()
    for p in $(tree "$1"); do files+=("/proc/$p/smaps_rollup"); done
    awk '/^Rss:/ {r += $2} /^Pss:/ {p += $2} /^Private_(Clean|Dirty):/ {v += $2}
         END {print r, p, v}' "${files[@]}"
}

# Context switches across every thread: each wakeup is at least one.
switches() {
    local p total=0
    for p in $(tree "$1"); do
        total=$((total + $(cat /proc/"$p"/task/*/status 2>/dev/null |
            awk '/ctxt_switches/ {s += $2} END {print s + 0}')))
    done
    echo "$total"
}

threads() { ls /proc/"$1"/task | wc -l; }
kib() { awk -v k="$1" 'BEGIN {printf (k >= 10240 ? "%.0f MiB" : "%.1f MiB"), k / 1024}'; }
size() { awk -v b="$(stat -Lc %s "$1")" 'BEGIN {printf (b >= 1048576 ? "%.1f MiB" : "%.0f KiB"), b / (b >= 1048576 ? 1048576 : 1024)}'; }
libs() { readelf -d "$1" 2>/dev/null | grep -c NEEDED || true; }
wait_for() { # wait_for SECS cmd...: poll until cmd succeeds
    local deadline=$((SECONDS + $1)); shift
    until "$@" >/dev/null 2>&1; do
        ((SECONDS < deadline)) || return 1
        sleep 0.1
    done
}

rows=() missing=()

# bench NAME VERSION BINARY START SET CHANGE
#   START launches the daemon in the foreground (we background it), SET puts
#   wallpaper A up once it is ready, CHANGE moves to B with a Transition
#   (empty when the daemon has none).
bench() {
    local name=$1 version=$2 bin=$3 start=$4 set=$5 change=$6
    log "== $name"
    "${sandbox[@]}" bash -c "exec $start" >"$work/$name.log" 2>&1 &
    local pid=$!
    if ! eval "$set" >>"$work/$name.log" 2>&1 || ! kill -0 "$pid" 2>/dev/null; then
        log "   $name failed to start; log:"; sed 's/^/   /' "$work/$name.log" >&2
        kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null || true
        rows+=("| $name | $version | failed to start | | | | | | |")
        return
    fi
    sleep "$settle"
    local idle_mem s0 s1 after="n/a (no Transitions)"
    [[ -n ${BENCH_DEBUG:-} ]] && debug_tree "$pid"
    read -ra idle_mem <<<"$(memory "$pid")"
    s0=$(switches "$pid"); sleep "$idle"; s1=$(switches "$pid")
    local per_min=$(((s1 - s0) * 60 / idle))
    if [[ -n $change ]]; then
        eval "$change" >>"$work/$name.log" 2>&1
        sleep "$settle"
        read -ra after_mem <<<"$(memory "$pid")"
        after=$(kib "${after_mem[0]}")
    fi
    rows+=("| $name | $version | $(size "$bin") | $(libs "$bin") | $(kib "${idle_mem[0]}") | $(kib "${idle_mem[1]}") | $(kib "${idle_mem[2]}") | $per_min | $after | $(threads "$pid") |")
    kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null || true
    log "   done"
}

# vellum
vbin=${VELLUM_BIN_DIR:-$repo/target/release}
if [[ -z ${VELLUM_BIN_DIR:-} ]]; then
    log "Building release vellum..."
    cargo build --release --locked --quiet --manifest-path "$repo/Cargo.toml" -p vellumd -p vellum-wallpaper
fi
vsock=$run/vellum-$display.sock
bench vellumd "$("$vbin/vellumd" --version | awk '{print $2}')" "$vbin/vellumd" \
    "'$vbin/vellumd'" \
    "wait_for 10 test -S '$vsock' && sandbox '$vbin/vellum' set '$A' --transition none \
        && sandbox '$vbin/vellum' monitors >'$work/monitors.txt'" \
    "sandbox '$vbin/vellum' set '$B' --transition fade --duration 600"
vellum_monitors=$(cat "$work/monitors.txt" 2>/dev/null || true)

# swaybg: one process, no IPC, no Transitions.
if bin=${SWAYBG:-$(command -v swaybg)}; then
    bench swaybg "$("$bin" -v 2>&1 | awk '{print $NF}')" "$bin" \
        "'$bin' -m fill -i '$A'" "sleep 1" ""
else missing+=(swaybg); fi

# hyprpaper: reads its config at start. 0.8+ uses wallpaper {} blocks.
if bin=${HYPRPAPER:-$(command -v hyprpaper)}; then
    version=$("$bin" --version 2>&1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)
    if [[ $(printf '%s\n' "0.8.0" "$version" | sort -V | head -1) == 0.8.0 ]]; then
        printf 'wallpaper {\n    monitor =\n    path = %s\n    fit_mode = cover\n}\nipc = false\nsplash = false\n' "$A" >"$work/hyprpaper.conf"
    else
        printf 'preload = %s\nwallpaper = ,%s\nipc = off\nsplash = false\n' "$A" "$A" >"$work/hyprpaper.conf"
    fi
    bench hyprpaper "$version" "$bin" "'$bin' -c '$work/hyprpaper.conf'" "sleep 1" ""
else missing+=(hyprpaper); fi

# awww: a daemon plus a client.
if bin=${AWWW_DAEMON:-$(command -v awww-daemon)} && client=${AWWW:-$(command -v awww)}; then
    bench awww "$("$client" --version 2>&1 | awk '{print $NF}')" "$bin" "'$bin'" \
        "wait_for 10 sandbox '$client' query && sandbox '$client' img '$A' --transition-type none" \
        "sandbox '$client' img '$B' --transition-type fade --transition-duration 0.6"
else missing+=(awww); fi

# --- report -----------------------------------------------------------------

{
    echo "Measured $(date -u +%Y-%m-%d) on $compositor; $(uname -sr); wallpaper ${image} PNG; idle window ${idle}s."
    echo
    echo "| Daemon | Version | Binary | Linked libs | Idle RSS | Idle PSS | Idle private | Wakeups/min idle | RSS after a Transition | Threads |"
    echo "|---|---|---|---|---|---|---|---|---|---|"
    printf '%s\n' "${rows[@]}"
    if ((${#missing[@]})); then echo; echo "Not installed, skipped: ${missing[*]}."; fi
    if [[ -n $vellum_monitors ]]; then echo; echo '```'; echo "$vellum_monitors"; echo '```'; fi
} | tee ${out:+"$out"}
