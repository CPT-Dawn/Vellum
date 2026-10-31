//! The renderer: a wallpaper image in, a Monitor-sized frame out.
//!
//! [`render`] is the one pure render function. `vellum set`, `vellum render`
//! and the TUI preview all call it, so the preview always matches the result.

use fast_image_resize::images::{Image, ImageRef};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::RgbaImage;

#[cfg(test)]
mod tests;

/// How a wallpaper is sized to a Monitor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum FitMode {
    /// Cover the Monitor and crop the overflow evenly from both sides.
    #[default]
    Fill,
    /// Show the whole wallpaper, letterboxed with the letterbox color.
    Fit,
    /// Actual size (one wallpaper pixel per logical pixel), centered.
    Center,
    /// Scale each axis independently to the Monitor's size.
    Stretch,
}

/// An opaque sRGB color, `[r, g, b]`.
pub type Color = [u8; 3];

/// The letterbox color used until Config overrides it.
pub const DEFAULT_LETTERBOX: Color = [0, 0, 0];

/// What to render for: a Monitor (or a TUI preview of one).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    /// Frame size in physical pixels.
    pub width: u32,
    pub height: u32,
    /// Physical pixels per logical pixel (1.5 for a 1.5x Monitor). Only
    /// Center depends on it. A TUI preview passes the Monitor's scale times
    /// the preview's shrink factor, so Center previews true to the Monitor.
    pub scale: f64,
}

impl Target {
    /// A target from a scale in 120ths, as fractional-scale-v1 and
    /// `MonitorInfo::scale_120` carry it.
    pub fn from_scale_120(width: u32, height: u32, scale_120: u32) -> Target {
        Target {
            width,
            height,
            scale: f64::from(scale_120.max(1)) / 120.0,
        }
    }
}

/// Renders `source` into `out`, a `target.width`×`target.height` XRGB8888
/// frame (bytes B, G, R, X) with a stride of `width * 4`.
///
/// Pure: the output depends only on the arguments. Areas the wallpaper does
/// not cover, and transparent pixels, show `letterbox`.
///
/// # Panics
/// If `out` is shorter than `width * height * 4` bytes.
pub fn render(source: &RgbaImage, target: Target, fit: FitMode, letterbox: Color, out: &mut [u8]) {
    let (width, height) = (target.width, target.height);
    let len = width as usize * height as usize * 4;
    assert!(
        out.len() >= len,
        "frame buffer too small for {width}x{height}"
    );
    let out = &mut out[..len];
    if len == 0 {
        return;
    }
    if source.width() == 0 || source.height() == 0 {
        fill(out, letterbox);
        return;
    }

    let place = Placement::new(source.width(), source.height(), target, fit);
    let opaque = source.pixels().all(|p| p[3] == u8::MAX);

    if place.covers(width, height) {
        place.draw(source, opaque, out);
        to_xrgb(out, letterbox, opaque);
        return;
    }

    fill(out, letterbox);
    let mut tile = vec![0; place.w as usize * place.h as usize * 4];
    place.draw(source, opaque, &mut tile);
    to_xrgb(&mut tile, letterbox, opaque);
    let row = place.w as usize * 4;
    let stride = width as usize * 4;
    for (y, src) in tile.chunks_exact(row).enumerate() {
        let start = (place.y as usize + y) * stride + place.x as usize * 4;
        out[start..start + row].copy_from_slice(src);
    }
}

/// Where the visible part of the wallpaper lands, and which part of the
/// source it comes from.
#[derive(Debug)]
struct Placement {
    /// Visible rectangle in the frame, in physical pixels.
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    /// The source rectangle shown there, in source pixels.
    crop: [f64; 4],
    /// Whether one source pixel maps onto exactly one frame pixel.
    one_to_one: bool,
}

impl Placement {
    fn new(sw: u32, sh: u32, target: Target, fit: FitMode) -> Placement {
        let (fw, fh) = (target.width, target.height);
        let (sw_f, sh_f, fw_f, fh_f) = (sw as f64, sh as f64, fw as f64, fh as f64);
        // The size the whole wallpaper is drawn at, in physical pixels.
        let (dw, dh) = match fit {
            FitMode::Stretch | FitMode::Fill => (fw, fh),
            FitMode::Fit => {
                let s = (fw_f / sw_f).min(fh_f / sh_f);
                (round_px(sw_f * s).min(fw), round_px(sh_f * s).min(fh))
            }
            FitMode::Center => {
                let s = if target.scale.is_finite() && target.scale > 0.0 {
                    target.scale
                } else {
                    1.0
                };
                (round_px(sw_f * s), round_px(sh_f * s))
            }
        };
        if fit == FitMode::Fill {
            // Cover: crop the source to the frame's aspect ratio, centered.
            let s = (fw_f / sw_f).max(fh_f / sh_f);
            let (cw, ch) = ((fw_f / s).min(sw_f), (fh_f / s).min(sh_f));
            return Placement {
                x: 0,
                y: 0,
                w: fw,
                h: fh,
                crop: [(sw_f - cw) / 2.0, (sh_f - ch) / 2.0, cw, ch],
                one_to_one: sw == fw && sh == fh,
            };
        }
        let (x, w, left, cw) = axis(fw, dw, sw);
        let (y, h, top, ch) = axis(fh, dh, sh);
        Placement {
            x,
            y,
            w,
            h,
            crop: [left, top, cw, ch],
            one_to_one: dw == sw && dh == sh,
        }
    }

    fn covers(&self, width: u32, height: u32) -> bool {
        self.x == 0 && self.y == 0 && self.w == width && self.h == height
    }

    /// Draws the visible part as RGBA into `dst` (`w`×`h` pixels).
    fn draw(&self, source: &RgbaImage, opaque: bool, dst: &mut [u8]) {
        if self.one_to_one {
            // No resampling: copy the visible source pixels as they are.
            let [left, top, ..] = self.crop.map(|v| v as usize);
            let src_stride = source.width() as usize * 4;
            let row = self.w as usize * 4;
            for (y, out) in dst.chunks_exact_mut(row).enumerate() {
                let start = (top + y) * src_stride + left * 4;
                out.copy_from_slice(&source.as_raw()[start..start + row]);
            }
            return;
        }
        let src = ImageRef::new(
            source.width(),
            source.height(),
            source.as_raw(),
            PixelType::U8x4,
        )
        .expect("an RgbaImage is a valid U8x4 buffer");
        let mut dst = Image::from_slice_u8(self.w, self.h, dst, PixelType::U8x4)
            .expect("the buffer holds w * h pixels");
        let [left, top, cw, ch] = self.crop;
        let options = ResizeOptions::new()
            .resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3))
            .crop(left, top, cw, ch)
            .use_alpha(!opaque);
        Resizer::new()
            .resize(&src, &mut dst, &options)
            .expect("source and frame share a pixel type");
    }
}

/// One axis of a centered placement: the frame is `frame` pixels long, the
/// wallpaper is drawn `drawn` pixels long from a `source`-pixel axis.
/// Returns the visible start and length in the frame, and the matching
/// source start and length.
fn axis(frame: u32, drawn: u32, source: u32) -> (u32, u32, f64, f64) {
    let start = (i64::from(frame) - i64::from(drawn)).div_euclid(2);
    let visible_start = start.max(0);
    let visible_end = (start + i64::from(drawn)).min(i64::from(frame));
    let per_px = f64::from(source) / f64::from(drawn);
    (
        visible_start as u32,
        (visible_end - visible_start) as u32,
        (visible_start - start) as f64 * per_px,
        (visible_end - visible_start) as f64 * per_px,
    )
}

fn round_px(v: f64) -> u32 {
    (v.round() as u32).max(1)
}

fn fill(out: &mut [u8], [r, g, b]: Color) {
    for px in out.as_chunks_mut::<4>().0 {
        *px = [b, g, r, u8::MAX];
    }
}

/// Converts RGBA bytes in place to XRGB8888 (bytes B, G, R, X), compositing
/// translucent pixels over `background`.
fn to_xrgb(pixels: &mut [u8], background: Color, opaque: bool) {
    let px = pixels.as_chunks_mut::<4>().0;
    if opaque {
        for p in px {
            *p = [p[2], p[1], p[0], u8::MAX];
        }
        return;
    }
    let blend =
        |c: u8, bg: u8, a: u16| ((u16::from(c) * a + u16::from(bg) * (255 - a) + 127) / 255) as u8;
    for p in px {
        let a = u16::from(p[3]);
        *p = [
            blend(p[2], background[2], a),
            blend(p[1], background[1], a),
            blend(p[0], background[0], a),
            u8::MAX,
        ];
    }
}
