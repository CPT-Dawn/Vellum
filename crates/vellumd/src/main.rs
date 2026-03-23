mod cli;
mod ipc;
mod monitor;
mod paths;
mod renderer;
mod state;

use anyhow::{Context, Result};
use clap::Parser;
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio::select;
use tokio::sync::{watch, Mutex};
use tracing::{info, warn};

use crate::cli::Args;
use crate::ipc::server::run_client_session;
use crate::monitor::detect_monitor_names;
use crate::paths::{resolve_socket_path, resolve_state_path};
use crate::renderer::RendererState;
use crate::state::load_state;

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
                        run_client_session(stream, shutdown_tx, state, renderer_state, state_path)
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
