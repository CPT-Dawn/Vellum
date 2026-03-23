use anyhow::{Context, Result};
use image::imageops::{crop_imm, resize, FilterType};
use image::{ImageBuffer, ImageReader, Rgba, RgbaImage};
use std::path::Path;
use vellum_ipc::ScaleMode;

#[derive(Debug, Clone)]
pub(crate) struct NativeFrame {
    pub(crate) stride: usize,
    pub(crate) pixels: Vec<u8>,
}

impl NativeFrame {
    pub(crate) fn solid_black(width: u32, height: u32) -> Self {
        let stride = width as usize * 4;
        let pixels = vec![0; stride.saturating_mul(height as usize)];
        Self { stride, pixels }
    }
}

pub(crate) fn render_frame(
    path: &Path,
    target_width: u32,
    target_height: u32,
    mode: ScaleMode,
) -> Result<NativeFrame> {
    let target_width = target_width.max(1);
    let target_height = target_height.max(1);

    let source = ImageReader::open(path)
        .with_context(|| format!("failed to open image {}", path.display()))?
        .decode()
        .with_context(|| format!("failed to decode image {}", path.display()))?
        .to_rgba8();

    let rendered = match mode {
        ScaleMode::Fill => render_fill(&source, target_width, target_height),
        ScaleMode::Fit => render_fit(&source, target_width, target_height),
        ScaleMode::Crop => render_crop(&source, target_width, target_height),
    };

    let stride = target_width as usize * 4;
    Ok(NativeFrame {
        stride,
        pixels: rendered.into_raw(),
    })
}

fn render_fill(source: &RgbaImage, target_width: u32, target_height: u32) -> RgbaImage {
    resize(source, target_width, target_height, FilterType::Lanczos3)
}

fn render_fit(source: &RgbaImage, target_width: u32, target_height: u32) -> RgbaImage {
    let src_w = source.width().max(1) as f32;
    let src_h = source.height().max(1) as f32;
    let dst_w = target_width as f32;
    let dst_h = target_height as f32;

    let scale = (dst_w / src_w).min(dst_h / src_h);
    let scaled_w = ((src_w * scale).round() as u32).clamp(1, target_width);
    let scaled_h = ((src_h * scale).round() as u32).clamp(1, target_height);

    let resized = resize(source, scaled_w, scaled_h, FilterType::Lanczos3);
    let mut canvas: RgbaImage =
        ImageBuffer::from_pixel(target_width, target_height, Rgba([0, 0, 0, 255]));

    let offset_x = (target_width.saturating_sub(scaled_w)) / 2;
    let offset_y = (target_height.saturating_sub(scaled_h)) / 2;

    for y in 0..scaled_h {
        for x in 0..scaled_w {
            let px = resized.get_pixel(x, y);
            canvas.put_pixel(offset_x + x, offset_y + y, *px);
        }
    }

    canvas
}

fn render_crop(source: &RgbaImage, target_width: u32, target_height: u32) -> RgbaImage {
    let src_w = source.width().max(1) as f32;
    let src_h = source.height().max(1) as f32;
    let dst_w = target_width as f32;
    let dst_h = target_height as f32;

    let scale = (dst_w / src_w).max(dst_h / src_h);
    let scaled_w = ((src_w * scale).round() as u32).max(target_width);
    let scaled_h = ((src_h * scale).round() as u32).max(target_height);

    let resized = resize(source, scaled_w, scaled_h, FilterType::Lanczos3);
    let offset_x = (scaled_w.saturating_sub(target_width)) / 2;
    let offset_y = (scaled_h.saturating_sub(target_height)) / 2;

    crop_imm(&resized, offset_x, offset_y, target_width, target_height).to_image()
}

#[cfg(test)]
mod tests {
    use super::render_frame;
    use std::time::{SystemTime, UNIX_EPOCH};
    use vellum_ipc::ScaleMode;

    fn new_fixture_path() -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("vellum-image-blit-{nonce}.png"))
    }

    #[test]
    fn render_frame_outputs_target_dimensions() {
        let fixture = new_fixture_path();
        let image = image::RgbImage::from_pixel(64, 32, image::Rgb([200, 100, 50]));
        image.save(&fixture).expect("fixture should be written");

        for mode in [ScaleMode::Fit, ScaleMode::Fill, ScaleMode::Crop] {
            let frame = render_frame(&fixture, 1920, 1080, mode).expect("render should succeed");
            assert_eq!(frame.stride, 1920 * 4);
            assert_eq!(frame.pixels.len(), 1920 * 1080 * 4);
        }

        let _ = std::fs::remove_file(fixture);
    }
}
