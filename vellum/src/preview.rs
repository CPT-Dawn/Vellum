use std::collections::{HashMap, VecDeque};
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
    pub monitor_name: String,
    pub monitor_width: u16,
    pub monitor_height: u16,
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

const SOURCE_CACHE_LIMIT: usize = 8;
const RENDER_CACHE_LIMIT: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PreviewRenderKey {
    path: PathBuf,
    scaling_id: u8,
    target_width: u16,
    target_height_rows: u16,
    monitor_name: String,
    monitor_width: u16,
    monitor_height: u16,
}

impl PreviewRenderKey {
    fn from_request(request: &PreviewRequest) -> Self {
        Self {
            path: request.path.clone(),
            scaling_id: scaling_id(request.scaling),
            target_width: request.target_width,
            target_height_rows: request.target_height_rows,
            monitor_name: request.monitor_name.clone(),
            monitor_width: request.monitor_width,
            monitor_height: request.monitor_height,
        }
    }
}

pub fn spawn_preview_worker(
    request_rx: Receiver<PreviewRequest>,
    result_tx: Sender<PreviewResult>,
) {
    thread::spawn(move || {
        let mut source_cache: HashMap<PathBuf, RgbImage> = HashMap::new();
        let mut source_lru: VecDeque<PathBuf> = VecDeque::new();
        let mut render_cache: HashMap<PreviewRenderKey, PreviewImage> = HashMap::new();
        let mut render_lru: VecDeque<PreviewRenderKey> = VecDeque::new();

        while let Ok(mut request) = request_rx.recv() {
            while let Ok(latest) = request_rx.try_recv() {
                request = latest;
            }

            let render_key = PreviewRenderKey::from_request(&request);
            let image = if let Some(cached) = render_cache.get(&render_key).cloned() {
                touch_lru(&mut render_lru, &render_key);
                Ok(cached)
            } else {
                let rendered = build_preview_image(&request, &mut source_cache, &mut source_lru);
                if let Ok(ref preview_image) = rendered {
                    insert_lru(
                        &mut render_cache,
                        &mut render_lru,
                        render_key,
                        preview_image.clone(),
                        RENDER_CACHE_LIMIT,
                    );
                }
                rendered
            };

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

fn build_preview_image(
    request: &PreviewRequest,
    source_cache: &mut HashMap<PathBuf, RgbImage>,
    source_lru: &mut VecDeque<PathBuf>,
) -> Result<PreviewImage, String> {
    if request.target_width < 1 || request.target_height_rows < 1 {
        return Err("preview area too small".to_string());
    }

    let target_width = request.target_width as u32;
    let target_height_px = request.target_height_rows.saturating_mul(2) as u32;

    if !source_cache.contains_key(&request.path) {
        let source = ImageReader::open(&request.path)
            .map_err(|error| format!("failed to open image: {error}"))?
            .decode()
            .map_err(|error| format!("failed to decode image: {error}"))?
            .to_rgb8();

        insert_lru(
            source_cache,
            source_lru,
            request.path.clone(),
            source,
            SOURCE_CACHE_LIMIT,
        );
    }

    touch_lru(source_lru, &request.path);
    trim_lru(source_cache, source_lru, SOURCE_CACHE_LIMIT);

    let source = source_cache
        .get(&request.path)
        .ok_or_else(|| "preview source cache miss".to_string())?;

    let rendered = match request.scaling {
        ScalingMode::Fit => render_fit(source, target_width, target_height_px),
        ScalingMode::Fill => render_stretch(source, target_width, target_height_px),
        ScalingMode::Crop => render_crop_center(source, target_width, target_height_px),
        ScalingMode::Center => render_center(source, target_width, target_height_px),
        ScalingMode::Tile => render_stretch(source, target_width, target_height_px),
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
        (0, 0),
        (offset_x, offset_y),
        (resized.width(), resized.height()),
    );

    canvas
}

fn render_stretch(source: &RgbImage, target_width: u32, target_height: u32) -> RgbImage {
    DynamicImage::ImageRgb8(source.clone())
        .resize_exact(target_width, target_height, FilterType::Lanczos3)
        .to_rgb8()
}

fn render_crop_center(source: &RgbImage, target_width: u32, target_height: u32) -> RgbImage {
    DynamicImage::ImageRgb8(source.clone())
        .resize_to_fill(target_width, target_height, FilterType::Lanczos3)
        .to_rgb8()
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
        (src_start_x, src_start_y),
        (dst_start_x, dst_start_y),
        (copy_width, copy_height),
    );

    canvas
}

fn blit(
    source: &RgbImage,
    destination: &mut RgbImage,
    src_origin: (u32, u32),
    dst_origin: (u32, u32),
    size: (u32, u32),
) {
    let (src_x, src_y) = src_origin;
    let (dst_x, dst_y) = dst_origin;
    let (width, height) = size;

    for row in 0..height {
        for col in 0..width {
            let pixel = source.get_pixel(src_x + col, src_y + row);
            destination.put_pixel(dst_x + col, dst_y + row, *pixel);
        }
    }
}

fn scaling_id(mode: ScalingMode) -> u8 {
    match mode {
        ScalingMode::Fill => 0,
        ScalingMode::Fit => 1,
        ScalingMode::Crop => 2,
        ScalingMode::Center => 3,
        ScalingMode::Tile => 4,
    }
}

fn touch_lru<K>(order: &mut VecDeque<K>, key: &K)
where
    K: Clone + PartialEq,
{
    if let Some(position) = order.iter().position(|existing| existing == key) {
        let _ = order.remove(position);
    }
    order.push_back(key.clone());
}

fn trim_lru<K, V>(map: &mut HashMap<K, V>, order: &mut VecDeque<K>, limit: usize)
where
    K: Eq + std::hash::Hash + Clone,
{
    while map.len() > limit {
        if let Some(oldest_key) = order.pop_front() {
            map.remove(&oldest_key);
        } else {
            break;
        }
    }
}

fn insert_lru<K, V>(
    map: &mut HashMap<K, V>,
    order: &mut VecDeque<K>,
    key: K,
    value: V,
    limit: usize,
) where
    K: Eq + std::hash::Hash + Clone + PartialEq,
{
    map.insert(key.clone(), value);
    touch_lru(order, &key);
    trim_lru(map, order, limit);
}
