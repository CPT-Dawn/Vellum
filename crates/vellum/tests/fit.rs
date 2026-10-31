//! `vellum set --fit` end to end: each Fit mode on a headless Monitor must
//! match the golden image the render unit tests lock in.

mod support;

#[path = "golden/sources.rs"]
mod sources;

use image::RgbImage;
use support::*;

fn golden(name: &str) -> RgbImage {
    let path = format!("{}/tests/golden/{name}.png", env!("CARGO_MANIFEST_DIR"));
    image::open(&path)
        .unwrap_or_else(|err| panic!("{path}: {err}"))
        .into_rgb8()
}

fn worst_diff(a: &RgbImage, b: &RgbImage) -> u8 {
    if a.dimensions() != b.dimensions() {
        return u8::MAX;
    }
    a.as_raw()
        .iter()
        .zip(b.as_raw())
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

#[test]
fn every_fit_mode_matches_its_golden_image() {
    let mut sway = Sway::start(&[Monitor {
        width: 320,
        height: 240,
    }]);
    sway.start_daemon();
    let path = sway.dir().join("card.png");
    sources::e2e().save(&path).unwrap();
    let path = path.to_string_lossy().into_owned();

    for fit in ["fill", "fit", "center", "stretch"] {
        let expected = golden(&format!("e2e-{fit}"));
        sway.vellum_ok(["set", &path, "--fit", fit]);
        let shot = sway.screenshot_until("HEADLESS-1", |s| worst_diff(s, &expected) == 0);
        assert_eq!(worst_diff(&shot, &expected), 0, "--fit {fit}");
    }
}

#[test]
fn fill_is_the_default_and_fit_letterboxes_in_black() {
    let mut sway = Sway::start(&[Monitor {
        width: 320,
        height: 240,
    }]);
    sway.start_daemon();
    // Wide and flat: Fit leaves bars above and below.
    let path = sway.dir().join("green.png");
    RgbImage::from_pixel(64, 16, image::Rgb(GREEN))
        .save(&path)
        .unwrap();
    let path = path.to_string_lossy().into_owned();

    sway.vellum_ok(["set", &path]);
    let shot = sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(160, 5).0 == GREEN);
    assert_pixel(&shot, 160, 5, GREEN, 0);

    sway.vellum_ok(["set", &path, "--fit", "fit"]);
    let shot = sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(160, 5).0 == [0, 0, 0]);
    assert_pixel(&shot, 160, 5, [0, 0, 0], 0);
    assert_pixel(&shot, 160, 235, [0, 0, 0], 0);
    assert_pixel(&shot, 160, 120, GREEN, 0);
}

#[test]
fn an_unknown_fit_mode_is_rejected_with_the_choices() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_vellum"))
        .args(["set", "whatever.png", "--fit", "zoom"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    for choice in ["fill", "fit", "center", "stretch"] {
        assert!(stderr.contains(choice), "stderr: {stderr}");
    }
}
