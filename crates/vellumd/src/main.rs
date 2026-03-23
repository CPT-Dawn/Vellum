use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use image::ImageReader;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::select;
use tokio::sync::watch;
use tracing::{error, info, warn};
use vellum_ipc::{Request, RequestEnvelope, Response, ResponseEnvelope};

#[derive(Debug, Parser)]
#[command(name = "vellumd", about = "Vellum wallpaper daemon")]
struct Args {
    #[arg(long, value_name = "PATH")]
    socket: Option<PathBuf>,

    #[arg(long, value_enum, default_value_t = BackendKind::Auto)]
    backend: BackendKind,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum BackendKind {
    Auto,
    Native,
    Swww,
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
    let backend = args.backend;

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
                let backend = backend;
                tokio::spawn(async move {
                    if let Err(err) = handle_client(stream, shutdown_tx, backend).await {
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

async fn handle_client(
    stream: UnixStream,
    shutdown_tx: watch::Sender<bool>,
    backend: BackendKind,
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
            Request::SetWallpaper { path, monitor } => {
                match apply_wallpaper(&path, monitor.as_deref(), backend) {
                    Ok(()) => Response::Ok,
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

fn apply_wallpaper(path: &str, monitor: Option<&str>, backend: BackendKind) -> Result<()> {
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

    match backend {
        BackendKind::Native => apply_wallpaper_native(&canonical, monitor),
        BackendKind::Swww => apply_wallpaper_swww(&canonical, monitor),
        BackendKind::Auto => match apply_wallpaper_native(&canonical, monitor) {
            Ok(()) => Ok(()),
            Err(native_err) => {
                warn!(error = %native_err, "native backend unavailable, falling back to swww");
                apply_wallpaper_swww(&canonical, monitor).with_context(|| {
                        format!(
                            "auto backend failed: native path failed ({native_err:#}) and swww fallback failed"
                        )
                    })
            }
        },
    }
}

fn apply_wallpaper_native(path: &PathBuf, _monitor: Option<&str>) -> Result<()> {
    let _ = path;
    anyhow::bail!(
        "native backend is not yet enabled in this build; run vellumd with --backend swww or --backend auto"
    )
}

fn apply_wallpaper_swww(path: &PathBuf, monitor: Option<&str>) -> Result<()> {
    if !command_exists("swww") {
        anyhow::bail!("swww is not installed");
    }

    ensure_swww_daemon_running().context("failed to ensure swww daemon is running")?;

    let mut command = ProcessCommand::new("swww");
    command
        .arg("img")
        .arg(path)
        .arg("--transition-type")
        .arg("simple");

    if let Some(output) = monitor {
        command.arg("--outputs").arg(output);
    }

    let status = command.status().context("failed to execute swww")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("swww exited with non-zero status")
    }
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

fn command_exists(command: &str) -> bool {
    ProcessCommand::new("which")
        .arg(command)
        .status()
        .is_ok_and(|status| status.success())
}

fn ensure_swww_daemon_running() -> Result<()> {
    let query_status = ProcessCommand::new("swww")
        .arg("query")
        .status()
        .context("failed to query swww daemon status")?;

    if query_status.success() {
        return Ok(());
    }

    if !command_exists("swww-daemon") {
        anyhow::bail!("swww-daemon binary is missing")
    }

    let daemon_status = ProcessCommand::new("swww-daemon")
        .arg("--format")
        .arg("xrgb")
        .status()
        .context("failed to start swww-daemon")?;

    if daemon_status.success() {
        Ok(())
    } else {
        anyhow::bail!("swww-daemon exited with non-zero status")
    }
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
