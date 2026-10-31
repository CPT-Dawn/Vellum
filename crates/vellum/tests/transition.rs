//! Transitions end to end: every kind lands on the exact target frame, the
//! animated ones really animate, and none of them leave memory behind.

mod support;

use std::time::{Duration, Instant};

use image::{ImageFormat, Rgb, RgbImage};
use support::*;

const SMALL: Monitor = Monitor {
    width: 320,
    height: 240,
};

const BLACK: [u8; 3] = [0, 0, 0];

fn save(sway: &Sway, name: &str, color_at: impl Fn(u32, u32) -> [u8; 3]) -> String {
    let img = RgbImage::from_fn(320, 240, |x, y| Rgb(color_at(x, y)));
    let path = sway.dir().join(name);
    img.save_with_format(&path, ImageFormat::Png).unwrap();
    path.to_string_lossy().into_owned()
}

fn solid(sway: &Sway, name: &str, color: [u8; 3]) -> String {
    save(sway, name, |_, _| color)
}

/// Four flat quadrants, so a misplaced or mirrored frame cannot pass.
fn quadrants(sway: &Sway) -> String {
    save(sway, "quadrants.png", |x, y| match (x < 160, y < 120) {
        (true, true) => RED,
        (false, true) => GREEN,
        (true, false) => BLUE,
        (false, false) => WHITE,
    })
}

const QUADRANT_PROBES: [(u32, u32, [u8; 3]); 8] = [
    (40, 30, RED),
    (155, 115, RED),
    (280, 30, GREEN),
    (165, 115, GREEN),
    (40, 210, BLUE),
    (155, 125, BLUE),
    (280, 210, WHITE),
    (165, 125, WHITE),
];

fn is_quadrants(s: &RgbImage) -> bool {
    QUADRANT_PROBES
        .iter()
        .all(|&(x, y, c)| s.get_pixel(x, y).0 == c)
}

/// Every capture taken while a Transition played.
struct Film {
    shots: Vec<RgbImage>,
    /// The Daemon mapped pixel memory at some point: only the CPU Fade does.
    daemon_mapped_pixels: bool,
}

/// Screenshots until `done`, keeping every capture along the way.
fn film(sway: &Sway, done: impl Fn(&RgbImage) -> bool) -> Film {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut film = Film {
        shots: Vec::new(),
        daemon_mapped_pixels: false,
    };
    loop {
        film.daemon_mapped_pixels |= sway.daemon_maps_memfd();
        let shot = sway.screenshot("HEADLESS-1");
        let finished = done(&shot);
        film.shots.push(shot);
        if finished || Instant::now() > deadline {
            return film;
        }
    }
}

#[test]
fn every_transition_lands_on_the_target_frame() {
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon();
    let black = solid(&sway, "black.png", BLACK);
    let target = quadrants(&sway);
    let runs: &[&[&str]] = &[
        &["--transition", "none"],
        &["--transition", "fade", "--duration", "150"],
        &[
            "--transition",
            "wipe",
            "--direction",
            "left",
            "--duration",
            "150",
        ],
        &[
            "--transition",
            "wipe",
            "--direction",
            "right",
            "--duration",
            "150",
        ],
        &[
            "--transition",
            "wipe",
            "--direction",
            "up",
            "--duration",
            "150",
        ],
        &[
            "--transition",
            "wipe",
            "--direction",
            "down",
            "--duration",
            "150",
        ],
        &[
            "--transition",
            "slide",
            "--direction",
            "left",
            "--duration",
            "150",
        ],
        &[
            "--transition",
            "slide",
            "--direction",
            "right",
            "--duration",
            "150",
        ],
        &[
            "--transition",
            "slide",
            "--direction",
            "up",
            "--duration",
            "150",
        ],
        &[
            "--transition",
            "slide",
            "--direction",
            "down",
            "--duration",
            "150",
        ],
        // The default: Fade, 600 ms, ease-out.
        &[],
    ];
    for flags in runs {
        sway.vellum_ok(["set", &black, "--transition", "none"]);
        sway.screenshot_until("HEADLESS-1", |s| {
            s.get_pixel(40, 30).0 == BLACK && s.get_pixel(280, 210).0 == BLACK
        });
        let mut args = vec!["set", target.as_str()];
        args.extend_from_slice(flags);
        sway.vellum_ok(&args);
        let shot = sway.screenshot_until("HEADLESS-1", is_quadrants);
        for &(x, y, color) in &QUADRANT_PROBES {
            let actual = shot.get_pixel(x, y).0;
            assert_eq!(actual, color, "{flags:?}: pixel ({x}, {y})");
        }
        assert!(sway.daemon_alive(), "{flags:?}:\n{}", sway.daemon_log());
    }
}

/// A Transition must show in-between frames, not just swap at the end.
/// Returns whether the Daemon mapped pixel memory while it played.
fn assert_animates(sway: &Sway, flags: &[&str], between: impl Fn(&RgbImage) -> bool) -> bool {
    let red = solid(sway, "red.png", RED);
    let blue = solid(sway, "blue.png", BLUE);
    sway.vellum_ok(["set", &red, "--transition", "none"]);
    sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(160, 120).0 == RED);
    let mut args = vec!["set", blue.as_str(), "--duration", "1200"];
    args.extend_from_slice(flags);
    sway.vellum_ok(&args);
    let all_blue = |s: &RgbImage| {
        [(2, 2), (160, 120), (317, 237), (2, 237), (317, 2)]
            .iter()
            .all(|&(x, y)| s.get_pixel(x, y).0 == BLUE)
    };
    let film = film(sway, all_blue);
    assert!(
        all_blue(film.shots.last().unwrap()),
        "{flags:?} never finished:\n{}",
        sway.daemon_log()
    );
    assert!(
        film.shots.iter().any(&between),
        "{flags:?} showed no in-between frame in {} captures",
        film.shots.len()
    );
    film.daemon_mapped_pixels
}

/// Neither pure red nor pure blue at the centre: a blend of the two.
fn blended(s: &RgbImage) -> bool {
    let [r, g, b] = s.get_pixel(160, 120).0;
    g == 0 && (20..235).contains(&r) && (20..235).contains(&b)
}

#[test]
fn fade_blends_on_the_gpu_through_alpha_modifier() {
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon();
    let mapped = assert_animates(&sway, &["--transition", "fade"], blended);
    assert!(
        !mapped,
        "the GPU fade touched pixels in the Daemon (the e2e Sway needs \
         wp_alpha_modifier_v1, Sway 1.10 or newer)"
    );
}

#[test]
fn fade_falls_back_to_a_cpu_blend_without_alpha_modifier() {
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon_with(&[("VELLUMD_NO_ALPHA_MODIFIER", "1")]);
    let mapped = assert_animates(&sway, &["--transition", "fade"], blended);
    assert!(mapped, "the CPU fade never mapped its blend buffers");
}

#[test]
fn wipe_reveals_the_new_frame_behind_a_moving_edge() {
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon();
    // Moving right: blue appears on the left first while red holds the right.
    let mapped = assert_animates(
        &sway,
        &["--transition", "wipe", "--direction", "right"],
        |s| s.get_pixel(5, 120).0 == BLUE && s.get_pixel(314, 120).0 == RED,
    );
    assert!(!mapped, "a Wipe must move viewports, not blend pixels");
}

#[test]
fn slide_brings_the_new_frame_in_from_the_edge() {
    let mut sway = Sway::start(&[SMALL]);
    // Even without alpha-modifier a Slide never needs the CPU.
    sway.start_daemon_with(&[("VELLUMD_NO_ALPHA_MODIFIER", "1")]);
    // Moving up: blue enters from the bottom.
    let mapped = assert_animates(
        &sway,
        &["--transition", "slide", "--direction", "up"],
        |s| s.get_pixel(160, 235).0 == BLUE && s.get_pixel(160, 5).0 == RED,
    );
    assert!(!mapped, "a Slide must move viewports, not blend pixels");
}

#[test]
fn a_new_wallpaper_mid_transition_wins() {
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon();
    let red = solid(&sway, "red.png", RED);
    let blue = solid(&sway, "blue.png", BLUE);
    let green = solid(&sway, "green.png", GREEN);
    sway.vellum_ok(["set", &red, "--transition", "none"]);
    sway.vellum_ok(["set", &blue, "--transition", "fade", "--duration", "3000"]);
    sway.vellum_ok(["set", &green, "--transition", "wipe", "--duration", "100"]);
    let shot = sway.screenshot_until("HEADLESS-1", |s| {
        s.get_pixel(160, 120).0 == GREEN && s.get_pixel(317, 2).0 == GREEN
    });
    assert_pixel(&shot, 160, 120, GREEN, 0);
    // And it stays: the abandoned fade must not resurface.
    std::thread::sleep(Duration::from_millis(400));
    let shot = sway.screenshot("HEADLESS-1");
    assert_pixel(&shot, 160, 120, GREEN, 0);
    assert_pixel(&shot, 2, 2, GREEN, 0);
    assert_pixel(&shot, 317, 237, GREEN, 0);
    assert!(sway.daemon_alive(), "{}", sway.daemon_log());
}

#[test]
fn a_scale_change_mid_transition_lands_the_new_frame() {
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon();
    let red = solid(&sway, "red.png", RED);
    let blue = solid(&sway, "blue.png", BLUE);
    sway.vellum_ok(["set", &red, "--transition", "none"]);
    sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(160, 120).0 == RED);
    sway.vellum_ok(["set", &blue, "--transition", "wipe", "--duration", "20000"]);
    sway.swaymsg("output HEADLESS-1 scale 2");
    // Long before the 20 s Wipe would end, the frame covers the Monitor.
    let covered = |s: &RgbImage| {
        let (w, h) = s.dimensions();
        [(2, 2), (w - 3, 2), (2, h - 3), (w - 3, h - 3)]
            .iter()
            .all(|&(x, y)| s.get_pixel(x, y).0 == BLUE)
    };
    let shot = sway.screenshot_until("HEADLESS-1", covered);
    assert!(covered(&shot), "the Wipe did not land on the scale change");
    assert!(sway.daemon_alive(), "{}", sway.daemon_log());
}

#[test]
fn transitions_leave_no_memory_behind() {
    let mut sway = Sway::start(&[Monitor {
        width: 1920,
        height: 1080,
    }]);
    // The CPU path allocates the most: two blend buffers plus a mapping.
    sway.start_daemon_with(&[("VELLUMD_NO_ALPHA_MODIFIER", "1")]);
    let red = solid(&sway, "red.png", RED);
    let blue = solid(&sway, "blue.png", BLUE);
    sway.vellum_ok(["set", &red, "--transition", "none"]);
    sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(900, 500).0 == RED);
    std::thread::sleep(Duration::from_millis(200));
    let before = sway.daemon_rss_kib();
    for (i, kind) in ["fade", "wipe", "slide", "fade"].iter().enumerate() {
        let (file, color) = if i % 2 == 0 {
            (&blue, BLUE)
        } else {
            (&red, RED)
        };
        sway.vellum_ok(["set", file, "--transition", kind, "--duration", "500"]);
        let shot = sway.screenshot_until("HEADLESS-1", |s| {
            s.get_pixel(2, 2).0 == color && s.get_pixel(1917, 1077).0 == color
        });
        assert_pixel(&shot, 960, 540, color, 0);
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !sway.daemon_maps_memfd(),
        "a finished Transition left pixel memory mapped"
    );
    let after = sway.daemon_rss_kib();
    // One 1080p frame is 8100 KiB; allow a little heap jitter, not a frame.
    assert!(
        after <= before + 1024,
        "RSS grew from {before} KiB to {after} KiB"
    );
}

#[test]
fn an_unknown_transition_is_refused_with_the_choices() {
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon();
    let red = solid(&sway, "red.png", RED);
    let out = sway.vellum(["set", &red, "--transition", "spin"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("fade") && err.contains("wipe"), "{err}");
}
