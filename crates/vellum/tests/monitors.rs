//! Several Monitors end to end: targeting with `-m`, `vellum monitors`,
//! hotplug and fractional scale.

mod support;

use image::{ImageFormat, Rgb, RgbImage};
use support::*;

const SMALL: Monitor = Monitor {
    width: 320,
    height: 240,
};
const TINY: Monitor = Monitor {
    width: 200,
    height: 150,
};

const BLACK: [u8; 3] = [0, 0, 0];

fn paint(width: u32, height: u32, color_at: impl Fn(u32, u32) -> [u8; 3]) -> RgbImage {
    RgbImage::from_fn(width, height, |x, y| Rgb(color_at(x, y)))
}

fn solid(sway: &Sway, color: [u8; 3]) -> String {
    let name = format!("solid-{:02x}{:02x}{:02x}.png", color[0], color[1], color[2]);
    save(sway, &name, &paint(8, 8, |_, _| color))
}

fn save(sway: &Sway, name: &str, img: &RgbImage) -> String {
    let path = sway.dir().join(name);
    img.save_with_format(&path, ImageFormat::Png).unwrap();
    path.to_string_lossy().into_owned()
}

/// One row of `vellum monitors`.
#[derive(Debug, PartialEq)]
struct Row {
    name: String,
    logical: String,
    physical: String,
    scale: String,
}

fn monitors(sway: &Sway) -> Vec<Row> {
    let out = sway.vellum_stdout(["monitors"]);
    let mut lines = out.lines();
    let header: Vec<&str> = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect();
    assert_eq!(header, ["MONITOR", "LOGICAL", "PHYSICAL", "SCALE"], "{out}");
    lines
        .map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            assert_eq!(cols.len(), 4, "unexpected row {line:?} in:\n{out}");
            Row {
                name: cols[0].into(),
                logical: cols[1].into(),
                physical: cols[2].into(),
                scale: cols[3].into(),
            }
        })
        .collect()
}

fn row(name: &str, logical: &str, physical: &str, scale: &str) -> Row {
    Row {
        name: name.into(),
        logical: logical.into(),
        physical: physical.into(),
        scale: scale.into(),
    }
}

fn center_is(sway: &Sway, monitor: &str, color: [u8; 3]) -> bool {
    let shot = sway.screenshot(monitor);
    let (w, h) = shot.dimensions();
    shot.get_pixel(w / 2, h / 2).0 == color
}

#[test]
fn set_without_a_monitor_targets_every_monitor() {
    let mut sway = Sway::start(&[SMALL, TINY]);
    sway.start_daemon();

    sway.vellum_ok(["set", &solid(&sway, GREEN)]);

    for (name, (w, h)) in [("HEADLESS-1", (320, 240)), ("HEADLESS-2", (200, 150))] {
        let shot = sway.screenshot_until(name, |s| s.get_pixel(w / 2, h / 2).0 == GREEN);
        assert_eq!(shot.dimensions(), (w, h));
        for (x, y) in [(1, 1), (w / 2, h / 2), (w - 2, h - 2)] {
            assert_pixel(&shot, x, y, GREEN, 0);
        }
    }
}

#[test]
fn set_with_a_monitor_changes_only_that_monitor() {
    let mut sway = Sway::start(&[SMALL, TINY]);
    sway.start_daemon();
    sway.vellum_ok(["set", &solid(&sway, WHITE)]);
    sway.wait_until("white everywhere", || {
        center_is(&sway, "HEADLESS-1", WHITE) && center_is(&sway, "HEADLESS-2", WHITE)
    });

    sway.vellum_ok(["set", &solid(&sway, BLUE), "-m", "HEADLESS-2"]);
    sway.wait_until("HEADLESS-2 turns blue", || {
        center_is(&sway, "HEADLESS-2", BLUE)
    });
    assert!(center_is(&sway, "HEADLESS-1", WHITE), "HEADLESS-1 changed");

    sway.vellum_ok(["set", &solid(&sway, RED), "--monitor", "HEADLESS-1"]);
    sway.wait_until("HEADLESS-1 turns red", || {
        center_is(&sway, "HEADLESS-1", RED)
    });
    assert!(center_is(&sway, "HEADLESS-2", BLUE), "HEADLESS-2 changed");
}

#[test]
fn an_unknown_monitor_is_a_clear_error_naming_the_real_ones() {
    let mut sway = Sway::start(&[SMALL, TINY]);
    sway.start_daemon();

    let out = sway.vellum(["set", &solid(&sway, RED), "-m", "DP-9"]);

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    for expected in ["DP-9", "HEADLESS-1", "HEADLESS-2"] {
        assert!(stderr.contains(expected), "stderr: {stderr}");
    }
}

#[test]
fn monitors_lists_name_logical_and_physical_size_and_scale() {
    let mut sway = Sway::start(&[
        SMALL,
        Monitor {
            width: 300,
            height: 240,
        },
    ]);
    sway.swaymsg("output HEADLESS-2 scale 1.5");
    sway.start_daemon();

    assert_eq!(
        monitors(&sway),
        [
            row("HEADLESS-1", "320x240", "320x240", "1"),
            row("HEADLESS-2", "200x160", "300x240", "1.5"),
        ]
    );
}

#[test]
fn a_monitor_plugged_in_at_runtime_can_be_targeted() {
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon();

    sway.swaymsg("create_output");
    sway.swaymsg("output HEADLESS-2 mode 200x150 position 10000 0");
    sway.wait_until("HEADLESS-2 to appear", || {
        monitors(&sway)
            .iter()
            .any(|m| m.name == "HEADLESS-2" && m.physical == "200x150")
    });

    sway.vellum_ok(["set", &solid(&sway, GREEN), "-m", "HEADLESS-2"]);
    let shot = sway.screenshot_until("HEADLESS-2", |s| s.get_pixel(100, 75).0 == GREEN);
    assert_eq!(shot.dimensions(), (200, 150));
    assert_pixel(&shot, 100, 75, GREEN, 0);
    assert_pixel(&shot, 198, 148, GREEN, 0);
    assert!(sway.daemon_alive(), "vellumd died:\n{}", sway.daemon_log());
}

#[test]
fn a_monitor_unplugged_at_runtime_disappears_and_the_rest_still_work() {
    let mut sway = Sway::start(&[SMALL, TINY]);
    sway.start_daemon();
    sway.vellum_ok(["set", &solid(&sway, WHITE)]);

    sway.swaymsg("output HEADLESS-2 unplug");
    sway.wait_until("HEADLESS-2 to disappear", || {
        monitors(&sway).iter().all(|m| m.name != "HEADLESS-2")
    });

    assert!(sway.daemon_alive(), "vellumd died:\n{}", sway.daemon_log());
    assert_eq!(
        monitors(&sway),
        [row("HEADLESS-1", "320x240", "320x240", "1")]
    );
    let out = sway.vellum(["set", &solid(&sway, RED), "-m", "HEADLESS-2"]);
    assert!(!out.status.success(), "targeting a gone monitor succeeded");

    sway.vellum_ok(["set", &solid(&sway, RED)]);
    sway.wait_until("HEADLESS-1 turns red", || {
        center_is(&sway, "HEADLESS-1", RED)
    });
}

/// One-pixel stripes survive only if the frame is not resampled on the way
/// to the screen.
fn assert_pixel_exact_at(scale: &str, width: u32, height: u32, logical: &str) {
    let mut sway = Sway::start(&[Monitor { width, height }]);
    sway.swaymsg(&format!("output HEADLESS-1 scale {scale}"));
    sway.start_daemon();
    let physical = format!("{width}x{height}");
    assert_eq!(
        monitors(&sway),
        [row("HEADLESS-1", logical, &physical, scale)]
    );

    let stripes = paint(width, height, |x, y| match (x + y) % 3 {
        0 => RED,
        1 => GREEN,
        _ => BLUE,
    });
    sway.vellum_ok(["set", &save(&sway, "stripes.png", &stripes)]);

    let shot = sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(0, 0).0 == RED);
    assert_eq!(shot.dimensions(), (width, height));
    for (x, y, px) in stripes.enumerate_pixels() {
        assert_pixel(&shot, x, y, px.0, 0);
    }
}

#[test]
fn frames_are_pixel_exact_at_scale_1_5() {
    assert_pixel_exact_at("1.5", 300, 240, "200x160");
}

#[test]
fn frames_are_pixel_exact_at_scale_1_25() {
    assert_pixel_exact_at("1.25", 320, 240, "256x192");
}

/// Four flat quadrants, so a cropped or shifted frame shows.
fn quadrants(sway: &Sway) -> String {
    let img = paint(320, 240, |x, y| match (x < 160, y < 120) {
        (true, true) => RED,
        (false, true) => GREEN,
        (true, false) => BLUE,
        (false, false) => WHITE,
    });
    save(sway, "quadrants.png", &img)
}

/// Samples near each quadrant's centre and each monitor corner.
fn assert_quadrants(shot: &RgbImage) {
    for (x, y, color) in [
        (80, 60, RED),
        (240, 60, GREEN),
        (80, 180, BLUE),
        (240, 180, WHITE),
        (2, 2, RED),
        (317, 2, GREEN),
        (2, 237, BLUE),
        (317, 237, WHITE),
    ] {
        assert_ne!(shot.get_pixel(x, y).0, BLACK, "black at ({x}, {y})");
        assert_pixel(shot, x, y, color, 0);
    }
}

/// After a scale change the old frame keeps covering the whole monitor,
/// never cropped and never uncovering black, until a frame for the new size
/// arrives. Then a new `set` renders at the new physical size.
fn assert_scale_change_keeps_the_wallpaper(from: &str, to: &str, logical_after: &str) {
    let mut sway = Sway::start(&[SMALL]);
    sway.swaymsg(&format!("output HEADLESS-1 scale {from}"));
    sway.start_daemon();
    let wallpaper = quadrants(&sway);
    sway.vellum_ok(["set", &wallpaper]);
    let shot = sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(80, 60).0 == RED);
    assert_quadrants(&shot);

    sway.swaymsg(&format!("output HEADLESS-1 scale {to}"));
    sway.wait_until("the monitors listing to catch up", || {
        monitors(&sway)[0] == row("HEADLESS-1", logical_after, "320x240", to)
    });
    for _ in 0..10 {
        assert_quadrants(&sway.screenshot("HEADLESS-1"));
    }

    sway.vellum_ok(["set", &solid(&sway, BLACK)]);
    sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(80, 60).0 == BLACK);
    sway.vellum_ok(["set", &wallpaper]);
    let shot = sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(80, 60).0 == RED);
    assert_quadrants(&shot);
}

#[test]
fn shrinking_the_scale_keeps_the_wallpaper_on_screen() {
    assert_scale_change_keeps_the_wallpaper("2", "1", "320x240");
}

#[test]
fn growing_the_scale_keeps_the_wallpaper_on_screen() {
    assert_scale_change_keeps_the_wallpaper("1", "2", "160x120");
}

#[test]
fn changing_to_a_fractional_scale_keeps_the_wallpaper_on_screen() {
    assert_scale_change_keeps_the_wallpaper("1", "1.25", "256x192");
}
