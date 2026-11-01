//! Favorites: wallpapers the user starred, kept as one absolute path per
//! line in `$XDG_DATA_HOME/vellum/favorites` (else
//! `~/.local/share/vellum/favorites`). They are the user's own curation,
//! so they live with user data rather than State.
//!
//! A Favorite whose file has gone missing stays in the file (it may be on
//! an unmounted drive) but is skipped by [`existing`], so Rotation and
//! `vellum fav list` carry on without it.

use std::path::{Path, PathBuf};

use crate::client;
use crate::config::Config;

/// Where Favorites are kept.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|dir| Path::new(dir).is_absolute())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/share")))?;
    Some(base.join("vellum").join("favorites"))
}

/// Every Favorite, in the order they were added, missing files included.
pub fn load() -> Vec<PathBuf> {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|text| parse(&text))
        .unwrap_or_default()
}

/// The Favorites whose files exist. Missing ones are reported on stderr
/// when `warn` is set.
pub fn existing(warn: bool) -> Vec<PathBuf> {
    load()
        .into_iter()
        .filter(|path| {
            let there = path.is_file();
            if !there && warn {
                eprintln!("vellum: skipping missing Favorite {}", path.display());
            }
            there
        })
        .collect()
}

fn parse(text: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let path = PathBuf::from(line);
        if path.is_absolute() && !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

fn save(favorites: &[PathBuf]) -> Result<(), String> {
    let path = path().ok_or("neither XDG_DATA_HOME nor HOME is set")?;
    let mut text = String::new();
    for f in favorites {
        text += &f.to_string_lossy();
        text.push('\n');
    }
    crate::atomic::write_atomically(&path, text.as_bytes())
        .map_err(|err| format!("cannot save {}: {err}", path.display()))
}

/// Makes `path` (absolute) a Favorite, or stops it being one. Doing what
/// is already so is not an error and leaves the file alone. The TUI's `f`.
pub fn set(path: &Path, favorite: bool) -> Result<(), String> {
    let mut favorites = load();
    let there = favorites.iter().any(|f| f == path);
    match (favorite, there) {
        (true, false) => favorites.push(path.to_owned()),
        (false, true) => favorites.retain(|f| f != path),
        _ => return Ok(()),
    }
    save(&favorites)
}

/// The wallpaper a `vellum fav` command means: `file` (a path or Library
/// name), else the one on `monitor`, else the one on the first Monitor
/// showing anything.
fn which(file: Option<&Path>, monitor: Option<&str>, config: &Config) -> Result<PathBuf, String> {
    if let Some(file) = file {
        let file = crate::library::resolve(file, &config.library)?;
        return Ok(std::path::absolute(&file).unwrap_or(file));
    }
    let status = client::status(&client::daemon()?)?;
    if let Some(name) = monitor
        && !status.monitors.iter().any(|m| m.name == name)
    {
        return Err(format!("no monitor named {name}"));
    }
    status
        .monitors
        .into_iter()
        .filter(|m| monitor.is_none_or(|name| m.name == name))
        .find_map(|m| m.wallpaper)
        .map(|w| PathBuf::from(w.path))
        .ok_or_else(|| "no wallpaper is shown; name the file to favorite".into())
}

/// `vellum fav add [file]`.
pub fn add(file: Option<&Path>, monitor: Option<&str>) -> Result<(), String> {
    let config = crate::config::load();
    let path = which(file, monitor, &config)?;
    if !path.is_file() {
        return Err(format!("{}: no such file", path.display()));
    }
    let mut favorites = load();
    if !favorites.contains(&path) {
        favorites.push(path.clone());
        save(&favorites)?;
    }
    println!("Favorite: {}", path.display());
    Ok(())
}

/// `vellum fav remove [file]`. Removing a wallpaper that is not a Favorite
/// is not an error.
pub fn remove(file: Option<&Path>, monitor: Option<&str>) -> Result<(), String> {
    let config = crate::config::load();
    let mut favorites = load();
    // A missing file cannot be resolved through the Library; match it as
    // given too, so stale Favorites can be removed.
    let path = match which(file, monitor, &config) {
        Ok(path) => path,
        Err(err) => match file {
            Some(file) if file.is_absolute() => file.to_owned(),
            _ => return Err(err),
        },
    };
    let before = favorites.len();
    favorites.retain(|f| f != &path);
    if favorites.len() == before {
        eprintln!("vellum: {} is not a Favorite", path.display());
        return Ok(());
    }
    save(&favorites)?;
    println!("No longer a Favorite: {}", path.display());
    Ok(())
}

/// `vellum fav list`: one Favorite per line; missing files are noted on
/// stderr and left out.
pub fn list() -> Result<(), String> {
    let favorites = existing(true);
    let mut out = String::new();
    for f in favorites {
        out += &f.to_string_lossy();
        out.push('\n');
    }
    print!("{out}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_keeps_order_and_drops_duplicates_and_junk() {
        assert_eq!(
            parse("/w/b.png\n\n/w/a.png\nrelative.png\n/w/b.png\n"),
            [PathBuf::from("/w/b.png"), PathBuf::from("/w/a.png")]
        );
    }
}
