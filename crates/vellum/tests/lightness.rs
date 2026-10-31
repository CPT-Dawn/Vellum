//! Lightness gates: the Daemon idles small and asleep, and stays that way
//! after a Transition. CI fails here if either claim regresses. The full
//! comparison with other daemons is `bench/bench.sh` (docs/benchmarks.md).

mod support;

use std::time::Duration;

use image::{ImageFormat, Rgb, RgbImage};
use support::*;

/// Idle RSS budget in KiB, debug or release. vellumd measures about 2.5 MiB
/// (release) and 3 MiB (debug) here, most of it shared libc pages. Holding
/// even one frame for these Monitors (14 MiB at 1440p) blows straight
/// through it.
const IDLE_RSS_BUDGET_KIB: u64 = 4 * 1024;

/// Heap and stack the Daemon has written to. Pixels must never land here
/// (ADR 0003); about 160 KiB release, 300 KiB debug.
const PRIVATE_DIRTY_BUDGET_KIB: u64 = 1024;

/// Context switches tolerated in the idle window: zero expected, a couple
/// allowed for a stray compositor event.
const IDLE_SWITCH_BUDGET: u64 = 2;

const DESKTOP: [Monitor; 2] = [
    Monitor {
        width: 2560,
        height: 1440,
    },
    Monitor {
        width: 1920,
        height: 1080,
    },
];

fn wallpaper(sway: &Sway, name: &str, color: [u8; 3]) -> String {
    // 1080p: a Daemon that kept the decoded source (8 MiB) would show it too.
    let img = RgbImage::from_pixel(1920, 1080, Rgb(color));
    let path = sway.dir().join(name);
    img.save_with_format(&path, ImageFormat::Png).unwrap();
    path.to_string_lossy().into_owned()
}

fn wait_for_both(sway: &Sway, color: [u8; 3]) {
    for monitor in ["HEADLESS-1", "HEADLESS-2"] {
        sway.screenshot_until(monitor, |s| s.get_pixel(5, 5).0 == color);
    }
}

fn assert_light(sway: &Sway, when: &str) {
    // The final Fade frame can be on screen a callback or two before the
    // Transition is torn down, and a loaded CI box stretches that. Settled
    // means no pixel memory mapped, then a moment for buffer releases.
    sway.wait_until(&format!("the Daemon to unmap pixel memory {when}"), || {
        !sway.daemon_maps_memfd()
    });
    std::thread::sleep(Duration::from_millis(500));
    let rss = sway.daemon_smaps_kib("Rss");
    let dirty = sway.daemon_smaps_kib("Private_Dirty");
    let summary = format!("{when}: Rss {rss} KiB, Private_Dirty {dirty} KiB");
    eprintln!("{summary}");
    assert!(
        rss <= IDLE_RSS_BUDGET_KIB,
        "{summary}; the idle RSS budget is {IDLE_RSS_BUDGET_KIB} KiB"
    );
    assert!(
        dirty <= PRIVATE_DIRTY_BUDGET_KIB,
        "{summary}; the Private_Dirty budget is {PRIVATE_DIRTY_BUDGET_KIB} KiB"
    );
}

#[test]
fn the_daemon_idles_within_its_memory_budget() {
    let mut sway = Sway::start(&DESKTOP);
    sway.start_daemon();
    let red = wallpaper(&sway, "red.png", RED);
    sway.vellum_ok(["set", &red, "--transition", "none"]);
    wait_for_both(&sway, RED);
    assert_light(&sway, "after the first wallpaper");
}

#[test]
fn a_transition_leaves_the_daemon_within_its_memory_budget() {
    for env in [&[][..], &[("VELLUMD_NO_ALPHA_MODIFIER", "1")][..]] {
        let mut sway = Sway::start(&DESKTOP);
        sway.start_daemon_with(env);
        let red = wallpaper(&sway, "red.png", RED);
        let blue = wallpaper(&sway, "blue.png", BLUE);
        sway.vellum_ok(["set", &red, "--transition", "none"]);
        wait_for_both(&sway, RED);
        sway.vellum_ok(["set", &blue, "--transition", "fade", "--duration", "300"]);
        wait_for_both(&sway, BLUE);
        assert_light(&sway, &format!("after a Fade {env:?}"));
    }
}

#[test]
fn the_idle_daemon_does_not_wake_up() {
    let mut sway = Sway::start(&DESKTOP);
    sway.start_daemon();
    let red = wallpaper(&sway, "red.png", RED);
    sway.vellum_ok(["set", &red, "--transition", "none"]);
    wait_for_both(&sway, RED);
    std::thread::sleep(Duration::from_millis(500));
    let before = sway.daemon_context_switches();
    std::thread::sleep(Duration::from_secs(2));
    let woke = sway.daemon_context_switches() - before;
    assert!(
        woke <= IDLE_SWITCH_BUDGET,
        "the idle Daemon woke {woke} times in 2 s"
    );
}
