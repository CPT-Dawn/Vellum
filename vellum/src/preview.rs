use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use image::imageops::FilterType;
use image::{DynamicImage, ImageBuffer, ImageReader, Rgb, RgbImage};

use crate::app::ScalingMode;

#[derive(Debug, Clone)]
pub struct PreviewRequest {
    pub seq: u64,
    pub path: PathBuf,
    pub scaling: ScalingMode,
    pub target_width: u16,
    pub target_height_rows: u16,
}

#[derive(Debug, Clone)]
pub struct PreviewResult {
    pub seq: u64,
    pub image: Result<PreviewImage, String>,
}

#[derive(Debug, Clone)]
pub struct PreviewImage {
    pub width: u16,
    pub height_px: u16,
    pub pixels_rgb: Vec<u8>,
}

pub fn spawn_preview_worker(
    request_rx: Receiver<PreviewRequest>,
    result_tx: Sender<PreviewResult>,
) {
    thread::spawn(move || {
        while let Ok(mut request) = request_rx.recv() {
            while let Ok(latest) = request_rx.try_recv() {
                request = latest;
            }

            let image = build_preview_image(&request);
            if result_tx
                .send(PreviewResult {
                    seq: request.seq,
                    image,
                })
                .is_err()
            {
                break;
            }
        }
    });
}

fn build_preview_image(request: &PreviewRequest) -> Result<PreviewImage, String> {
    if request.target_width < 1 || request.target_height_rows < 1 {
        return Err("preview area too small".to_string());
    }

    let target_width = request.target_width as u32;
    let target_height_px = request.target_height_rows.saturating_mul(2) as u32;

    let source = ImageReader::open(&request.path)
        .map_err(|error| format!("failed to open image: {error}"))?
        .decode()
        .map_err(|error| format!("failed to decode image: {error}"))?
        .to_rgb8();

    let rendered = match request.scaling {
        ScalingMode::Fit => render_fit(&source, target_width, target_height_px),
        ScalingMode::Fill => render_fill(&source, target_width, target_height_px),
        ScalingMode::Crop => render_crop_top_left(&source, target_width, target_height_px),
        ScalingMode::Center => render_center(&source, target_width, target_height_px),
        ScalingMode::Tile => render_tile(&source, target_width, target_height_px),
    };

    Ok(PreviewImage {
        width: rendered.width() as u16,
        height_px: rendered.height() as u16,
        pixels_rgb: rendered.into_raw(),
    })
}

fn blank_canvas(width: u32, height: u32) -> RgbImage {
    ImageBuffer::from_pixel(width, height, Rgb([0, 0, 0]))
}

fn render_fit(source: &RgbImage, target_width: u32, target_height: u32) -> RgbImage {
    let resized = DynamicImage::ImageRgb8(source.clone())
        .resize(target_width, target_height, FilterType::Lanczos3)
        .to_rgb8();

    let mut canvas = blank_canvas(target_width, target_height);
    let offset_x = (target_width.saturating_sub(resized.width())) / 2;
    let offset_y = (target_height.saturating_sub(resized.height())) / 2;

    blit(
        &resized,
        &mut canvas,
        0,
        0,
        offset_x,
        offset_y,
        resized.width(),
        resized.height(),
    );

    canvas
}

fn render_fill(source: &RgbImage, target_width: u32, target_height: u32) -> RgbImage {
    DynamicImage::ImageRgb8(source.clone())
        .resize_to_fill(target_width, target_height, FilterType::Lanczos3)
        .to_rgb8()
}

fn render_crop_top_left(source: &RgbImage, target_width: u32, target_height: u32) -> RgbImage {
    let mut canvas = blank_canvas(target_width, target_height);
    let copy_width = source.width().min(target_width);
    let copy_height = source.height().min(target_height);

    blit(source, &mut canvas, 0, 0, 0, 0, copy_width, copy_height);

    canvas
}

fn render_center(source: &RgbImage, target_width: u32, target_height: u32) -> RgbImage {
    let mut canvas = blank_canvas(target_width, target_height);

    let src_start_x = if source.width() > target_width {
        (source.width() - target_width) / 2
    } else {
        0
    };
    let src_start_y = if source.height() > target_height {
        (source.height() - target_height) / 2
    } else {
        0
    };

    let dst_start_x = if target_width > source.width() {
        (target_width - source.width()) / 2
    } else {
        0
    };
    let dst_start_y = if target_height > source.height() {
        (target_height - source.height()) / 2
    } else {
        0
    };

    let copy_width = source.width().min(target_width);
    let copy_height = source.height().min(target_height);

    blit(
        source,
        &mut canvas,
        src_start_x,
        src_start_y,
        dst_start_x,
        dst_start_y,
        copy_width,
        copy_height,
    );

    canvas
}

fn render_tile(source: &RgbImage, target_width: u32, target_height: u32) -> RgbImage {
    let mut canvas = blank_canvas(target_width, target_height);
    let source_width = source.width().max(1);
    let source_height = source.height().max(1);

    for y in 0..target_height {
        for x in 0..target_width {
            let sample = source.get_pixel(x % source_width, y % source_height);
            canvas.put_pixel(x, y, *sample);
        }
    }

    canvas
}

fn blit(
    source: &RgbImage,
    destination: &mut RgbImage,
    src_x: u32,
    src_y: u32,
    dst_x: u32,
    dst_y: u32,
    width: u32,
    height: u32,
) {
    for row in 0..height {
        for col in 0..width {
            let pixel = source.get_pixel(src_x + col, src_y + row);
            destination.put_pixel(dst_x + col, dst_y + row, *pixel);
        }
    }
}
