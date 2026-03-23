use anyhow::{Context, Result};
use std::path::PathBuf;

pub(crate) fn resolve_socket_path(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }

    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .context("XDG_RUNTIME_DIR is not set; pass --socket explicitly")?;
    Ok(PathBuf::from(runtime_dir).join("vellum.sock"))
}

pub(crate) fn resolve_state_path(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }

    if let Ok(state_home) = std::env::var("XDG_STATE_HOME") {
        return Ok(PathBuf::from(state_home).join("vellum").join("state.json"));
    }

    let home = std::env::var("HOME").context("HOME is not set; pass --state-file explicitly")?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("state")
        .join("vellum")
        .join("state.json"))
}
