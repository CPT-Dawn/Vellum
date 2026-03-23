mod app;
mod cli;
mod commands;
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

use crate::app::input::handle_key_event;
use crate::app::state::App;
use crate::app::ui::draw_frame;
use crate::cli::{Args, Command};
use crate::commands::execute_command;
use crate::daemon_client::resolve_socket_path_optional;
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

    if matches!(command, Command::Ui) {
        run_ui(
            args.socket,
            args.images_dir,
            args.monitor_width,
            args.monitor_height,
        )
    } else {
        execute_command(command, args.socket).await
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
