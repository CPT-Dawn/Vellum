use anyhow::{Context, Result};
use clap::Parser;
use image::ImageReader;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::select;
use tokio::sync::{watch, Mutex};
use tracing::{error, info, warn};
use vellum_ipc::{
    AssignmentEntry, Request, RequestEnvelope, Response, ResponseEnvelope, ScaleMode,
};

#[derive(Debug, Parser)]
#[command(name = "vellumd", about = "Vellum wallpaper daemon")]
struct Args {
    #[arg(long, value_name = "PATH")]
    socket: Option<PathBuf>,

    #[arg(long, value_name = "PATH")]
    state_file: Option<PathBuf>,
}

#[derive(Default)]
struct DaemonState {
    // `None` means "all outputs" target; Some(name) is per-monitor targeting.
    assignments: HashMap<Option<String>, WallpaperAssignment>,
}

struct WallpaperAssignment {
    path: PathBuf,
    mode: ScaleMode,
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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .compact()
        .init();

    let args = Args::parse();
    let socket_path = resolve_socket_path(args.socket)?;
    let state_path = resolve_state_path(args.state_file)?;
    let state = Arc::new(Mutex::new(load_state(&state_path)?));

    if socket_path.exists() {
        std::fs::remove_file(&socket_path).with_context(|| {
            format!("failed to remove stale socket at {}", socket_path.display())
        })?;
    }

    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("failed to bind daemon socket at {}", socket_path.display()))?;
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

    info!(path = %socket_path.display(), "vellumd listening");

    loop {
        select! {
            accept_result = listener.accept() => {
                let (stream, _) = accept_result.context("socket accept failed")?;
                let shutdown_tx = shutdown_tx.clone();
                let state = Arc::clone(&state);
                let state_path = state_path.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_client(stream, shutdown_tx, state, state_path).await {
                        warn!(error = %err, "client session ended with error");
                    }
                });
            }

            signal_result = tokio::signal::ctrl_c() => {
                signal_result.context("failed to listen for Ctrl-C")?;
                info!("Ctrl-C received, shutting down");
                break;
            }

            changed = shutdown_rx.changed() => {
                changed.context("shutdown channel unexpectedly closed")?;
                if *shutdown_rx.borrow_and_update() {
                    info!("shutdown requested by client");
                    break;
                }
            }
        }
    }

    if socket_path.exists() {
        std::fs::remove_file(&socket_path).with_context(|| {
            format!(
                "failed to remove daemon socket at {}",
                socket_path.display()
            )
        })?;
    }

    info!("vellumd terminated cleanly");
    Ok(())
}

fn resolve_socket_path(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }

    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .context("XDG_RUNTIME_DIR is not set; pass --socket explicitly")?;
    Ok(PathBuf::from(runtime_dir).join("vellum.sock"))
}

fn resolve_state_path(explicit: Option<PathBuf>) -> Result<PathBuf> {
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

async fn handle_client(
    stream: UnixStream,
    shutdown_tx: watch::Sender<bool>,
    daemon_state: Arc<Mutex<DaemonState>>,
    state_path: PathBuf,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        let bytes = reader
            .read_line(&mut line)
            .await
            .context("failed to read from client socket")?;

        if bytes == 0 {
            return Ok(());
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let envelope = match serde_json::from_str::<RequestEnvelope>(trimmed) {
            Ok(envelope) => envelope,
            Err(err) => {
                let response = Response::Error {
                    message: format!("invalid request json: {err}"),
                };
                send_response(&mut writer, &response).await?;
                continue;
            }
        };

        if let Err(err) = envelope.validate_version() {
            let response = Response::Error {
                message: err.to_string(),
            };
            send_response(&mut writer, &response).await?;
            continue;
        }

        let request = envelope.request;

        let response = match request {
            Request::Ping => Response::Pong,
            Request::SetWallpaper {
                path,
                monitor,
                mode,
            } => {
                let mut state = daemon_state.lock().await;
                match apply_wallpaper_native(&path, monitor.as_deref(), mode, &mut state) {
                    Ok(()) => {
                        if let Err(err) = save_state(&state_path, &state) {
                            warn!(error = %err, "failed to persist daemon state");
                        }
                        Response::Ok
                    }
                    Err(err) => Response::Error {
                        message: format!("failed to apply wallpaper: {err:#}"),
                    },
                }
            }
            Request::GetMonitors => match detect_monitor_names() {
                Ok(monitors) => Response::Monitors { names: monitors },
                Err(err) => Response::Error {
                    message: format!("failed to query monitors: {err:#}"),
                },
            },
            Request::GetAssignments => {
                let state = daemon_state.lock().await;
                Response::Assignments {
                    entries: assignment_entries(&state.assignments),
                }
            }
            Request::KillDaemon => {
                let response = Response::Ok;
                send_response(&mut writer, &response).await?;
                let _ = shutdown_tx.send(true);
                return Ok(());
            }
        };

        if let Err(err) = send_response(&mut writer, &response).await {
            error!(error = %err, "failed sending response to client");
            return Err(err);
        }
    }
}

fn apply_wallpaper_native(
    path: &str,
    monitor: Option<&str>,
    mode: ScaleMode,
    daemon_state: &mut DaemonState,
) -> Result<()> {
    let input = PathBuf::from(path);
    let canonical = input
        .canonicalize()
        .with_context(|| format!("invalid wallpaper path: {}", input.display()))?;

    if !canonical.is_file() {
        anyhow::bail!("path is not a file: {}", canonical.display());
    }

    ImageReader::open(&canonical)
        .with_context(|| format!("unable to open image: {}", canonical.display()))?
        .decode()
        .with_context(|| format!("unable to decode image: {}", canonical.display()))?;

    if let Some(target) = monitor {
        let monitors = detect_monitor_names().context("failed to validate monitor target")?;
        if !monitors.iter().any(|name| name == target) {
            anyhow::bail!(
                "unknown monitor target '{target}', available: {}",
                monitors.join(", ")
            );
        }
    }

    let key = monitor.map(str::to_string);
    daemon_state.assignments.insert(
        key,
        WallpaperAssignment {
            path: canonical.clone(),
            mode,
        },
    );
    info!(path = %canonical.display(), target = ?monitor, ?mode, "accepted native wallpaper assignment");

    // Rendering pipeline wiring (SCTK + layer-shell + wl_shm) is introduced incrementally.
    Ok(())
}

fn assignment_entries(
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

fn load_state(state_path: &PathBuf) -> Result<DaemonState> {
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

fn save_state(state_path: &PathBuf, state: &DaemonState) -> Result<()> {
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

fn detect_monitor_names() -> Result<Vec<String>> {
    if let Some(monitors) = detect_hyprland_monitors() {
        return Ok(monitors);
    }

    if let Some(monitors) = detect_wlr_randr_monitors() {
        return Ok(monitors);
    }

    anyhow::bail!("unable to detect monitors from hyprctl or wlr-randr")
}

fn detect_hyprland_monitors() -> Option<Vec<String>> {
    let value = run_json_command("hyprctl", &["monitors", "-j"])?;
    let items = value.as_array()?;
    let mut names = Vec::new();
    for item in items {
        if let Some(name) = item.get("name").and_then(Value::as_str) {
            names.push(name.to_string());
        }
    }

    if names.is_empty() {
        None
    } else {
        Some(names)
    }
}

fn detect_wlr_randr_monitors() -> Option<Vec<String>> {
    let value = run_json_command("wlr-randr", &["--json"])?;
    let items = value.as_array()?;
    let mut names = Vec::new();
    for item in items {
        if let Some(name) = item
            .get("name")
            .or_else(|| item.get("output"))
            .and_then(Value::as_str)
        {
            names.push(name.to_string());
        }
    }

    if names.is_empty() {
        None
    } else {
        Some(names)
    }
}

fn run_json_command(command: &str, args: &[&str]) -> Option<Value> {
    let output = ProcessCommand::new(command).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    serde_json::from_str::<Value>(&stdout).ok()
}

async fn send_response(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    response: &Response,
) -> Result<()> {
    let envelope = ResponseEnvelope::new(response.clone());
    let payload = serde_json::to_string(&envelope).context("failed to serialize response")?;
    writer
        .write_all(payload.as_bytes())
        .await
        .context("failed to write response payload")?;
    writer
        .write_all(b"\n")
        .await
        .context("failed to write response newline")?;
    writer.flush().await.context("failed to flush response")?;
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
}
