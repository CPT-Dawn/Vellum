//! Serde-backed persistence for profiles and playlists.

use std::{fs, io, path::PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::backend::awww::TransitionKind;

/// Stored app state containing profiles and playlists.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StoredState {
    /// Named saved profiles.
    pub profiles: Vec<StoredProfile>,
    /// Saved playlists for automated cycling.
    pub playlists: Vec<StoredPlaylist>,
}

/// Persisted profile snapshot of one configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredProfile {
    /// Profile identifier.
    pub name: String,
    /// Selected wallpaper absolute path.
    pub wallpaper_path: String,
    /// Target monitor name if pinned.
    pub monitor_name: Option<String>,
    /// Transition kind at save time.
    pub transition_kind: TransitionKind,
    /// Transition step at save time.
    pub transition_step: u16,
    /// Transition FPS at save time.
    pub transition_fps: u16,
    /// Simulator mode string (`fit`, `fill`, `crop`).
    pub simulator_mode: String,
}

/// Persisted playlist definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredPlaylist {
    /// Playlist identifier.
    pub name: String,
    /// Ordered wallpaper absolute paths.
    pub entries: Vec<String>,
    /// Cycle interval in seconds.
    pub interval_secs: u64,
}

/// Returns the persisted state file path under XDG config directory.
pub fn state_file_path() -> io::Result<PathBuf> {
    let dirs = ProjectDirs::from("dev", "awww", "awww-tui").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "unable to resolve project config path",
        )
    })?;

    Ok(dirs.config_dir().join("state.json"))
}

/// Loads state JSON from disk, returning defaults when file does not exist.
pub fn load_state(path: &PathBuf) -> io::Result<StoredState> {
    if !path.exists() {
        return Ok(StoredState::default());
    }

    let raw = fs::read_to_string(path)?;
    let state = serde_json::from_str::<StoredState>(&raw)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    Ok(state)
}

/// Saves state JSON to disk, creating parent directories as needed.
pub fn save_state(path: &PathBuf, state: &StoredState) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let payload = serde_json::to_string_pretty(state)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    fs::write(path, payload)
}
