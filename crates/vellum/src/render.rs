//! The renderer: a wallpaper image in, a Monitor-sized frame out.

use fast_image_resize::images::{Image, ImageRef};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::RgbaImage;

/// Renders `source` with Fit mode Fill (cover the frame, crop the overflow
/// evenly from both sides) into `out`, a `width`×`height` XRGB8888 frame
/// with a stride of `width * 4`.
pub fn render_fill(source: &RgbaImage, width: u32, height: u32, out: &mut [u8]) {
    let len = width as usize * height as usize * 4;
    let out = &mut out[..len];
    let src = ImageRef::new(
        source.width(),
        source.height(),
        source.as_raw(),
        PixelType::U8x4,
    )
    .expect("an RgbaImage is a valid U8x4 buffer");
    let mut dst = Image::from_slice_u8(width, height, out, PixelType::U8x4)
        .expect("the frame holds width * height pixels");
    let options = ResizeOptions::new()
        .resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3))
        .fit_into_destination(None)
        .use_alpha(false);
    Resizer::new()
        .resize(&src, &mut dst, &options)
        .expect("source and frame share a pixel type");
    drop(dst);
    rgba_to_xrgb(out);
}

/// Converts RGBA bytes in place to XRGB8888 (bytes B, G, R, X).
fn rgba_to_xrgb(pixels: &mut [u8]) {
    for px in pixels.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
        px[3] = 0xff;
    }
}
