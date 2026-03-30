#[path = "shared_imgproc.rs"]
mod shared_imgproc;

pub use shared_imgproc::*;

#[inline(always)]
fn touch_shared_imgproc_symbols() {
    let _ = ResizeStrategy::as_str;
    let _ = ImgBuf::as_frames;
    let _ = RasterImage::is_animated;
    let _ = RasterImage::as_frames;
    let _ = compress_frames;
}

pub fn load_wallpaper_bytes(
    path: &std::path::Path,
    dimensions: (u32, u32),
    pixel_format: common::ipc::PixelFormat,
    resize: ResizeStrategy,
) -> Result<Box<[u8]>, String> {
    touch_shared_imgproc_symbols();

    let img_buf = ImgBuf::new(path)?;
    let _ = img_buf.is_animated();

    match img_buf.decode_prepare() {
        DecodeBuffer::RasterImage(imgbuf) => {
            let img_raw = imgbuf.decode(pixel_format)?;
            match resize {
                ResizeStrategy::No => Ok(img_pad(&img_raw, dimensions, [0, 0, 0, 255])),
                ResizeStrategy::Crop => img_resize_crop(
                    &img_raw,
                    dimensions,
                    fast_image_resize::FilterType::Lanczos3,
                ),
                ResizeStrategy::Fit => img_resize_fit(
                    &img_raw,
                    dimensions,
                    fast_image_resize::FilterType::Lanczos3,
                    [0, 0, 0, 255],
                ),
                ResizeStrategy::Stretch => img_resize_stretch(
                    &img_raw,
                    dimensions,
                    fast_image_resize::FilterType::Lanczos3,
                ),
            }
        }
        DecodeBuffer::VectorImage(imgbuf) => {
            let img = imgbuf.decode(pixel_format, dimensions.0, dimensions.1)?;
            match resize {
                ResizeStrategy::No => Ok(img_pad(&img, dimensions, [0, 0, 0, 255])),
                ResizeStrategy::Crop => {
                    img_resize_crop(&img, dimensions, fast_image_resize::FilterType::Lanczos3)
                }
                ResizeStrategy::Fit => img_resize_fit(
                    &img,
                    dimensions,
                    fast_image_resize::FilterType::Lanczos3,
                    [0, 0, 0, 255],
                ),
                ResizeStrategy::Stretch => {
                    img_resize_stretch(&img, dimensions, fast_image_resize::FilterType::Lanczos3)
                }
            }
        }
    }
}
