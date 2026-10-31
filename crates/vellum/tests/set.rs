//! `vellum set` end to end: real Daemon, headless Sway, grim screenshots.

mod support;

use image::{ImageFormat, Rgb, RgbImage};
use support::*;

const SMALL: Monitor = Monitor {
    width: 320,
    height: 240,
};

/// A `width`×`height` image whose columns are painted by `color_at(x, y)`.
fn paint(width: u32, height: u32, color_at: impl Fn(u32, u32) -> [u8; 3]) -> RgbImage {
    RgbImage::from_fn(width, height, |x, y| Rgb(color_at(x, y)))
}

fn save(sway: &Sway, name: &str, img: &RgbImage, format: ImageFormat) -> String {
    let path = sway.dir().join(name);
    img.save_with_format(&path, format).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn set_puts_the_wallpaper_on_the_monitor() {
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon();
    // Four quadrants at exactly twice the monitor size.
    let img = paint(640, 480, |x, y| match (x < 320, y < 240) {
        (true, true) => RED,
        (false, true) => GREEN,
        (true, false) => BLUE,
        (false, false) => WHITE,
    });
    let path = save(&sway, "quadrants.png", &img, ImageFormat::Png);

    sway.vellum_ok(["set", &path]);

    let shot = sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(80, 60).0 == RED);
    assert_eq!(shot.dimensions(), (320, 240));
    assert_pixel(&shot, 80, 60, RED, 0);
    assert_pixel(&shot, 240, 60, GREEN, 0);
    assert_pixel(&shot, 80, 180, BLUE, 0);
    assert_pixel(&shot, 240, 180, WHITE, 0);
}

#[test]
fn fill_covers_the_monitor_and_crops_the_overflow() {
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon();
    // A wide image in thirds: only the middle third fits a 4:3 monitor.
    let wide = paint(960, 240, |x, _| match x / 320 {
        0 => RED,
        1 => GREEN,
        _ => BLUE,
    });
    let wide = save(&sway, "wide.png", &wide, ImageFormat::Png);
    sway.vellum_ok(["set", &wide]);
    let shot = sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(160, 120).0 == GREEN);
    for x in [2, 160, 317] {
        assert_pixel(&shot, x, 120, GREEN, 0);
    }

    // A tall image, upscaled: only the middle third of its height shows.
    let tall = paint(160, 360, |_, y| match y / 120 {
        0 => RED,
        1 => BLUE,
        _ => GREEN,
    });
    let tall = save(&sway, "tall.png", &tall, ImageFormat::Png);
    sway.vellum_ok(["set", &tall]);
    let shot = sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(160, 120).0 == BLUE);
    for y in [10, 120, 230] {
        assert_pixel(&shot, 160, y, BLUE, 0);
    }
}

#[test]
fn jpeg_png_and_webp_wallpapers_all_work() {
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon();
    let halves = paint(320, 240, |x, _| if x < 160 { RED } else { BLUE });

    for (name, format, tolerance) in [
        ("halves.jpg", ImageFormat::Jpeg, 12),
        ("halves.png", ImageFormat::Png, 0),
        ("halves.webp", ImageFormat::WebP, 0),
    ] {
        // Reset to white so each format proves itself.
        let white = save(
            &sway,
            "white.png",
            &paint(8, 8, |_, _| WHITE),
            ImageFormat::Png,
        );
        sway.vellum_ok(["set", &white, "--transition", "none"]);
        sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(80, 120).0 == WHITE);

        let path = save(&sway, name, &halves, format);
        sway.vellum_ok(["set", &path, "--transition", "none"]);
        let shot = sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(80, 120).0 != WHITE);
        assert_pixel(&shot, 80, 120, RED, tolerance);
        assert_pixel(&shot, 240, 120, BLUE, tolerance);
    }
}

#[test]
fn an_undecodable_file_is_a_clear_error_and_the_daemon_survives() {
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon();
    let bogus = sway.dir().join("not-really.jpg");
    std::fs::write(&bogus, b"this is not a jpeg").unwrap();

    let out = sway.vellum(["set".as_ref(), bogus.as_os_str()]);

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not-really.jpg"), "stderr: {stderr}");
    assert!(sway.daemon_alive(), "vellumd died:\n{}", sway.daemon_log());

    let green = save(
        &sway,
        "green.png",
        &paint(4, 3, |_, _| GREEN),
        ImageFormat::Png,
    );
    sway.vellum_ok(["set", &green]);
    let shot = sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(160, 120).0 == GREEN);
    assert_pixel(&shot, 160, 120, GREEN, 0);
}

#[test]
fn the_daemon_keeps_no_frame_memory_after_presenting() {
    let mut sway = Sway::start(&[SMALL]);
    let pid = sway.start_daemon();
    for (i, color) in [RED, GREEN, BLUE].into_iter().enumerate() {
        let path = save(
            &sway,
            &format!("{i}.png"),
            &paint(64, 48, |_, _| color),
            ImageFormat::Png,
        );
        sway.vellum_ok(["set", &path]);
        sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(160, 120).0 == color);
    }

    // The memfd is closed right after the commit and never mapped.
    let fds: Vec<String> = std::fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .filter_map(|e| std::fs::read_link(e.ok()?.path()).ok())
        .map(|target| target.to_string_lossy().into_owned())
        .collect();
    assert!(
        fds.iter().all(|t| !t.contains("memfd:")),
        "vellumd still holds a frame: {fds:?}"
    );
    let maps = std::fs::read_to_string(format!("/proc/{pid}/maps")).unwrap();
    assert!(!maps.contains("memfd:"), "vellumd mapped a frame:\n{maps}");
}
