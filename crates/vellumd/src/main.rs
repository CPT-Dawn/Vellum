mod cli;
mod monitor;
mod paths;
mod renderer;
mod state;

use anyhow::{Context, Result};
use clap::Parser;
use image::ImageReader;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::select;
use tokio::sync::{watch, Mutex};
use tracing::{error, info, warn};
use vellum_ipc::{Request, RequestEnvelope, Response, ResponseEnvelope, ScaleMode};

use crate::cli::Args;
use crate::monitor::detect_monitor_names;
use crate::paths::{resolve_socket_path, resolve_state_path};
use crate::renderer::RendererState;
use crate::state::{assignment_entries, load_state, save_state, DaemonState, WallpaperAssignment};

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
    let renderer_state = Arc::new(Mutex::new(RendererState::default()));

    if let Ok(monitors) = detect_monitor_names() {
        let mut renderer = renderer_state.lock().await;
        renderer.refresh_outputs(monitors);
    }

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
                let renderer_state = Arc::clone(&renderer_state);
                let state_path = state_path.clone();
                tokio::spawn(async move {
                    if let Err(err) =
                        handle_client(stream, shutdown_tx, state, renderer_state, state_path).await
                    {
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

async fn handle_client(
    stream: UnixStream,
    shutdown_tx: watch::Sender<bool>,
    daemon_state: Arc<Mutex<DaemonState>>,
    renderer_state: Arc<Mutex<RendererState>>,
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

        let response = match envelope.request {
            Request::Ping => Response::Pong,
            Request::SetWallpaper {
                path,
                monitor,
                mode,
            } => {
                let mut state = daemon_state.lock().await;
                match apply_wallpaper_native(&path, monitor.as_deref(), mode, &mut state) {
                    Ok(canonical) => {
                        if let Err(err) = save_state(&state_path, &state) {
                            warn!(error = %err, "failed to persist daemon state");
                        }

                        drop(state);

                        let mut renderer = renderer_state.lock().await;
                        renderer.enqueue_apply(monitor, canonical, mode);
                        renderer.apply_pending();
                        Response::Ok
                    }
                    Err(err) => Response::Error {
                        message: format!("failed to apply wallpaper: {err:#}"),
                    },
                }
            }
            Request::GetMonitors => match detect_monitor_names() {
                Ok(monitors) => {
                    let mut renderer = renderer_state.lock().await;
                    renderer.refresh_outputs(monitors.clone());
                    let _ = renderer.output_names();
                    Response::Monitors { names: monitors }
                }
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
            Request::ClearAssignments => {
                let mut state = daemon_state.lock().await;
                state.assignments.clear();
                if let Err(err) = save_state(&state_path, &state) {
                    warn!(error = %err, "failed to persist daemon state after clear");
                }

                drop(state);

                let mut renderer = renderer_state.lock().await;
                renderer.enqueue_clear();
                renderer.apply_pending();
                Response::Ok
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
    info!(path = %canonical.display(), target = ?monitor, ?mode, "accepted native wallpaper assignment");

    // Rendering pipeline wiring (SCTK + layer-shell + wl_shm) is introduced incrementally.
    Ok(canonical)
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
