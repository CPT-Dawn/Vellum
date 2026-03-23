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
use crate::monitor::{
    detect_monitor_layouts, normalize_monitor_snapshot, MonitorLayout, MonitorSnapshot,
};
use crate::paths::{resolve_socket_path, resolve_state_path};
use crate::renderer::{OutputLayout, RendererState};
use crate::state::{load_state, save_state, DaemonState};

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
    state_path: &PathBuf,
) {
    let snapshot = {
        let state = state.lock().await;
        replay_snapshot(&state)
    };

    if snapshot.is_empty() {
        return;
    }

    let mut failed = Vec::new();
    for (monitor, path, mode) in snapshot {
        let mut renderer = renderer_state.lock().await;
        renderer.enqueue_apply(monitor.clone(), path, mode);

        if let Err(err) = renderer.apply_pending() {
            warn!(error = %err, target = ?monitor, "failed replay assignment during startup");
            failed.push(monitor);
        }
    }

    if failed.is_empty() {
        info!("replayed persisted assignments into renderer");
        return;
    }

    let mut state = state.lock().await;
    let pruned = prune_failed_assignments(&mut state, &failed);
    if pruned == 0 {
        return;
    }

    if let Err(err) = save_state(state_path, &state) {
        warn!(error = %err, pruned, "failed to persist pruned startup assignments");
    } else {
        warn!(
            pruned,
            "pruned failed startup assignments from persisted state"
        );
    }
}

fn prune_failed_assignments(state: &mut DaemonState, failed: &[Option<String>]) -> usize {
    let mut removed = 0usize;
    for key in failed {
        if state.assignments.remove(key).is_some() {
            removed = removed.saturating_add(1);
        }
    }
    removed
}

fn monitor_detection_transition<E>(
    was_failed: bool,
    detection_result: &std::result::Result<Vec<String>, E>,
) -> (bool, bool, bool) {
    match detection_result {
        Ok(_) => {
            if was_failed {
                (false, false, true)
            } else {
                (false, false, false)
            }
        }
        Err(_) => {
            if was_failed {
                (true, false, false)
            } else {
                (true, true, false)
            }
        }
    }
}

async fn monitor_refresh_loop(
    renderer_state: Arc<Mutex<RendererState>>,
    monitor_snapshot: MonitorSnapshot,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut ticker = time::interval(Duration::from_secs(2));
    let mut monitor_detection_failed = false;

    loop {
        select! {
            _ = ticker.tick() => {
                let detection = detect_monitor_layouts();
                let names_result = detection
                    .as_ref()
                    .map(|layouts| layouts.iter().map(|layout| layout.name.clone()).collect());
                let (next_failed, log_failure, log_recovery) =
                    monitor_detection_transition(monitor_detection_failed, &names_result);
                monitor_detection_failed = next_failed;

                if log_failure {
                    if let Err(err) = &names_result {
                        warn!(error = %err, "monitor refresh detection failed; keeping last known snapshot");
                    }
                }
                if log_recovery {
                    info!("monitor refresh detection recovered");
                }

                if let Ok(layouts) = detection {
                    let names: Vec<String> = layouts.iter().map(|layout| layout.name.clone()).collect();
                    let normalized = normalize_monitor_snapshot(names);
                    if monitor_snapshot
                        .replace_if_changed(normalized.clone())
                        .await
                    {
                        let mut renderer = renderer_state.lock().await;
                        renderer.refresh_outputs(normalized.clone());
                        renderer.refresh_output_layouts(to_output_layouts(&layouts));
                        info!(outputs = ?normalized, "monitor snapshot changed; renderer outputs refreshed");
                    } else {
                        let mut renderer = renderer_state.lock().await;
                        renderer.refresh_output_layouts(to_output_layouts(&layouts));
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

    if let Ok(layouts) = detect_monitor_layouts() {
        let names: Vec<String> = layouts.iter().map(|layout| layout.name.clone()).collect();
        let normalized = normalize_monitor_snapshot(names);
        let _ = monitor_snapshot
            .replace_if_changed(normalized.clone())
            .await;
        let mut renderer = renderer_state.lock().await;
        renderer.refresh_outputs(normalized.clone());
        renderer.refresh_output_layouts(to_output_layouts(&layouts));
    }

    replay_persisted_assignments(Arc::clone(&state), Arc::clone(&renderer_state), &state_path)
        .await;

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
    use super::{monitor_detection_transition, prune_failed_assignments, replay_snapshot};
    use crate::state::{DaemonState, WallpaperAssignment};
    use anyhow::{anyhow, Result};
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

    #[test]
    fn prune_failed_assignments_removes_only_requested_keys() {
        let mut state = DaemonState::default();
        state.assignments.insert(
            None,
            WallpaperAssignment {
                path: PathBuf::from("/tmp/all.png"),
                mode: ScaleMode::Fill,
            },
        );
        state.assignments.insert(
            Some("DP-1".to_string()),
            WallpaperAssignment {
                path: PathBuf::from("/tmp/dp.png"),
                mode: ScaleMode::Crop,
            },
        );

        let removed = prune_failed_assignments(&mut state, &[None, Some("HDMI-A-1".to_string())]);
        assert_eq!(removed, 1);
        assert_eq!(state.assignments.len(), 1);
        assert!(state.assignments.contains_key(&Some("DP-1".to_string())));
    }

    #[test]
    fn monitor_detection_transition_logs_only_on_state_edges() {
        let ok: Result<Vec<String>> = Ok(vec!["DP-1".to_string()]);
        let err: Result<Vec<String>> = Err(anyhow!("boom"));

        let (failed, log_failure, log_recovery) = monitor_detection_transition(false, &err);
        assert!(failed);
        assert!(log_failure);
        assert!(!log_recovery);

        let (failed, log_failure, log_recovery) = monitor_detection_transition(failed, &err);
        assert!(failed);
        assert!(!log_failure);
        assert!(!log_recovery);

        let (failed, log_failure, log_recovery) = monitor_detection_transition(failed, &ok);
        assert!(!failed);
        assert!(!log_failure);
        assert!(log_recovery);

        let (failed, log_failure, log_recovery) = monitor_detection_transition(failed, &ok);
        assert!(!failed);
        assert!(!log_failure);
        assert!(!log_recovery);
    }
}
