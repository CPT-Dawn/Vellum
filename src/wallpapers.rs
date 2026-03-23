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

/// Scans a directory recursively for supported image files with an item cap.
pub fn discover_wallpapers_limited(
    root: &Path,
    max_items: usize,
) -> io::Result<Vec<WallpaperItem>> {
    let mut items = Vec::new();
    walk_dir_limited(root, &mut items, max_items)?;
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

/// Traverses directories iteratively and appends supported wallpapers up to a cap.
fn walk_dir_limited(root: &Path, out: &mut Vec<WallpaperItem>, max_items: usize) -> io::Result<()> {
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err)
                if err.kind() == io::ErrorKind::PermissionDenied
                    || err.kind() == io::ErrorKind::NotFound =>
            {
                continue;
            }
            Err(err) => return Err(err),
        };

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err)
                    if err.kind() == io::ErrorKind::PermissionDenied
                        || err.kind() == io::ErrorKind::NotFound =>
                {
                    continue;
                }
                Err(err) => return Err(err),
            };

            let path = entry.path();

            if path.is_dir() {
                stack.push(path);
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

            if out.len() >= max_items {
                return Ok(());
            }
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
