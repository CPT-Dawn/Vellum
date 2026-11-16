//! Clicks on the bare desktop must reach the compositor (a root menu, a
//! binding) instead of being swallowed by the wallpaper: the Daemon's
//! surfaces take no pointer input.

mod support;

use std::time::Duration;

use support::*;

const SIZE: Monitor = Monitor {
    width: 320,
    height: 240,
};

/// Binds a right click anywhere to creating a marker file, clicks on the
/// desktop and reports whether Sway saw the click.
fn desktop_click_reaches_sway(sway: &Sway) -> bool {
    let marker = sway.dir().join("clicked");
    sway.swaymsg(&format!(
        "bindsym --whole-window button3 exec touch {}",
        marker.display()
    ));
    sway.swaymsg("seat - cursor set 100 100");
    sway.swaymsg("seat - cursor press button3");
    sway.swaymsg("seat - cursor release button3");
    for _ in 0..50 {
        if marker.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn control_a_bare_desktop_click_reaches_sway() {
    let sway = Sway::start(&[SIZE]);
    assert!(desktop_click_reaches_sway(&sway));
}

fn wallpaper(sway: &Sway, color: [u8; 3]) -> String {
    let path = sway.dir().join("wall.png");
    image::RgbImage::from_pixel(64, 48, image::Rgb(color))
        .save(&path)
        .unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn a_desktop_click_falls_through_the_wallpaper() {
    let mut sway = Sway::start(&[SIZE]);
    sway.start_daemon();
    let path = wallpaper(&sway, RED);
    sway.vellum_ok(["set", &path, "--transition", "none"]);
    assert!(
        desktop_click_reaches_sway(&sway),
        "the wallpaper surface swallowed the click:\n{}",
        sway.daemon_log()
    );
}

#[test]
fn a_desktop_click_falls_through_a_running_transition() {
    let mut sway = Sway::start(&[SIZE]);
    sway.start_daemon();
    let path = wallpaper(&sway, RED);
    sway.vellum_ok(["set", &path, "--transition", "none"]);
    let path = wallpaper(&sway, GREEN);
    sway.vellum_ok(["set", &path, "--transition", "wipe", "--duration", "5000"]);
    assert!(
        desktop_click_reaches_sway(&sway),
        "the transition surface swallowed the click:\n{}",
        sway.daemon_log()
    );
}
