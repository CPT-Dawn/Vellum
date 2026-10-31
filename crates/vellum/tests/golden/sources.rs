//! Source wallpapers for the golden images, shared by the render unit tests
//! (`src/render/tests.rs`) and the end-to-end Fit mode tests (`tests/fit.rs`).
//! Changing one means re-blessing: `VELLUM_BLESS=1 cargo test --workspace`.

#![allow(dead_code)]

use image::{Rgba, RgbaImage};

/// A recognisable test card: a red-to-white gradient across, green down,
/// with a blue checkerboard, so crops, offsets and flips all show.
pub fn card(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        let r = (x * 255 / (width - 1).max(1)) as u8;
        let g = (y * 255 / (height - 1).max(1)) as u8;
        let b = if (x / 8 + y / 8) % 2 == 0 { 230 } else { 30 };
        Rgba([r, g, b, 255])
    })
}

/// Larger than the golden target in both axes.
pub fn landscape() -> RgbaImage {
    card(160, 90)
}

/// Taller but narrower than the golden target.
pub fn portrait() -> RgbaImage {
    card(30, 80)
}

/// Smaller than the golden target.
pub fn square() -> RgbaImage {
    card(32, 32)
}

/// The wallpaper the end-to-end tests set on a 320x240 Monitor.
pub fn e2e() -> RgbaImage {
    card(480, 200)
}
