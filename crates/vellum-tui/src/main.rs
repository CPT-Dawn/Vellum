mod app;
mod cli;
mod daemon_client;
mod display;
mod images;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use std::path::PathBuf;
use std::time::Duration as StdDuration;
use tracing::info;
use vellum_ipc::{Request, Response};

use crate::app::input::handle_key_event;
use crate::app::state::{print_assignment_entries, App};
use crate::app::ui::draw_frame;
use crate::cli::{Args, Command};
use crate::daemon_client::{resolve_socket_path, resolve_socket_path_optional, send_request};
use crate::display::MonitorProfile;
use crate::images::default_image_root;

use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .compact()
        .init();

    let args = Args::parse();
    let command = args.command.unwrap_or(Command::Ui);

    match command {
        Command::Ui => run_ui(
            args.socket,
            args.images_dir,
            args.monitor_width,
            args.monitor_height,
        ),
        Command::Ping => {
            let socket_path = resolve_socket_path(args.socket)?;
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
            let socket_path = resolve_socket_path(args.socket)?;
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
            let socket_path = resolve_socket_path(args.socket)?;
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
            let socket_path = resolve_socket_path(args.socket)?;
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
            let socket_path = resolve_socket_path(args.socket)?;
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
            let socket_path = resolve_socket_path(args.socket)?;
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

fn run_ui(
    socket: Option<PathBuf>,
    images_dir: Option<PathBuf>,
    monitor_width: Option<u32>,
    monitor_height: Option<u32>,
) -> Result<()> {
    let image_root = images_dir.unwrap_or_else(default_image_root);
    let monitor_profile = MonitorProfile::resolve(monitor_width, monitor_height);
    let socket_path = resolve_socket_path_optional(socket);
    let mut app = App::discover_files(image_root, monitor_profile, socket_path)?;

    enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("failed to initialize terminal UI")?;

    let app_result = run_ui_loop(&mut terminal, &mut app);

    disable_raw_mode().context("failed to disable raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("failed to leave alternate screen")?;
    terminal.show_cursor().context("failed to show cursor")?;

    app_result
}

fn run_ui_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal
            .draw(|frame| draw_frame(frame, app))
            .context("failed to draw UI frame")?;

        if !event::poll(StdDuration::from_millis(200)).context("event polling failed")? {
            continue;
        }

        let ev = event::read().context("failed to read terminal event")?;
        if let Event::Key(key) = ev {
            if handle_key_event(app, key) {
                return Ok(());
            }
        }
    }
}
