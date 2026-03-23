use anyhow::{Context, Result};
use image::DynamicImage;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

pub(crate) fn default_image_root() -> PathBuf {
    dirs::picture_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join("Pictures")))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub(crate) fn load_image(path: &Path) -> Result<DynamicImage> {
    image::ImageReader::open(path)
        .with_context(|| format!("failed to open image {}", path.display()))?
        .decode()
        .with_context(|| format!("failed to decode image {}", path.display()))
}

pub(crate) fn is_supported_image_path(path: &Path) -> bool {
    match path.extension().and_then(OsStr::to_str) {
        Some(ext) => matches!(
            ext.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "webp" | "bmp"
        ),
        None => false,
    }
}
