use anyhow::{Context, Result};
use image::ImageReader;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::warn;
use vellum_ipc::{Request, Response, ScaleMode};

use crate::monitor::{
    detect_monitor_layouts, normalize_monitor_snapshot, MonitorLayout, MonitorSnapshot,
};
use crate::renderer::{OutputLayout, RendererState};
use crate::state::{assignment_entries, save_state, DaemonState, WallpaperAssignment};

pub(crate) struct HandlerOutcome {
    pub(crate) response: Response,
    pub(crate) shutdown: bool,
}

pub(crate) async fn handle_request(
    request: Request,
    daemon_state: &Arc<Mutex<DaemonState>>,
    renderer_state: &Arc<Mutex<RendererState>>,
    monitor_snapshot: &MonitorSnapshot,
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
            let monitors = get_or_refresh_monitors(monitor_snapshot, renderer_state)
                .await
                .ok();
            let mut state = daemon_state.lock().await;
            match apply_wallpaper_native(
                &path,
                monitor.as_deref(),
                mode,
                &mut state,
                monitors.as_deref(),
            ) {
                Ok((canonical, previous_assignment)) => {
                    drop(state);

                    let mut renderer = renderer_state.lock().await;
                    renderer.enqueue_apply(monitor.clone(), canonical, mode);
                    match renderer.apply_pending() {
                        Ok(()) => {
                            drop(renderer);

                            let state = daemon_state.lock().await;
                            if let Err(err) = save_state(state_path, &state) {
                                warn!(error = %err, "failed to persist daemon state");
                            }

                            HandlerOutcome {
                                response: Response::Ok,
                                shutdown: false,
                            }
                        }
                        Err(err) => {
                            drop(renderer);

                            let mut state = daemon_state.lock().await;
                            let key = monitor.clone();
                            if let Some(previous) = previous_assignment {
                                state.assignments.insert(key, previous);
                            } else {
                                state.assignments.remove(&key);
                            }

                            HandlerOutcome {
                                response: Response::Error {
                                    message: format!(
                                        "failed to apply wallpaper to renderer: {err:#}"
                                    ),
                                },
                                shutdown: false,
                            }
                        }
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
        Request::GetMonitors => {
            match get_or_refresh_monitors(monitor_snapshot, renderer_state).await {
                Ok(monitors) => HandlerOutcome {
                    response: Response::Monitors { names: monitors },
                    shutdown: false,
                },
                Err(err) => HandlerOutcome {
                    response: Response::Error {
                        message: format!("failed to query monitors: {err:#}"),
                    },
                    shutdown: false,
                },
            }
        }
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
            let mut renderer = renderer_state.lock().await;
            renderer.enqueue_clear();
            match renderer.apply_pending() {
                Ok(()) => {
                    drop(renderer);

                    let mut state = daemon_state.lock().await;
                    state.assignments.clear();
                    if let Err(err) = save_state(state_path, &state) {
                        warn!(error = %err, "failed to persist daemon state after clear");
                    }

                    HandlerOutcome {
                        response: Response::Ok,
                        shutdown: false,
                    }
                }
                Err(err) => HandlerOutcome {
                    response: Response::Error {
                        message: format!("failed to clear renderer assignments: {err:#}"),
                    },
                    shutdown: false,
                },
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
    monitors: Option<&[String]>,
) -> Result<(PathBuf, Option<WallpaperAssignment>)> {
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
        let Some(monitors) = monitors else {
            anyhow::bail!("failed to validate monitor target: monitor snapshot unavailable");
        };

        if !monitors.iter().any(|name| name == target) {
            anyhow::bail!(
                "unknown monitor target '{target}', available: {}",
                monitors.join(", ")
            );
        }
    }

    let key = monitor.map(str::to_string);
    let previous = daemon_state.assignments.insert(
        key,
        WallpaperAssignment {
            path: canonical.clone(),
            mode,
        },
    );

    Ok((canonical, previous))
}

async fn get_or_refresh_monitors(
    monitor_snapshot: &MonitorSnapshot,
    renderer_state: &Arc<Mutex<RendererState>>,
) -> Result<Vec<String>> {
    let cached = monitor_snapshot.get().await;
    if !cached.is_empty() {
        return Ok(cached);
    }

    let detected = detect_monitor_layouts().context("monitor detection failed")?;
    let names: Vec<String> = detected.iter().map(|layout| layout.name.clone()).collect();
    let normalized = normalize_monitor_snapshot(names);
    let _ = monitor_snapshot
        .replace_if_changed(normalized.clone())
        .await;

    let mut renderer = renderer_state.lock().await;
    renderer.refresh_outputs(normalized.clone());
    renderer.refresh_output_layouts(to_output_layouts(&detected));
    Ok(normalized)
}

fn to_output_layouts(layouts: &[MonitorLayout]) -> Vec<OutputLayout> {
    layouts
        .iter()
        .map(|layout| OutputLayout {
            name: layout.name.clone(),
            width: layout.width,
            height: layout.height,
            scale_factor: layout.scale_factor,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::WallpaperAssignment;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn new_temp_dir(prefix: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{nonce}"));
        std::fs::create_dir_all(&path).expect("test dir should be created");
        path
    }

    fn write_test_png(path: &PathBuf) {
        let image = image::RgbImage::from_pixel(1, 1, image::Rgb([11, 22, 33]));
        image
            .save(path)
            .expect("png fixture should be generated and written");
    }

    #[tokio::test]
    async fn set_then_clear_updates_daemon_and_renderer_state() {
        let temp_dir = new_temp_dir("vellum-handlers-test");
        let state_file = temp_dir.join("state.json");
        let image_path = temp_dir.join("sample.png");
        write_test_png(&image_path);

        let daemon_state = Arc::new(Mutex::new(DaemonState::default()));
        let renderer_state = Arc::new(Mutex::new(RendererState::default()));
        let monitor_snapshot = MonitorSnapshot::default();
        let _ = monitor_snapshot
            .replace_if_changed(vec!["DP-1".to_string()])
            .await;

        let outcome = handle_request(
            Request::SetWallpaper {
                path: image_path.display().to_string(),
                monitor: None,
                mode: ScaleMode::Crop,
            },
            &daemon_state,
            &renderer_state,
            &monitor_snapshot,
            &state_file,
        )
        .await
        .expect("set wallpaper handler should succeed");

        assert!(matches!(outcome.response, Response::Ok));
        assert!(!outcome.shutdown);

        {
            let state = daemon_state.lock().await;
            assert_eq!(state.assignments.len(), 1);
        }

        {
            let renderer = renderer_state.lock().await;
            assert_eq!(renderer.backend_assignment_count(), 1);
            assert_eq!(renderer.backend_mode_for(None), Some(ScaleMode::Crop));
        }

        let outcome = handle_request(
            Request::ClearAssignments,
            &daemon_state,
            &renderer_state,
            &monitor_snapshot,
            &state_file,
        )
        .await
        .expect("clear assignments handler should succeed");

        assert!(matches!(outcome.response, Response::Ok));

        {
            let state = daemon_state.lock().await;
            assert!(state.assignments.is_empty());
        }

        {
            let renderer = renderer_state.lock().await;
            assert_eq!(renderer.backend_assignment_count(), 0);
        }

        std::fs::remove_dir_all(temp_dir).expect("test dir should be removed");
    }

    #[test]
    fn apply_wallpaper_native_returns_previous_assignment_when_replacing() {
        let temp_dir = new_temp_dir("vellum-handlers-replace-test");
        let image_a = temp_dir.join("a.png");
        let image_b = temp_dir.join("b.png");
        write_test_png(&image_a);
        write_test_png(&image_b);

        let mut state = DaemonState::default();
        state.assignments.insert(
            None,
            WallpaperAssignment {
                path: image_a.clone(),
                mode: ScaleMode::Fit,
            },
        );

        let (_, previous) = apply_wallpaper_native(
            &image_b.display().to_string(),
            None,
            ScaleMode::Crop,
            &mut state,
            None,
        )
        .expect("replacement should succeed");

        let previous = previous.expect("previous assignment should be returned");
        assert_eq!(
            previous.path,
            image_a.canonicalize().expect("canonical path")
        );
        assert_eq!(previous.mode, ScaleMode::Fit);

        std::fs::remove_dir_all(temp_dir).expect("test dir should be removed");
    }

    #[test]
    fn apply_wallpaper_native_validates_monitor_from_snapshot() {
        let temp_dir = new_temp_dir("vellum-handlers-monitor-validate-test");
        let image = temp_dir.join("sample.png");
        write_test_png(&image);

        let mut state = DaemonState::default();
        let monitors = vec!["DP-1".to_string()];
        let err = apply_wallpaper_native(
            &image.display().to_string(),
            Some("HDMI-A-1"),
            ScaleMode::Fit,
            &mut state,
            Some(&monitors),
        )
        .expect_err("unknown monitor should fail validation");

        assert!(err.to_string().contains("unknown monitor target"));
        std::fs::remove_dir_all(temp_dir).expect("test dir should be removed");
    }
}
