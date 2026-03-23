use anyhow::{Context, Result};
use std::io::{BufRead, BufReader as StdBufReader, Write};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::{timeout, Duration};
use vellum_ipc::{Request, RequestEnvelope, Response, ResponseEnvelope};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) fn resolve_socket_path(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }

    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .context("XDG_RUNTIME_DIR is not set; pass --socket explicitly")?;
    Ok(PathBuf::from(runtime_dir).join("vellum.sock"))
}

pub(crate) fn resolve_socket_path_optional(explicit: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path);
    }

    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").ok()?;
    Some(PathBuf::from(runtime_dir).join("vellum.sock"))
}

pub(crate) fn send_request_blocking(socket_path: &PathBuf, request: Request) -> Result<Response> {
    let mut stream = StdUnixStream::connect(socket_path)
        .with_context(|| format!("failed to connect to daemon at {}", socket_path.display()))?;

    let payload = serde_json::to_string(&RequestEnvelope::new(request))
        .context("failed to encode request")?;
    stream
        .write_all(payload.as_bytes())
        .context("failed to write request")?;
    stream
        .write_all(b"\n")
        .context("failed to terminate request")?;
    stream.flush().context("failed to flush request")?;

    let mut reader = StdBufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .context("failed to read daemon response")?;

    let envelope = serde_json::from_str::<ResponseEnvelope>(line.trim())
        .context("daemon returned invalid response JSON")?;
    envelope
        .validate_version()
        .context("daemon returned unsupported protocol version")?;
    Ok(envelope.response)
}

pub(crate) async fn send_request(socket_path: &PathBuf, request: Request) -> Result<Response> {
    let stream = timeout(CONNECT_TIMEOUT, UnixStream::connect(socket_path))
        .await
        .context("timed out while connecting to daemon socket")?
        .with_context(|| format!("failed to connect to daemon at {}", socket_path.display()))?;

    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    let payload = serde_json::to_string(&RequestEnvelope::new(request))
        .context("failed to encode request")?;
    writer
        .write_all(payload.as_bytes())
        .await
        .context("failed to write request")?;
    writer
        .write_all(b"\n")
        .await
        .context("failed to terminate request")?;
    writer.flush().await.context("failed to flush request")?;

    let mut line = String::new();
    timeout(RESPONSE_TIMEOUT, reader.read_line(&mut line))
        .await
        .context("timed out waiting for daemon response")?
        .context("failed to read daemon response")?;

    let envelope = serde_json::from_str::<ResponseEnvelope>(line.trim())
        .context("daemon returned invalid response JSON")?;
    envelope
        .validate_version()
        .context("daemon returned unsupported protocol version")?;
    Ok(envelope.response)
}
