# Benchmarks

How Vellum backs the "lightest Wayland wallpaper manager" claim, and how to
reproduce the numbers.

## Run it

```sh
bench/bench.sh                        # headless Sway: 2560x1440 + 1920x1080
bench/bench.sh --headless 3840x2160   # headless Sway with your Monitor sizes
bench/bench.sh --live                 # your running compositor, e.g. Hyprland
```

Needs `bash`, `python3`, `readelf` and, for headless runs, `sway`. It builds
release `vellum` and `vellumd` first, unless `VELLUM_BIN_DIR` points at
existing binaries. swaybg, hyprpaper and awww are taken from `PATH` (override
with `SWAYBG`, `HYPRPAPER`, `AWWW` and `AWWW_DAEMON`). The script skips any
that are missing and names them under the table. `--out FILE` also saves the
table.

Every daemon runs with a private `XDG_RUNTIME_DIR`, `HOME`, config, state and
cache in a temp dir, so the run never touches your files or your own Daemon.
In `--live` mode each daemon draws a test wallpaper on your Monitors for about
half a minute. Your own wallpaper is back once the run ends.

## Method

Each daemon runs alone, one after the other, against the same compositor and
the same two generated 3840x2160 PNG wallpapers:

1. Start the daemon and put wallpaper A on every Monitor, with no Transition.
2. Wait `--settle` seconds (default 3).
3. **Idle memory**: read `/proc/PID/smaps_rollup` for the daemon and any
   child processes. RSS counts every resident page, including shared libraries.
   PSS splits shared pages between the processes that map them. Private is
   `Private_Clean + Private_Dirty`, the memory only this daemon pays for.
4. **Idle wakeups**: sum `voluntary_ctxt_switches` and
   `nonvoluntary_ctxt_switches` over every thread, before and after an
   `--idle` window (default 20 s), scaled to a minute. A thread that wakes up
   switches at least once, so an idle daemon that never wakes reads 0.
5. **After a Transition**: switch to wallpaper B with a 600 ms Fade (`vellum
   set --transition fade --duration 600`, `awww img --transition-type fade
   --transition-duration 0.6`), wait `--settle` seconds, and read RSS again.
   swaybg and hyprpaper have no Transitions, so they show n/a.
6. **Binary**: the size of the daemon executable on disk, plus the number of
   shared libraries it links directly (`readelf -d`, `NEEDED`). A small
   binary that links a large stack isn't small, so read this column next to
   RSS.

## Results

Measured 2026-10-31 on headless Sway 1.12 (pixman), Monitors 2560x1440 +
1920x1080, Linux 7.2.11, 3840x2160 PNG wallpaper, 20 s idle window.

| Daemon | Version | Binary | Linked libs | Idle RSS | Idle PSS | Idle private | Wakeups/min idle | RSS after a Transition | Threads |
|---|---|---|---|---|---|---|---|---|---|
| vellumd | 2.0.0-dev | 446 KiB | 2 | 2.4 MiB | 0.5 MiB | 0.5 MiB | 0 | 2.4 MiB | 1 |

swaybg, hyprpaper and awww weren't installed on the measuring machine, so
there is no comparison yet. The Hyprland run on real Monitors is still to
come. For reference, Vellum 1.x idled at about 30 MB RSS, about 25 MB of it a
heap copy of every Monitor's frame.

vellumd holds no pixels (ADR 0003). Frames live in memfds that the CLI fills
and the compositor reads, and the Daemon only maps them during a CPU Fade.
RSS after a Transition therefore equals idle RSS, both with the
alpha-modifier Fade and with the CPU fallback, so mmap-backed pixel memory
and `malloc_trim` after Transitions weren't needed.

## CI gates

- `crates/vellum/tests/lightness.rs` runs the real Daemon on headless Sway
  with the 2560x1440 + 1920x1080 Monitors. It fails if idle RSS goes over
  4 MiB or Private_Dirty over 1 MiB (after the first wallpaper, and after a
  Fade on both the GPU and CPU paths), if the Daemon still maps a memfd once
  idle, or if it wakes more than twice in 2 s of idle.
- `.github/workflows/ci.yml` fails if the release `vellumd` grows past
  640 KiB.

Both budgets are deliberately above today's numbers. Tighten them as the
Daemon's feature set settles.
