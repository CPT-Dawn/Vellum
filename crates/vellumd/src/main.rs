mod cli;
mod ipc;
mod monitor;
mod paths;
mod renderer;
mod state;

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio::select;
use tokio::sync::{watch, Mutex};
use tokio::time::{self, Duration};
use tracing::{info, warn};
use vellum_ipc::ScaleMode;

use crate::cli::Args;
use crate::ipc::server::run_client_session;
use crate::monitor::{detect_monitor_names, normalize_monitor_snapshot, MonitorSnapshot};
use crate::paths::{resolve_socket_path, resolve_state_path};
use crate::renderer::RendererState;
use crate::state::{load_state, DaemonState};

fn replay_snapshot(state: &DaemonState) -> Vec<(Option<String>, PathBuf, ScaleMode)> {
    let mut snapshot: Vec<(Option<String>, PathBuf, ScaleMode)> = state
        .assignments
        .iter()
        .map(|(monitor, assignment)| (monitor.clone(), assignment.path.clone(), assignment.mode))
        .collect();

    snapshot.sort_by(|a, b| a.0.cmp(&b.0));
    snapshot
}

async fn replay_persisted_assignments(
    state: Arc<Mutex<DaemonState>>,
    renderer_state: Arc<Mutex<RendererState>>,
) {
    let snapshot = {
        let state = state.lock().await;
        replay_snapshot(&state)
    };

    if snapshot.is_empty() {
        return;
    }

    let mut renderer = renderer_state.lock().await;
    for (monitor, path, mode) in snapshot {
        renderer.enqueue_apply(monitor, path, mode);
    }

    if let Err(err) = renderer.apply_pending() {
        warn!(error = %err, "failed to replay persisted assignments into renderer");
    } else {
        info!("replayed persisted assignments into renderer");
    }
}

async fn monitor_refresh_loop(
    renderer_state: Arc<Mutex<RendererState>>,
    monitor_snapshot: MonitorSnapshot,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut ticker = time::interval(Duration::from_secs(2));

    loop {
        select! {
            _ = ticker.tick() => {
                match detect_monitor_names() {
                    Ok(monitors) => {
                        let normalized = normalize_monitor_snapshot(monitors);
                        if monitor_snapshot
                            .replace_if_changed(normalized.clone())
                            .await
                        {
                            let mut renderer = renderer_state.lock().await;
                            renderer.refresh_outputs(normalized.clone());
                            info!(outputs = ?normalized, "monitor snapshot changed; renderer outputs refreshed");
                        }
                    }
                    Err(err) => {
                        warn!(error = %err, "monitor refresh tick failed");
                    }
                }
            }
            changed = shutdown_rx.changed() => {
                if changed.is_err() {
                    break;
                }
                if *shutdown_rx.borrow_and_update() {
                    break;
                }
            }
        }
    }
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
    let renderer_state = Arc::new(Mutex::new(RendererState::default()));
    let monitor_snapshot = MonitorSnapshot::default();

    if socket_path.exists() {
        std::fs::remove_file(&socket_path).with_context(|| {
            format!("failed to remove stale socket at {}", socket_path.display())
        })?;
    }

    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("failed to bind daemon socket at {}", socket_path.display()))?;
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

    if let Ok(monitors) = detect_monitor_names() {
        let normalized = normalize_monitor_snapshot(monitors);
        let _ = monitor_snapshot
            .replace_if_changed(normalized.clone())
            .await;
        let mut renderer = renderer_state.lock().await;
        renderer.refresh_outputs(normalized.clone());
    }

    replay_persisted_assignments(Arc::clone(&state), Arc::clone(&renderer_state)).await;

    tokio::spawn(monitor_refresh_loop(
        Arc::clone(&renderer_state),
        monitor_snapshot.clone(),
        shutdown_tx.subscribe(),
    ));

    info!(path = %socket_path.display(), "vellumd listening");

    loop {
        select! {
            accept_result = listener.accept() => {
                let (stream, _) = accept_result.context("socket accept failed")?;
                let shutdown_tx = shutdown_tx.clone();
                let state = Arc::clone(&state);
                let renderer_state = Arc::clone(&renderer_state);
                let monitor_snapshot = monitor_snapshot.clone();
                let state_path = state_path.clone();
                tokio::spawn(async move {
                    if let Err(err) =
                        run_client_session(
                            stream,
                            shutdown_tx,
                            state,
                            renderer_state,
                            monitor_snapshot,
                            state_path,
                        )
                        .await
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

#[cfg(test)]
mod tests {
    use super::replay_snapshot;
    use crate::state::{DaemonState, WallpaperAssignment};
    use std::path::PathBuf;
    use vellum_ipc::ScaleMode;

    #[test]
    fn replay_snapshot_sorts_monitor_specific_after_global() {
        let mut state = DaemonState::default();
        state.assignments.insert(
            Some("DP-1".to_string()),
            WallpaperAssignment {
                path: PathBuf::from("/tmp/dp.png"),
                mode: ScaleMode::Crop,
            },
        );
        state.assignments.insert(
            None,
            WallpaperAssignment {
                path: PathBuf::from("/tmp/all.png"),
                mode: ScaleMode::Fill,
            },
        );

        let snapshot = replay_snapshot(&state);
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].0, None);
        assert_eq!(snapshot[0].1, PathBuf::from("/tmp/all.png"));
        assert_eq!(snapshot[1].0, Some("DP-1".to_string()));
        assert_eq!(snapshot[1].1, PathBuf::from("/tmp/dp.png"));
    }
}
