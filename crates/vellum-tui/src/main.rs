use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use std::path::PathBuf;
use std::time::Duration as StdDuration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::{timeout, Duration};
use tracing::info;
use vellum_ipc::{Request, RequestEnvelope, Response, ResponseEnvelope};

use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Terminal;

#[derive(Debug, Parser)]
#[command(name = "vellum-tui", about = "Vellum terminal client")]
struct Args {
    #[arg(long, value_name = "PATH")]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Ui,
    Ping,
    Kill,
}

struct App {
    files: Vec<PathBuf>,
    selected: usize,
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

    let command = args.command.unwrap_or(Command::Ui);

    match command {
        Command::Ui => run_ui(),
        Command::Ping => {
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
        Command::Kill => {
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

fn run_ui() -> Result<()> {
    let mut app = App::discover_files()?;

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
                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Percentage(30),
                        Constraint::Percentage(30),
                        Constraint::Percentage(40),
                    ])
                    .split(frame.area());

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
                            .title("Browser")
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(Color::Rgb(65, 72, 104))),
                    )
                    .highlight_style(
                        Style::default()
                            .fg(Color::Rgb(122, 162, 247))
                            .add_modifier(Modifier::BOLD),
                    )
                    .highlight_symbol("> ");

                frame.render_stateful_widget(browser, chunks[0], &mut list_state);

                let selected = app.selected_file_name().unwrap_or("None selected");
                let metadata = Paragraph::new(format!(
                    "Total images: {}\nSelected index: {}\nSelected: {}\n\nKeys:\n- j/k or arrows: move\n- q: quit\n- p: ping daemon\n- x: request daemon shutdown",
                    app.files.len(),
                    app.selected,
                    selected
                ))
                .block(
                    Block::default()
                        .title("Metadata")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Rgb(65, 72, 104))),
                )
                .style(Style::default().fg(Color::Rgb(169, 177, 214)));

                frame.render_widget(metadata, chunks[1]);

                let preview = Paragraph::new(
                    "Preview pipeline placeholder.\n\nNext step will integrate ratatui-image with Kitty graphics and Sixel fallback.",
                )
                .block(
                    Block::default()
                        .title("Preview")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Rgb(65, 72, 104))),
                )
                .style(Style::default().fg(Color::Rgb(192, 202, 245)));

                frame.render_widget(preview, chunks[2]);
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
                KeyCode::Char('q') => return Ok(()),
                KeyCode::Down | KeyCode::Char('j') => app.select_next(),
                KeyCode::Up | KeyCode::Char('k') => app.select_previous(),
                KeyCode::Char('p') => {
                    info!("UI requested daemon ping shortcut");
                }
                KeyCode::Char('x') => {
                    info!("UI requested daemon shutdown shortcut");
                }
                _ => {}
            }
        }
    }
}

impl App {
    fn discover_files() -> Result<Self> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(".").context("failed to read current directory")? {
            let entry = entry.context("failed to read directory entry")?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            if is_supported_image_path(&path) {
                files.push(path);
            }
        }

        files.sort();
        Ok(Self { files, selected: 0 })
    }

    fn select_next(&mut self) {
        if self.files.is_empty() {
            return;
        }

        self.selected = (self.selected + 1) % self.files.len();
    }

    fn select_previous(&mut self) {
        if self.files.is_empty() {
            return;
        }

        self.selected = if self.selected == 0 {
            self.files.len().saturating_sub(1)
        } else {
            self.selected.saturating_sub(1)
        };
    }

    fn selected_file_name(&self) -> Option<&str> {
        self.files
            .get(self.selected)
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
    }
}

fn is_supported_image_path(path: &PathBuf) -> bool {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => matches!(
            ext.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "webp" | "bmp"
        ),
        None => false,
    }
}

fn resolve_socket_path(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }

    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .context("XDG_RUNTIME_DIR is not set; pass --socket explicitly")?;
    Ok(PathBuf::from(runtime_dir).join("vellum.sock"))
}

async fn send_request(socket_path: &PathBuf, request: Request) -> Result<Response> {
    let stream = timeout(Duration::from_secs(2), UnixStream::connect(socket_path))
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
    timeout(Duration::from_secs(2), reader.read_line(&mut line))
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
