mod app;
mod cli;
mod daemon_client;
mod display;
mod images;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui_image::StatefulImage;
use std::path::PathBuf;
use std::time::Duration as StdDuration;
use tracing::info;
use vellum_ipc::{Request, Response};

use crate::app::state::{print_assignment_entries, App};
use crate::cli::{Args, Command};
use crate::daemon_client::{resolve_socket_path, resolve_socket_path_optional, send_request};
use crate::display::{fit_aspect_rect, MonitorProfile};
use crate::images::default_image_root;

use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
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
            .draw(|frame| {
                let frame_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(6), Constraint::Length(2)])
                    .split(frame.area());

                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Percentage(30),
                        Constraint::Percentage(30),
                        Constraint::Percentage(40),
                    ])
                    .split(frame_chunks[0]);

                let mut list_state = ListState::default();
                if !app.files.is_empty() {
                    list_state.select(Some(app.selected));
                }

                let browser_items: Vec<ListItem> = if app.files.is_empty() {
                    vec![ListItem::new("No image files found")]
                } else {
                    app.files
                        .iter()
                        .map(|path| {
                            let name = path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("<invalid utf8>");
                            ListItem::new(Line::from(name.to_string()))
                        })
                        .collect()
                };

                let browser = List::new(browser_items)
                    .block(
                        Block::default()
                            .title("Browser [Vim Motion]")
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(app.theme.panel)),
                    )
                    .highlight_style(
                        Style::default()
                            .fg(app.theme.accent)
                            .add_modifier(Modifier::BOLD),
                    )
                    .highlight_symbol(">>");

                frame.render_stateful_widget(browser, chunks[0], &mut list_state);

                let selected = app.selected_file_name().unwrap_or("None selected");
                let assignments = app.assignments_overview();
                let metadata = Paragraph::new(format!(
                    "Root: {}\nTotal images: {}\nCursor: {}\nSelected: {}\nMonitor: {}x{} ({:.2}:1) [{}]\nTarget Output: {}\nScale Mode: {}\nAssignments: {}\nPreview: {}\nDaemon: {}\n\nMode: Normal\nHint: press ? for full keymap",
                    app.image_root.display(),
                    app.files.len(),
                    app.selected,
                    selected,
                    app.monitor_profile.width,
                    app.monitor_profile.height,
                    app.monitor_profile.aspect_ratio(),
                    app.monitor_profile.source,
                    app.current_target_label(),
                    app.scale_mode_label(),
                    assignments,
                    app.preview_info,
                    app.daemon_status(),
                ))
                .block(
                    Block::default()
                        .title("Inspector")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(app.theme.panel)),
                )
                .style(Style::default().fg(app.theme.text));

                frame.render_widget(metadata, chunks[1]);

                let preview_block = Block::default()
                    .title("Preview Stage")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(app.theme.panel));
                let preview_inner = preview_block.inner(chunks[2]);
                frame.render_widget(preview_block, chunks[2]);

                let monitor_rect = fit_aspect_rect(
                    preview_inner,
                    app.monitor_profile.width,
                    app.monitor_profile.height,
                );

                let monitor_block = Block::default()
                    .title(format!(
                        "Monitor Frame {}x{}",
                        app.monitor_profile.width, app.monitor_profile.height
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(app.theme.accent_alt));

                let monitor_inner = monitor_block.inner(monitor_rect);
                frame.render_widget(monitor_block, monitor_rect);

                if let Some(state) = app.image_state.as_mut() {
                    frame.render_stateful_widget(StatefulImage::default(), monitor_inner, state);
                } else {
                    let empty = Paragraph::new("No preview available")
                        .style(Style::default().fg(app.theme.warn));
                    frame.render_widget(empty, monitor_inner);
                }

                let status_line = if app.show_help {
                    "h/j/k/l move  gg/G top/bottom  Ctrl-u/Ctrl-d page  Enter|Space apply  t cycle-target  s cycle-scale  m monitors  a assignments  x clear  r reload  ? help  q quit"
                        .to_string()
                } else {
                    app.status.clone()
                };
                let status = Paragraph::new(status_line)
                    .style(
                        Style::default()
                            .bg(app.theme.chrome)
                            .fg(app.theme.muted)
                            .add_modifier(Modifier::BOLD),
                    )
                    .wrap(Wrap { trim: true });
                frame.render_widget(status, frame_chunks[1]);
            })
            .context("failed to draw UI frame")?;

        if !event::poll(StdDuration::from_millis(200)).context("event polling failed")? {
            continue;
        }

        let ev = event::read().context("failed to read terminal event")?;
        if let Event::Key(key) = ev {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('l') => app.select_next(),
                KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('h') => app.select_previous(),
                KeyCode::Home => app.select_first(),
                KeyCode::End | KeyCode::Char('G') => app.select_last(),
                KeyCode::PageDown => app.select_page_down(10),
                KeyCode::PageUp => app.select_page_up(10),
                KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.select_page_down(10)
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.select_page_up(10)
                }
                KeyCode::Enter | KeyCode::Char(' ') => app.apply_selected_wallpaper(),
                KeyCode::Char('t') => app.cycle_monitor_target(),
                KeyCode::Char('s') => app.cycle_scale_mode(),
                KeyCode::Char('m') => app.fetch_monitors(),
                KeyCode::Char('a') => app.fetch_assignments(),
                KeyCode::Char('x') => app.clear_assignments(),
                KeyCode::Char('r') => app.reload_files(),
                KeyCode::Char('?') => app.toggle_help(),
                KeyCode::Char('g') => {
                    if app.pending_g {
                        app.select_first();
                        app.pending_g = false;
                    } else {
                        app.pending_g = true;
                        app.status = "pending motion: g (press g again for top)".to_string();
                    }
                }
                _ => {}
            }

            if !matches!(key.code, KeyCode::Char('g')) {
                app.pending_g = false;
            }
        }
    }
}
