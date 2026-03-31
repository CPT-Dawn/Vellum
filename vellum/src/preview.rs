use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use common::ipc::PixelFormat;
use fast_image_resize::FilterType as FirFilterType;
use image::imageops::FilterType;
use image::{DynamicImage, RgbImage};

use crate::app::ScalingMode;
use crate::imgproc::{self, ImgBuf, ResizeStrategy};

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
                let rendered = build_preview_image(&request);
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

fn build_preview_image(request: &PreviewRequest) -> Result<PreviewImage, String> {
    if request.target_width < 1 || request.target_height_rows < 1 {
        return Err("preview area too small".to_string());
    }

    let target_width = request.target_width as u32;
    let target_height_px = request.target_height_rows.saturating_mul(2) as u32;
    let monitor_dimensions = resolved_monitor_dimensions(request, target_width, target_height_px);
    let source_render = render_like_backend(request, monitor_dimensions)?;
    let panel_render =
        if source_render.width() == target_width && source_render.height() == target_height_px {
            source_render
        } else {
            DynamicImage::ImageRgb8(source_render)
                .resize_exact(target_width, target_height_px, FilterType::Lanczos3)
                .to_rgb8()
        };

    Ok(PreviewImage {
        width: panel_render.width() as u16,
        height_px: panel_render.height() as u16,
        pixels_rgb: panel_render.into_raw(),
    })
}

fn render_like_backend(
    request: &PreviewRequest,
    dimensions: (u32, u32),
) -> Result<RgbImage, String> {
    let img_buf = ImgBuf::new(&request.path)?;
    let resize = resize_strategy_for_scaling_mode(request.scaling);
    let filter = FirFilterType::Lanczos3;

    let bytes = match img_buf.decode_prepare() {
        imgproc::DecodeBuffer::RasterImage(imgbuf) => {
            let decoded = imgbuf.decode(PixelFormat::Rgb)?;
            resize_with_strategy(&decoded, resize, dimensions, filter)?
        }
        imgproc::DecodeBuffer::VectorImage(imgbuf) => {
            let decoded = imgbuf.decode(PixelFormat::Rgb, dimensions.0, dimensions.1)?;
            resize_with_strategy(&decoded, resize, dimensions, filter)?
        }
    };

    RgbImage::from_raw(dimensions.0, dimensions.1, bytes.into_vec())
        .ok_or_else(|| "failed to build preview RGB image from resized bytes".to_string())
}

fn resize_with_strategy(
    image: &imgproc::Image,
    resize: ResizeStrategy,
    dimensions: (u32, u32),
    filter: FirFilterType,
) -> Result<Box<[u8]>, String> {
    match resize {
        ResizeStrategy::No => Ok(imgproc::img_pad(image, dimensions, [0, 0, 0, 255])),
        ResizeStrategy::Crop => imgproc::img_resize_crop(image, dimensions, filter),
        ResizeStrategy::Fit => imgproc::img_resize_fit(image, dimensions, filter, [0, 0, 0, 255]),
        ResizeStrategy::Stretch => imgproc::img_resize_stretch(image, dimensions, filter),
    }
}

fn resolved_monitor_dimensions(
    request: &PreviewRequest,
    fallback_width: u32,
    fallback_height: u32,
) -> (u32, u32) {
    let width = u32::from(request.monitor_width);
    let height = u32::from(request.monitor_height);

    if width > 0 && height > 0 {
        (width, height)
    } else {
        (fallback_width.max(1), fallback_height.max(1))
    }
}

fn resize_strategy_for_scaling_mode(mode: ScalingMode) -> ResizeStrategy {
    match mode {
        ScalingMode::Fill => ResizeStrategy::Stretch,
        ScalingMode::Fit => ResizeStrategy::Fit,
        ScalingMode::Crop => ResizeStrategy::Crop,
        ScalingMode::Center => ResizeStrategy::No,
        ScalingMode::Tile => ResizeStrategy::Stretch,
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
