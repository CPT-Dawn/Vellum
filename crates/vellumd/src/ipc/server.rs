use anyhow::{Context, Result};
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{watch, Mutex};
use tracing::{debug, error};
use vellum_ipc::{RequestEnvelope, Response, ResponseEnvelope};

use crate::ipc::handlers::handle_request;
use crate::monitor::MonitorSnapshot;
use crate::renderer::RendererState;
use crate::state::DaemonState;

pub(crate) async fn run_client_session(
    stream: UnixStream,
    shutdown_tx: watch::Sender<bool>,
    daemon_state: Arc<Mutex<DaemonState>>,
    renderer_state: Arc<Mutex<RendererState>>,
    monitor_snapshot: MonitorSnapshot,
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

        let outcome = handle_request(
            envelope.request,
            &daemon_state,
            &renderer_state,
            &monitor_snapshot,
            &state_path,
        )
        .await?;

        if let Err(err) = send_response(&mut writer, &outcome.response).await {
            if is_client_disconnect_error(&err) {
                debug!(error = %err, "client disconnected before response write completed");
                return Ok(());
            }

            error!(error = %err, "failed sending response to client");
            return Err(err);
        }

        if outcome.shutdown {
            let _ = shutdown_tx.send(true);
            return Ok(());
        }
    }
}

fn is_client_disconnect_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .map(|io| {
                matches!(
                    io.kind(),
                    ErrorKind::BrokenPipe
                        | ErrorKind::ConnectionReset
                        | ErrorKind::ConnectionAborted
                )
            })
            .unwrap_or(false)
    })
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
