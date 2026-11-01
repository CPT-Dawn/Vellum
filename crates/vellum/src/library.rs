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
            if let Some(hint) = missing_hint(folders) {
                return Err(format!("{}: no such file, and {hint}", file.display()));
            }
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

/// One line telling a CLI user what to do when none of the Library
/// folders exist, or `None` if one does. (The TUI shows its welcome
/// screen instead.)
pub fn missing_hint(folders: &[PathBuf]) -> Option<String> {
    if folders.iter().any(|f| f.is_dir()) {
        return None;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let names: Vec<String> = folders
        .iter()
        .map(|f| crate::config::edit::tilde(f, home.as_deref()))
        .collect();
    let what = match names.as_slice() {
        [] => "no Library folder is set".to_owned(),
        [one] => format!("the Library folder {one} does not exist"),
        many => format!("none of the Library folders exist ({})", many.join(", ")),
    };
    Some(format!(
        "{what}; run `vellum` to choose one, or set library = [\"~/Pictures\"] in the Config"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_library_gets_a_one_line_hint() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("Wallpapers");
        let hint = missing_hint(std::slice::from_ref(&missing)).unwrap();
        assert!(!hint.contains('\n'), "{hint}");
        assert!(hint.contains("does not exist"), "{hint}");
        assert!(hint.contains("run `vellum`"), "{hint}");
        assert_eq!(missing_hint(&[missing, dir.path().to_owned()]), None);
    }

    #[test]
    fn set_by_name_without_a_library_says_how_to_get_one() {
        let dir = tempfile::tempdir().unwrap();
        let err = resolve(Path::new("dunes.jpg"), &[dir.path().join("nope")]).unwrap_err();
        assert!(err.contains("run `vellum` to choose one"), "{err}");
        assert!(!err.contains('\n'), "{err}");
    }
}
