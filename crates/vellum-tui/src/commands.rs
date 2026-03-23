use anyhow::Result;
use std::path::PathBuf;
use tracing::info;
use vellum_ipc::{Request, Response};

use crate::app::state::print_assignment_entries;
use crate::cli::Command;
use crate::daemon_client::{resolve_socket_path, send_request};

pub(crate) async fn execute_command(command: Command, socket: Option<PathBuf>) -> Result<()> {
    match command {
        Command::Ui => Ok(()),
        Command::Ping => {
            let socket_path = resolve_socket_path(socket)?;
            let response = send_request(&socket_path, Request::Ping).await?;
            info!(?response, "daemon responded to ping");
            match response {
                Response::Pong => {
                    println!("daemon handshake successful");
                    Ok(())
                }
                other => anyhow::bail!("unexpected ping response from daemon: {other:?}"),
            }
        }
        Command::Set {
            path,
            monitor,
            mode,
        } => {
            let socket_path = resolve_socket_path(socket)?;
            let response = send_request(
                &socket_path,
                Request::SetWallpaper {
                    path: path.display().to_string(),
                    monitor,
                    mode: mode.into(),
                },
            )
            .await?;

            match response {
                Response::Ok => {
                    println!("wallpaper applied: {}", path.display());
                    Ok(())
                }
                Response::Error { message } => anyhow::bail!("daemon error: {message}"),
                other => anyhow::bail!("unexpected set response from daemon: {other:?}"),
            }
        }
        Command::Monitors => {
            let socket_path = resolve_socket_path(socket)?;
            let response = send_request(&socket_path, Request::GetMonitors).await?;

            match response {
                Response::Monitors { names } => {
                    if names.is_empty() {
                        println!("no monitors detected");
                    } else {
                        for name in names {
                            println!("{name}");
                        }
                    }
                    Ok(())
                }
                Response::Error { message } => anyhow::bail!("daemon error: {message}"),
                other => anyhow::bail!("unexpected monitors response from daemon: {other:?}"),
            }
        }
        Command::Assignments => {
            let socket_path = resolve_socket_path(socket)?;
            let response = send_request(&socket_path, Request::GetAssignments).await?;

            match response {
                Response::Assignments { entries } => {
                    print_assignment_entries(&entries);
                    Ok(())
                }
                Response::Error { message } => anyhow::bail!("daemon error: {message}"),
                other => anyhow::bail!("unexpected assignments response from daemon: {other:?}"),
            }
        }
        Command::Clear => {
            let socket_path = resolve_socket_path(socket)?;
            let response = send_request(&socket_path, Request::ClearAssignments).await?;

            match response {
                Response::Ok => {
                    println!("daemon assignments cleared");
                    Ok(())
                }
                Response::Error { message } => anyhow::bail!("daemon error: {message}"),
                other => anyhow::bail!("unexpected clear response from daemon: {other:?}"),
            }
        }
        Command::Kill => {
            let socket_path = resolve_socket_path(socket)?;
            let response = send_request(&socket_path, Request::KillDaemon).await?;
            info!(?response, "daemon responded to kill request");
            match response {
                Response::Ok => {
                    println!("daemon shutdown requested");
                    Ok(())
                }
                other => anyhow::bail!("unexpected kill response from daemon: {other:?}"),
            }
        }
    }
}
