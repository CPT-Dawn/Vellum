use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::warn;
use vellum_ipc::{AssignmentEntry, ScaleMode};

#[derive(Default)]
pub(crate) struct DaemonState {
    // `None` means "all outputs" target; Some(name) is per-monitor targeting.
    pub(crate) assignments: HashMap<Option<String>, WallpaperAssignment>,
}

pub(crate) struct WallpaperAssignment {
    pub(crate) path: PathBuf,
    pub(crate) mode: ScaleMode,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedState {
    assignments: Vec<PersistedAssignment>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedAssignment {
    monitor: Option<String>,
    path: String,
    mode: ScaleMode,
}

pub(crate) fn assignment_entries(
    assignments: &HashMap<Option<String>, WallpaperAssignment>,
) -> Vec<AssignmentEntry> {
    let mut entries: Vec<AssignmentEntry> = assignments
        .iter()
        .map(|(monitor, assignment)| AssignmentEntry {
            monitor: monitor.clone(),
            path: assignment.path.display().to_string(),
            mode: assignment.mode,
        })
        .collect();

    entries.sort_by(|a, b| a.monitor.cmp(&b.monitor));
    entries
}

pub(crate) fn load_state(state_path: &PathBuf) -> Result<DaemonState> {
    if !state_path.exists() {
        return Ok(DaemonState::default());
    }

    let payload = std::fs::read_to_string(state_path)
        .with_context(|| format!("failed reading state file {}", state_path.display()))?;
    let persisted: PersistedState = serde_json::from_str(&payload)
        .with_context(|| format!("failed decoding state file {}", state_path.display()))?;

    let mut assignments = HashMap::new();
    for entry in persisted.assignments {
        let input = PathBuf::from(entry.path);
        let canonical = match input.canonicalize() {
            Ok(path) => path,
            Err(err) => {
                warn!(error = %err, path = %input.display(), "skipping invalid persisted wallpaper path");
                continue;
            }
        };

        if !canonical.is_file() {
            warn!(path = %canonical.display(), "skipping non-file persisted wallpaper path");
            continue;
        }

        assignments.insert(
            entry.monitor,
            WallpaperAssignment {
                path: canonical,
                mode: entry.mode,
            },
        );
    }

    Ok(DaemonState { assignments })
}

pub(crate) fn save_state(state_path: &PathBuf, state: &DaemonState) -> Result<()> {
    if let Some(parent) = state_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed creating state directory {}", parent.display()))?;
    }

    let mut persisted = PersistedState {
        assignments: state
            .assignments
            .iter()
            .map(|(monitor, assignment)| PersistedAssignment {
                monitor: monitor.clone(),
                path: assignment.path.display().to_string(),
                mode: assignment.mode,
            })
            .collect(),
    };

    persisted
        .assignments
        .sort_by(|a, b| a.monitor.cmp(&b.monitor));

    let json =
        serde_json::to_string_pretty(&persisted).context("failed encoding daemon state to JSON")?;
    std::fs::write(state_path, json)
        .with_context(|| format!("failed writing state file {}", state_path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn save_and_load_state_roundtrip() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let base = std::env::temp_dir().join(format!("vellum-daemon-test-{nonce}"));
        std::fs::create_dir_all(&base).expect("test directory should be created");

        let image_a = base.join("wall-a.png");
        let image_b = base.join("wall-b.png");
        std::fs::write(&image_a, b"a").expect("should create image A fixture");
        std::fs::write(&image_b, b"b").expect("should create image B fixture");

        let mut state = DaemonState::default();
        state.assignments.insert(
            None,
            WallpaperAssignment {
                path: image_a.clone(),
                mode: ScaleMode::Fit,
            },
        );
        state.assignments.insert(
            Some("DP-1".to_string()),
            WallpaperAssignment {
                path: image_b.clone(),
                mode: ScaleMode::Crop,
            },
        );

        let state_path = base.join("state.json");
        save_state(&state_path, &state).expect("state should save");
        let loaded = load_state(&state_path).expect("state should load");

        let entries = assignment_entries(&loaded.assignments);
        assert_eq!(entries.len(), 2);

        let all_entry = entries
            .iter()
            .find(|entry| entry.monitor.is_none())
            .expect("missing all-target assignment");
        assert_eq!(all_entry.mode, ScaleMode::Fit);

        let monitor_entry = entries
            .iter()
            .find(|entry| entry.monitor.as_deref() == Some("DP-1"))
            .expect("missing DP-1 assignment");
        assert_eq!(monitor_entry.mode, ScaleMode::Crop);

        std::fs::remove_dir_all(&base).expect("test directory should be removed");
    }

    #[test]
    fn save_state_with_no_assignments_writes_empty_list() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let base = std::env::temp_dir().join(format!("vellum-daemon-test-empty-{nonce}"));
        std::fs::create_dir_all(&base).expect("test directory should be created");

        let state = DaemonState::default();
        let state_path = base.join("state.json");
        save_state(&state_path, &state).expect("empty state should save");
        let loaded = load_state(&state_path).expect("empty state should load");

        assert!(loaded.assignments.is_empty());

        std::fs::remove_dir_all(&base).expect("test directory should be removed");
    }
}
