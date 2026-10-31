//! The Library: the folders Vellum searches recursively for wallpapers.
//!
//! [`scan`] lists every wallpaper in the Library; Rotation and the TUI
//! index build on it. [`resolve`] lets `vellum set` take a name from the
//! Library instead of a full path.

use std::path::{Path, PathBuf};

/// File extensions Vellum treats as wallpapers.
pub const EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "avif", "jxl"];

fn is_wallpaper(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
}

/// Every wallpaper under `folders`, recursively, sorted by path. Folders
/// that do not exist or cannot be read are skipped; hidden entries too.
pub fn scan(folders: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending: Vec<PathBuf> = folders.to_vec();
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path();
            // Follows symlinks, as people link folders into their Library.
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                pending.push(path);
            } else if meta.is_file() && is_wallpaper(&path) {
                found.push(path);
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// The file `vellum set <file>` means: `file` itself if it exists, else a
/// Library wallpaper whose path ends with `file` (`dunes.jpg`,
/// `beach/dunes.jpg`).
pub fn resolve(file: &Path, folders: &[PathBuf]) -> Result<PathBuf, String> {
    if file.exists() || file.is_absolute() {
        return Ok(file.to_owned());
    }
    let matches: Vec<PathBuf> = scan(folders)
        .into_iter()
        .filter(|path| path.ends_with(file))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.clone()),
        [] => {
            let looked: Vec<String> = folders.iter().map(|f| f.display().to_string()).collect();
            Err(format!(
                "{}: no such file here or in the Library ({})",
                file.display(),
                looked.join(", ")
            ))
        }
        many => {
            let list: Vec<String> = many.iter().map(|p| format!("  {}", p.display())).collect();
            Err(format!(
                "{} matches several Library wallpapers; give more of the path:\n{}",
                file.display(),
                list.join("\n")
            ))
        }
    }
}
