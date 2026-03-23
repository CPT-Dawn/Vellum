use anyhow::{Context, Result};
use image::ImageReader;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::warn;
use vellum_ipc::{Request, Response, ScaleMode};

use crate::monitor::detect_monitor_names;
use crate::renderer::RendererState;
use crate::state::{assignment_entries, save_state, DaemonState, WallpaperAssignment};

pub(crate) struct HandlerOutcome {
    pub(crate) response: Response,
    pub(crate) shutdown: bool,
}

pub(crate) async fn handle_request(
    request: Request,
    daemon_state: &Arc<Mutex<DaemonState>>,
    renderer_state: &Arc<Mutex<RendererState>>,
    state_path: &PathBuf,
) -> Result<HandlerOutcome> {
    let outcome = match request {
        Request::Ping => HandlerOutcome {
            response: Response::Pong,
            shutdown: false,
        },
        Request::SetWallpaper {
            path,
            monitor,
            mode,
        } => {
            let mut state = daemon_state.lock().await;
            match apply_wallpaper_native(&path, monitor.as_deref(), mode, &mut state) {
                Ok(canonical) => {
                    if let Err(err) = save_state(state_path, &state) {
                        warn!(error = %err, "failed to persist daemon state");
                    }

                    drop(state);

                    let mut renderer = renderer_state.lock().await;
                    renderer.enqueue_apply(monitor, canonical, mode);
                    renderer.apply_pending();
                    HandlerOutcome {
                        response: Response::Ok,
                        shutdown: false,
                    }
                }
                Err(err) => HandlerOutcome {
                    response: Response::Error {
                        message: format!("failed to apply wallpaper: {err:#}"),
                    },
                    shutdown: false,
                },
            }
        }
        Request::GetMonitors => match detect_monitor_names() {
            Ok(monitors) => {
                let mut renderer = renderer_state.lock().await;
                renderer.refresh_outputs(monitors.clone());
                HandlerOutcome {
                    response: Response::Monitors { names: monitors },
                    shutdown: false,
                }
            }
            Err(err) => HandlerOutcome {
                response: Response::Error {
                    message: format!("failed to query monitors: {err:#}"),
                },
                shutdown: false,
            },
        },
        Request::GetAssignments => {
            let state = daemon_state.lock().await;
            HandlerOutcome {
                response: Response::Assignments {
                    entries: assignment_entries(&state.assignments),
                },
                shutdown: false,
            }
        }
        Request::ClearAssignments => {
            let mut state = daemon_state.lock().await;
            state.assignments.clear();
            if let Err(err) = save_state(state_path, &state) {
                warn!(error = %err, "failed to persist daemon state after clear");
            }

            drop(state);

            let mut renderer = renderer_state.lock().await;
            renderer.enqueue_clear();
            renderer.apply_pending();
            HandlerOutcome {
                response: Response::Ok,
                shutdown: false,
            }
        }
        Request::KillDaemon => HandlerOutcome {
            response: Response::Ok,
            shutdown: true,
        },
    };

    Ok(outcome)
}

fn apply_wallpaper_native(
    path: &str,
    monitor: Option<&str>,
    mode: ScaleMode,
    daemon_state: &mut DaemonState,
) -> Result<PathBuf> {
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

    Ok(canonical)
}
