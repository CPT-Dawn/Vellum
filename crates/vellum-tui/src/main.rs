use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use image::DynamicImage;
use ratatui::layout::Rect;
use ratatui_image::{picker::Picker, protocol::StatefulProtocol, StatefulImage};
use serde_json::Value;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader as StdBufReader, Write};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::time::Duration as StdDuration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::{timeout, Duration};
use tracing::info;
use vellum_ipc::{AssignmentEntry, Request, RequestEnvelope, Response, ResponseEnvelope};

use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Terminal;

#[derive(Debug, Parser)]
#[command(name = "vellum-tui", about = "Vellum terminal client")]
struct Args {
    #[arg(long, value_name = "PATH")]
    socket: Option<PathBuf>,

    #[arg(long, value_name = "PATH")]
    images_dir: Option<PathBuf>,

    #[arg(long, value_name = "WIDTH")]
    monitor_width: Option<u32>,

    #[arg(long, value_name = "HEIGHT")]
    monitor_height: Option<u32>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Ui,
    Ping,
    Set {
        #[arg(value_name = "PATH")]
        path: PathBuf,

        #[arg(long, value_name = "NAME")]
        monitor: Option<String>,
    },
    Monitors,
    Assignments,
    Kill,
}

struct App {
    files: Vec<PathBuf>,
    selected: usize,
    image_root: PathBuf,
    picker: Picker,
    image_state: Option<StatefulProtocol>,
    preview_info: String,
    monitor_profile: MonitorProfile,
    status: String,
    pending_g: bool,
    show_help: bool,
    theme: UiTheme,
    socket_path: Option<PathBuf>,
    monitor_targets: Vec<String>,
    target_index: usize,
    assignments: Vec<AssignmentEntry>,
}

#[derive(Clone)]
struct MonitorProfile {
    width: u32,
    height: u32,
    source: String,
}

#[derive(Clone)]
struct UiTheme {
    chrome: Color,
    panel: Color,
    accent: Color,
    accent_alt: Color,
    text: Color,
    muted: Color,
    warn: Color,
}

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
        Command::Set { path, monitor } => {
            let socket_path = resolve_socket_path(args.socket)?;
            let response = send_request(
                &socket_path,
                Request::SetWallpaper {
                    path: path.display().to_string(),
                    monitor,
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
                    "Root: {}\nTotal images: {}\nCursor: {}\nSelected: {}\nMonitor: {}x{} ({:.2}:1) [{}]\nTarget Output: {}\nAssignments: {}\nPreview: {}\nDaemon: {}\n\nMode: Normal\nHint: press ? for full keymap",
                    app.image_root.display(),
                    app.files.len(),
                    app.selected,
                    selected,
                    app.monitor_profile.width,
                    app.monitor_profile.height,
                    app.monitor_profile.aspect_ratio(),
                    app.monitor_profile.source,
                    app.current_target_label(),
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
                    "h/j/k/l move  gg/G top/bottom  Ctrl-u/Ctrl-d page  Enter|Space apply  t cycle-target  m monitors  a assignments  r reload  ? help  q quit"
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
                KeyCode::Char('m') => app.fetch_monitors(),
                KeyCode::Char('a') => app.fetch_assignments(),
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

impl App {
    fn discover_files(
        image_root: PathBuf,
        monitor_profile: MonitorProfile,
        socket_path: Option<PathBuf>,
    ) -> Result<Self> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&image_root)
            .with_context(|| format!("failed to read image directory {}", image_root.display()))?
        {
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
        let mut app = Self {
            files,
            selected: 0,
            image_root,
            picker: Picker::from_query_stdio().unwrap_or_else(|_| Picker::from_fontsize((16, 8))),
            image_state: None,
            preview_info: "No image selected".to_string(),
            monitor_profile,
            status: "Normal | j/k move | gg/G edges | ? help | q quit".to_string(),
            pending_g: false,
            show_help: false,
            theme: UiTheme::arch_punk(),
            socket_path,
            monitor_targets: Vec::new(),
            target_index: 0,
            assignments: Vec::new(),
        };
        app.refresh_preview();
        app.refresh_monitor_targets();
        app.refresh_assignments_cache_silent();
        Ok(app)
    }

    fn select_next(&mut self) {
        if self.files.is_empty() {
            return;
        }

        self.selected = (self.selected + 1) % self.files.len();
        self.status = format!("Normal | moved to {}", self.selected);
        self.refresh_preview();
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
        self.status = format!("Normal | moved to {}", self.selected);
        self.refresh_preview();
    }

    fn select_first(&mut self) {
        if self.files.is_empty() {
            return;
        }

        self.selected = 0;
        self.status = "Normal | jumped to first image".to_string();
        self.refresh_preview();
    }

    fn select_last(&mut self) {
        if self.files.is_empty() {
            return;
        }

        self.selected = self.files.len().saturating_sub(1);
        self.status = "Normal | jumped to last image".to_string();
        self.refresh_preview();
    }

    fn select_page_down(&mut self, amount: usize) {
        if self.files.is_empty() {
            return;
        }

        self.selected = (self.selected + amount).min(self.files.len().saturating_sub(1));
        self.status = format!("Normal | page down {}", amount);
        self.refresh_preview();
    }

    fn select_page_up(&mut self, amount: usize) {
        if self.files.is_empty() {
            return;
        }

        self.selected = self.selected.saturating_sub(amount);
        self.status = format!("Normal | page up {}", amount);
        self.refresh_preview();
    }

    fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
        self.status = if self.show_help {
            "Help overlay enabled".to_string()
        } else {
            "Help overlay disabled".to_string()
        };
    }

    fn daemon_status(&self) -> &'static str {
        if self.socket_path.is_some() {
            "online-configured"
        } else {
            "offline"
        }
    }

    fn selected_file_name(&self) -> Option<&str> {
        self.files
            .get(self.selected)
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
    }

    fn refresh_preview(&mut self) {
        let Some(path) = self.files.get(self.selected) else {
            self.image_state = None;
            self.preview_info = "No image found in directory".to_string();
            return;
        };

        match load_image(path) {
            Ok(image) => {
                let dimensions = (image.width(), image.height());
                self.image_state = Some(self.picker.new_resize_protocol(image));
                self.preview_info = format!("Loaded {}x{}", dimensions.0, dimensions.1);
            }
            Err(err) => {
                self.image_state = None;
                self.preview_info = format!("Failed to load: {err}");
            }
        }
    }

    fn reload_files(&mut self) {
        let mut files = Vec::new();
        let entries = std::fs::read_dir(&self.image_root);
        let Ok(entries) = entries else {
            self.status = format!(
                "Failed to reload: cannot read {}",
                self.image_root.display()
            );
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && is_supported_image_path(&path) {
                files.push(path);
            }
        }

        files.sort();
        self.files = files;
        if self.selected >= self.files.len() {
            self.selected = self.files.len().saturating_sub(1);
        }
        self.status = format!("Reloaded {} images", self.files.len());
        self.refresh_preview();
    }

    fn apply_selected_wallpaper(&mut self) {
        let Some(path) = self.files.get(self.selected).cloned() else {
            self.status = "No selected image to apply".to_string();
            return;
        };

        let monitor = self.current_target_name();

        let response = self.send_daemon_request(Request::SetWallpaper {
            path: path.display().to_string(),
            monitor,
        });

        match response {
            Ok(Response::Ok) => {
                self.status = format!("Applied wallpaper: {}", path.display());
                self.refresh_assignments_cache_silent();
            }
            Ok(Response::Error { message }) => {
                self.status = format!("Daemon rejected wallpaper: {message}");
            }
            Ok(other) => {
                self.status = format!("Unexpected daemon response: {other:?}");
            }
            Err(err) => {
                self.status = format!("Apply failed: {err:#}");
            }
        }
    }

    fn fetch_monitors(&mut self) {
        let response = self.send_daemon_request(Request::GetMonitors);
        match response {
            Ok(Response::Monitors { names }) => {
                if names.is_empty() {
                    self.status = "Daemon reported no monitors".to_string();
                } else {
                    self.monitor_targets = names.clone();
                    if self.target_index > self.monitor_targets.len() {
                        self.target_index = 0;
                    }
                    self.status = format!("Monitors: {}", names.join(", "));
                }
            }
            Ok(Response::Error { message }) => {
                self.status = format!("Monitor query failed: {message}");
            }
            Ok(other) => {
                self.status = format!("Unexpected monitor response: {other:?}");
            }
            Err(err) => {
                self.status = format!("Monitor query failed: {err:#}");
            }
        }
    }

    fn fetch_assignments(&mut self) {
        let response = self.send_daemon_request(Request::GetAssignments);
        match response {
            Ok(Response::Assignments { entries }) => {
                self.assignments = entries.clone();
                if entries.is_empty() {
                    self.status = "No wallpaper assignments recorded yet".to_string();
                } else {
                    let compact = entries
                        .iter()
                        .map(|entry| {
                            let monitor =
                                entry.monitor.clone().unwrap_or_else(|| "all".to_string());
                            let file = PathBuf::from(&entry.path)
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("<path>")
                                .to_string();
                            format!("{monitor}:{file}")
                        })
                        .collect::<Vec<_>>()
                        .join(" | ");
                    self.status = format!("Assignments {compact}");
                }
            }
            Ok(Response::Error { message }) => {
                self.status = format!("Assignments query failed: {message}");
            }
            Ok(other) => {
                self.status = format!("Unexpected assignments response: {other:?}");
            }
            Err(err) => {
                self.status = format!("Assignments query failed: {err:#}");
            }
        }
    }

    fn refresh_monitor_targets(&mut self) {
        let response = self.send_daemon_request(Request::GetMonitors);
        if let Ok(Response::Monitors { names }) = response {
            self.monitor_targets = names;
            if self.target_index > self.monitor_targets.len() {
                self.target_index = 0;
            }
        }
    }

    fn cycle_monitor_target(&mut self) {
        if self.monitor_targets.is_empty() {
            self.refresh_monitor_targets();
        }

        let total_slots = self.monitor_targets.len() + 1;
        if total_slots == 0 {
            self.target_index = 0;
            self.status = "Target output: all".to_string();
            return;
        }

        self.target_index = (self.target_index + 1) % total_slots;
        self.status = format!("Target output: {}", self.current_target_label());
    }

    fn current_target_name(&self) -> Option<String> {
        if self.target_index == 0 {
            None
        } else {
            self.monitor_targets.get(self.target_index - 1).cloned()
        }
    }

    fn current_target_label(&self) -> String {
        self.current_target_name()
            .unwrap_or_else(|| "all outputs".to_string())
    }

    fn send_daemon_request(&self, request: Request) -> Result<Response> {
        let socket_path = self
            .socket_path
            .as_ref()
            .context("daemon socket is not configured in this environment")?;

        send_request_blocking(socket_path, request)
    }

    fn refresh_assignments_cache_silent(&mut self) {
        let response = self.send_daemon_request(Request::GetAssignments);
        if let Ok(Response::Assignments { entries }) = response {
            self.assignments = entries;
        }
    }

    fn assignments_overview(&self) -> String {
        if self.assignments.is_empty() {
            return "none".to_string();
        }

        let shown = self
            .assignments
            .iter()
            .take(3)
            .map(|entry| {
                let monitor = entry.monitor.as_deref().unwrap_or("all");
                let file = PathBuf::from(&entry.path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("<path>")
                    .to_string();
                format!("{monitor}:{file}")
            })
            .collect::<Vec<_>>()
            .join(" | ");

        if self.assignments.len() > 3 {
            format!("{shown} | +{} more", self.assignments.len() - 3)
        } else {
            shown
        }
    }
}

impl UiTheme {
    fn arch_punk() -> Self {
        Self {
            chrome: Color::Rgb(14, 17, 23),
            panel: Color::Rgb(58, 74, 112),
            accent: Color::Rgb(129, 207, 255),
            accent_alt: Color::Rgb(255, 92, 143),
            text: Color::Rgb(196, 206, 240),
            muted: Color::Rgb(156, 168, 198),
            warn: Color::Rgb(255, 158, 100),
        }
    }
}

fn default_image_root() -> PathBuf {
    dirs::picture_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join("Pictures")))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn load_image(path: &Path) -> Result<DynamicImage> {
    image::ImageReader::open(path)
        .with_context(|| format!("failed to open image {}", path.display()))?
        .decode()
        .with_context(|| format!("failed to decode image {}", path.display()))
}

fn fit_aspect_rect(area: Rect, target_width: u32, target_height: u32) -> Rect {
    if area.width < 3 || area.height < 3 || target_width == 0 || target_height == 0 {
        return area;
    }

    let area_w = u32::from(area.width);
    let area_h = u32::from(area.height);

    let (width, height) =
        if area_w.saturating_mul(target_height) > area_h.saturating_mul(target_width) {
            let width = area_h.saturating_mul(target_width) / target_height;
            (width.max(1), area_h)
        } else {
            let height = area_w.saturating_mul(target_height) / target_width;
            (area_w, height.max(1))
        };

    let width_u16 = u16::try_from(width).unwrap_or(area.width);
    let height_u16 = u16::try_from(height).unwrap_or(area.height);
    let x = area.x + area.width.saturating_sub(width_u16) / 2;
    let y = area.y + area.height.saturating_sub(height_u16) / 2;

    Rect::new(x, y, width_u16, height_u16)
}

impl MonitorProfile {
    fn resolve(width: Option<u32>, height: Option<u32>) -> Self {
        if let (Some(width), Some(height)) = (width, height) {
            if width > 0 && height > 0 {
                return Self {
                    width,
                    height,
                    source: "cli override".to_string(),
                };
            }
        }

        if let Some(profile) = detect_hyprland_monitor() {
            return profile;
        }

        if let Some(profile) = detect_wlr_randr_monitor() {
            return profile;
        }

        if let Ok((cols, rows)) = crossterm::terminal::size() {
            if cols > 0 && rows > 0 {
                return Self {
                    width: u32::from(cols),
                    height: u32::from(rows),
                    source: "terminal size fallback".to_string(),
                };
            }
        }

        Self {
            width: 1920,
            height: 1080,
            source: "default 1080p fallback".to_string(),
        }
    }

    fn aspect_ratio(&self) -> f32 {
        self.width as f32 / self.height as f32
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

fn detect_hyprland_monitor() -> Option<MonitorProfile> {
    let value = run_json_command("hyprctl", &["monitors", "-j"])?;
    let monitors = value.as_array()?;

    let selected = monitors
        .iter()
        .find(|monitor| monitor.get("focused").and_then(Value::as_bool) == Some(true))
        .or_else(|| monitors.first())?;

    let width = selected.get("width").and_then(Value::as_u64)? as u32;
    let height = selected.get("height").and_then(Value::as_u64)? as u32;
    if width == 0 || height == 0 {
        return None;
    }

    let name = selected
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("hyprland")
        .to_string();

    Some(MonitorProfile {
        width,
        height,
        source: format!("hyprctl:{name}"),
    })
}

fn detect_wlr_randr_monitor() -> Option<MonitorProfile> {
    let value = run_json_command("wlr-randr", &["--json"])?;
    let (width, height) = find_resolution_pair(&value)?;
    if width == 0 || height == 0 {
        return None;
    }

    Some(MonitorProfile {
        width,
        height,
        source: "wlr-randr".to_string(),
    })
}

fn find_resolution_pair(value: &Value) -> Option<(u32, u32)> {
    match value {
        Value::Object(map) => {
            if let (Some(width), Some(height)) = (map.get("width"), map.get("height")) {
                let width = width.as_u64()? as u32;
                let height = height.as_u64()? as u32;
                if width > 0 && height > 0 {
                    return Some((width, height));
                }
            }

            for child in map.values() {
                if let Some(pair) = find_resolution_pair(child) {
                    return Some(pair);
                }
            }
            None
        }
        Value::Array(values) => {
            for child in values {
                if let Some(pair) = find_resolution_pair(child) {
                    return Some(pair);
                }
            }
            None
        }
        _ => None,
    }
}

fn is_supported_image_path(path: &Path) -> bool {
    match path.extension().and_then(OsStr::to_str) {
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

fn resolve_socket_path_optional(explicit: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path);
    }

    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").ok()?;
    Some(PathBuf::from(runtime_dir).join("vellum.sock"))
}

fn send_request_blocking(socket_path: &PathBuf, request: Request) -> Result<Response> {
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

fn print_assignment_entries(entries: &[AssignmentEntry]) {
    if entries.is_empty() {
        println!("no assignments recorded");
        return;
    }

    for entry in entries {
        let monitor = entry.monitor.as_deref().unwrap_or("all");
        println!("{monitor} -> {}", entry.path);
    }
}
