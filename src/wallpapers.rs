//! Filesystem wallpaper discovery and fuzzy filtering helpers.

use std::{
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
};

use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};

/// One wallpaper entry displayed by the browser pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallpaperItem {
    /// File name shown in the TUI list.
    pub name: String,
    /// Absolute path used for backend apply operations.
    pub path: PathBuf,
    /// Parsed image dimensions used by aspect ratio simulator.
    pub dimensions: Option<(u32, u32)>,
}

/// Scans a directory recursively for supported image files.
pub fn discover_wallpapers(root: &Path) -> io::Result<Vec<WallpaperItem>> {
    let mut items = Vec::new();
    walk_dir(root, &mut items)?;
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(items)
}

/// Returns fuzzy-ranked item indices for a search query.
#[must_use]
pub fn fuzzy_filter_indices(items: &[WallpaperItem], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..items.len()).collect();
    }

    let matcher = SkimMatcherV2::default();
    let mut scored = Vec::with_capacity(items.len());

    for (index, item) in items.iter().enumerate() {
        if let Some(score) = matcher.fuzzy_match(&item.name, query) {
            scored.push((index, score));
        }
    }

    scored.sort_by(|lhs, rhs| rhs.1.cmp(&lhs.1).then_with(|| lhs.0.cmp(&rhs.0)));
    scored.into_iter().map(|entry| entry.0).collect()
}

/// Recursively traverses directories and appends supported wallpapers to output.
fn walk_dir(root: &Path, out: &mut Vec<WallpaperItem>) -> io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            walk_dir(&path, out)?;
            continue;
        }

        if !is_supported_image(&path) {
            continue;
        }

        if let Some(name) = path.file_name().and_then(OsStr::to_str) {
            let dimensions = image::image_dimensions(&path).ok();
            out.push(WallpaperItem {
                name: name.to_owned(),
                path,
                dimensions,
            });
        }
    }

    Ok(())
}

/// Checks whether a file path matches common still-image extensions.
#[must_use]
fn is_supported_image(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(OsStr::to_str) else {
        return false;
    };

    matches!(
        ext.to_ascii_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "avif" | "bmp"
    )
}
