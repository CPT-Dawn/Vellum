//! The render pipeline as a pure function: behaviour checks with flat colors,
//! then golden images for every Fit mode across sources, scales and
//! letterbox colors.

use std::path::{Path, PathBuf};

use image::{Rgb, RgbImage, Rgba, RgbaImage};

use super::*;

#[path = "../../tests/golden/sources.rs"]
mod sources;

const RED: Color = [255, 0, 0];
const GREEN: Color = [0, 255, 0];
const BLUE: Color = [0, 0, 255];
const WHITE: Color = [255, 255, 255];
const NAVY: Color = [0x1a, 0x1b, 0x2e];

fn target(width: u32, height: u32) -> Target {
    Target {
        width,
        height,
        scale: 1.0,
    }
}

/// Renders and decodes the XRGB frame back into RGB for inspection.
fn shot(source: &RgbaImage, target: Target, fit: FitMode, letterbox: Color) -> RgbImage {
    let mut frame = vec![0xAA; target.width as usize * target.height as usize * 4];
    render(source, target, fit, letterbox, &mut frame);
    RgbImage::from_fn(target.width, target.height, |x, y| {
        let i = (y * target.width + x) as usize * 4;
        assert_eq!(frame[i + 3], 0xff, "X byte at {x},{y}");
        Rgb([frame[i + 2], frame[i + 1], frame[i]])
    })
}

fn paint(width: u32, height: u32, color_at: impl Fn(u32, u32) -> Color) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        let [r, g, b] = color_at(x, y);
        Rgba([r, g, b, 255])
    })
}

#[track_caller]
fn assert_px(img: &RgbImage, x: u32, y: u32, expected: Color) {
    let got = img.get_pixel(x, y).0;
    let close = got.iter().zip(expected).all(|(&g, e)| g.abs_diff(e) <= 2);
    assert!(close, "pixel {x},{y}: got {got:?}, want {expected:?}");
}

#[test]
fn fill_is_the_default_fit_mode() {
    assert_eq!(FitMode::default(), FitMode::Fill);
}

#[test]
fn fill_covers_the_frame_and_crops_the_overflow_evenly() {
    // Thirds across; a 1:1 frame shows only the middle third.
    let wide = paint(300, 100, |x, _| [RED, GREEN, BLUE][(x / 100) as usize]);
    let img = shot(&wide, target(50, 50), FitMode::Fill, NAVY);
    for x in [2, 25, 47] {
        assert_px(&img, x, 2, GREEN);
        assert_px(&img, x, 47, GREEN);
    }
}

#[test]
fn fit_letterboxes_a_wide_wallpaper_above_and_below() {
    // 2:1 into 40x40: a 40x20 band in the middle, bars of 10 above and below.
    let wide = paint(200, 100, |x, _| if x < 100 { RED } else { BLUE });
    let img = shot(&wide, target(40, 40), FitMode::Fit, NAVY);
    for x in [0, 20, 39] {
        assert_px(&img, x, 0, NAVY);
        assert_px(&img, x, 9, NAVY);
        assert_px(&img, x, 30, NAVY);
        assert_px(&img, x, 39, NAVY);
    }
    assert_px(&img, 5, 20, RED);
    assert_px(&img, 34, 20, BLUE);
}

#[test]
fn fit_letterboxes_a_tall_wallpaper_left_and_right() {
    let tall = paint(50, 100, |_, y| if y < 50 { GREEN } else { WHITE });
    let img = shot(&tall, target(60, 40), FitMode::Fit, DEFAULT_LETTERBOX);
    // 20x40 in the middle: bars 20 wide on each side.
    for y in [0, 20, 39] {
        assert_px(&img, 0, y, [0, 0, 0]);
        assert_px(&img, 19, y, [0, 0, 0]);
        assert_px(&img, 40, y, [0, 0, 0]);
        assert_px(&img, 59, y, [0, 0, 0]);
    }
    assert_px(&img, 30, 5, GREEN);
    assert_px(&img, 30, 34, WHITE);
}

#[test]
fn center_at_scale_one_is_pixel_exact_and_cropped_evenly() {
    let card = sources::card(100, 60);
    let img = shot(&card, target(40, 20), FitMode::Center, NAVY);
    for (x, y) in [(0, 0), (39, 19), (17, 11)] {
        let [r, g, b, _] = card.get_pixel(30 + x, 20 + y).0;
        assert_eq!(img.get_pixel(x, y).0, [r, g, b], "at {x},{y}");
    }
}

#[test]
fn center_draws_one_wallpaper_pixel_per_logical_pixel() {
    // 10x10 at scale 2 is 20x20 physical, centered in 40x30 with bars.
    let halves = paint(10, 10, |x, _| if x < 5 { RED } else { BLUE });
    let t = Target {
        width: 40,
        height: 30,
        scale: 2.0,
    };
    let img = shot(&halves, t, FitMode::Center, NAVY);
    assert_px(&img, 9, 15, NAVY);
    assert_px(&img, 30, 15, NAVY);
    assert_px(&img, 20, 4, NAVY);
    assert_px(&img, 20, 25, NAVY);
    assert_px(&img, 12, 15, RED);
    assert_px(&img, 27, 15, BLUE);
}

#[test]
fn stretch_maps_the_whole_wallpaper_onto_the_whole_frame() {
    let quads = paint(100, 100, |x, y| match (x < 50, y < 50) {
        (true, true) => RED,
        (false, true) => GREEN,
        (true, false) => BLUE,
        (false, false) => WHITE,
    });
    let img = shot(&quads, target(80, 20), FitMode::Stretch, NAVY);
    assert_px(&img, 1, 1, RED);
    assert_px(&img, 78, 1, GREEN);
    assert_px(&img, 1, 18, BLUE);
    assert_px(&img, 78, 18, WHITE);
}

#[test]
fn transparent_pixels_show_the_letterbox_color() {
    let ghost = RgbaImage::from_fn(20, 20, |x, _| {
        if x < 10 {
            Rgba([255, 255, 255, 0])
        } else {
            Rgba([255, 0, 0, 255])
        }
    });
    let img = shot(&ghost, target(20, 20), FitMode::Fill, NAVY);
    assert_px(&img, 3, 10, NAVY);
    assert_px(&img, 16, 10, RED);
}

#[test]
fn the_scale_comes_from_120ths() {
    assert_eq!(Target::from_scale_120(30, 20, 150).scale, 1.25);
    assert_eq!(Target::from_scale_120(30, 20, 120).scale, 1.0);
}

// ---- Golden images ----

/// Logical size of the golden target: 16:10, unlike any source.
const LOGICAL: (u32, u32) = (64, 40);
const SCALES: [f64; 4] = [1.0, 1.25, 1.5, 2.0];
const FITS: [FitMode; 4] = [
    FitMode::Fill,
    FitMode::Fit,
    FitMode::Center,
    FitMode::Stretch,
];

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn fit_name(fit: FitMode) -> &'static str {
    match fit {
        FitMode::Fill => "fill",
        FitMode::Fit => "fit",
        FitMode::Center => "center",
        FitMode::Stretch => "stretch",
    }
}

/// Every golden case: (file name, source, target, Fit mode, letterbox).
fn golden_cases() -> Vec<(String, RgbaImage, Target, FitMode, Color)> {
    let mut cases = Vec::new();
    let sources = [
        ("landscape", sources::landscape()),
        ("portrait", sources::portrait()),
        ("square", sources::square()),
    ];
    for (source_name, source) in &sources {
        for scale in SCALES {
            let t = Target {
                width: (LOGICAL.0 as f64 * scale).round() as u32,
                height: (LOGICAL.1 as f64 * scale).round() as u32,
                scale,
            };
            for fit in FITS {
                let name = format!("{source_name}-{}-x{:03}", fit_name(fit), scale * 100.0);
                cases.push((name, source.clone(), t, fit, DEFAULT_LETTERBOX));
            }
            for fit in [FitMode::Fit, FitMode::Center] {
                let name = format!("{source_name}-{}-x{:03}-navy", fit_name(fit), scale * 100.0);
                cases.push((name, source.clone(), t, fit, NAVY));
            }
        }
    }
    // The end-to-end Fit mode tests compare screenshots against these.
    for fit in FITS {
        let name = format!("e2e-{}", fit_name(fit));
        cases.push((
            name,
            sources::e2e(),
            target(320, 240),
            fit,
            DEFAULT_LETTERBOX,
        ));
    }
    cases
}

/// Renders every golden case and compares it with `tests/golden/<name>.png`.
/// `VELLUM_BLESS=1` rewrites the files instead; inspect the diff by eye.
#[test]
fn renders_match_the_golden_images() {
    let bless = std::env::var_os("VELLUM_BLESS").is_some();
    let mut failures = Vec::new();
    for (name, source, t, fit, letterbox) in golden_cases() {
        let img = shot(&source, t, fit, letterbox);
        let path = golden_dir().join(format!("{name}.png"));
        if bless {
            img.save(&path).unwrap();
            continue;
        }
        let golden = match image::open(&path) {
            Ok(golden) => golden.into_rgb8(),
            Err(err) => {
                failures.push(format!("{name}: {err} (run with VELLUM_BLESS=1)"));
                continue;
            }
        };
        if golden.dimensions() != img.dimensions() {
            failures.push(format!(
                "{name}: size {:?}, golden {:?}",
                img.dimensions(),
                golden.dimensions()
            ));
            continue;
        }
        // Allow tiny rounding differences between SIMD code paths.
        let worst = img
            .as_raw()
            .iter()
            .zip(golden.as_raw())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        if worst > 2 {
            failures.push(format!("{name}: differs by up to {worst}"));
        }
    }
    assert!(
        failures.is_empty(),
        "golden mismatches:\n{}",
        failures.join("\n")
    );
}
