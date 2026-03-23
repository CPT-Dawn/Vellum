use std::{
    io,
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};
use serde::Deserialize;
use serde_json::Value;
use tokio::{process::Command, task::JoinHandle};
use vellum_core::{VellumServer, VellumServerConfig};

/// UI refresh cadence in milliseconds.
const TICK_RATE_MS: u64 = 33;
/// Minimum interval between automatic monitor re-probes.
const MONITOR_REFRESH_INTERVAL: Duration = Duration::from_secs(10);

/// Runtime monitor snapshot used by the TUI monitor pane.
#[derive(Debug, Clone)]
struct MonitorEntry {
    /// Connector or output name as reported by the compositor.
    name: String,
    /// Width in physical pixels.
    width: u32,
    /// Height in physical pixels.
    height: u32,
    /// Output position on the global desktop space.
    x: i32,
    /// Output position on the global desktop space.
    y: i32,
    /// Whether this monitor is the currently focused output.
    focused: bool,
}

/// Native backend lifecycle state tracked by the TUI.
#[derive(Debug)]
struct BackendRuntime {
    /// Optional running daemon task handle.
    task: Option<JoinHandle<Result<(), String>>>,
    /// Namespace used by the native server instance.
    namespace: String,
    /// Last backend runtime error captured from the task.
    last_error: Option<String>,
}

impl Default for BackendRuntime {
    /// Builds default backend state.
    fn default() -> Self {
        Self {
            task: None,
            namespace: String::from("vellum-tui"),
            last_error: None,
        }
    }
}

/// Application-level state for the initial Vellum TUI shell.
#[derive(Debug)]
struct AppState {
    /// Whether the event loop should exit.
    should_quit: bool,
    /// Status message rendered in the footer.
    status: String,
    /// Number of frames rendered in this session.
    frame_count: u64,
    /// Timestamp of the last frame.
    last_frame: Instant,
    /// Native backend runtime metadata.
    backend: BackendRuntime,
    /// Latest discovered monitor state.
    monitors: Vec<MonitorEntry>,
    /// Timestamp for most recent monitor refresh.
    last_monitor_refresh: Option<Instant>,
    /// Requests an immediate monitor refresh on the next tick.
    needs_monitor_refresh: bool,
}

impl Default for AppState {
    /// Builds the default UI state used at startup.
    fn default() -> Self {
        Self {
            should_quit: false,
            status: String::from("Bootstrapping native backend"),
            frame_count: 0,
            last_frame: Instant::now(),
            backend: BackendRuntime::default(),
            monitors: Vec::new(),
            last_monitor_refresh: None,
            needs_monitor_refresh: true,
        }
    }
}

/// Terminal setup guard that restores screen state on drop.
struct TerminalGuard;

impl Drop for TerminalGuard {
    /// Restores terminal raw mode and alternate screen settings.
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, LeaveAlternateScreen);
    }
}

/// Launches the async runtime and starts the TUI loop.
#[tokio::main(flavor = "multi_thread")]
async fn main() -> io::Result<()> {
    run_app().await
}

/// Initializes terminal mode and executes the draw/input loop.
async fn run_app() -> io::Result<()> {
    enable_raw_mode()?;
    let _guard = TerminalGuard;

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut state = AppState::default();
    start_native_backend(&mut state);
    let mut tick = tokio::time::interval(Duration::from_millis(TICK_RATE_MS));

    while !state.should_quit {
        tick.tick().await;
        handle_input(&mut state)?;
        refresh_monitors_if_due(&mut state).await;
        poll_backend_status(&mut state).await;
        terminal.draw(|frame| draw_ui(frame, &state))?;
        state.frame_count = state.frame_count.saturating_add(1);
        state.last_frame = Instant::now();
    }

    Ok(())
}

/// Reads keyboard events and mutates app state.
fn handle_input(state: &mut AppState) -> io::Result<()> {
    while event::poll(Duration::from_millis(0))? {
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    state.status = String::from("Exiting Vellum");
                    state.should_quit = true;
                }
                KeyCode::Char('r') => {
                    state.status = String::from("Manual monitor refresh requested");
                    state.needs_monitor_refresh = true;
                }
                KeyCode::Char('b') => {
                    start_native_backend(state);
                }
                KeyCode::Char('h') => {
                    state.status =
                        String::from("Help: q/Esc quit, r refresh monitors, b start backend");
                }
                _ => {}
            }
        }
    }

    Ok(())
}

/// Starts the native Vellum backend daemon in a dedicated blocking task.
fn start_native_backend(state: &mut AppState) {
    if state
        .backend
        .task
        .as_ref()
        .is_some_and(|task| !task.is_finished())
    {
        state.status = String::from("Native backend already running");
        return;
    }

    let namespace = state.backend.namespace.clone();
    state.status = format!("Starting native backend in namespace '{namespace}'");
    state.backend.last_error = None;
    state.backend.task = Some(tokio::task::spawn_blocking(move || {
        let config = VellumServerConfig {
            namespace,
            ..VellumServerConfig::default()
        };
        let server = VellumServer::new(config);
        server.run().map_err(|err| err.to_string())
    }));
}

/// Polls backend task completion and updates runtime status text.
async fn poll_backend_status(state: &mut AppState) {
    if state
        .backend
        .task
        .as_ref()
        .is_none_or(|task| !task.is_finished())
    {
        return;
    }

    let task = state.backend.task.take().expect("backend task must exist");
    match task.await {
        Ok(Ok(())) => {
            state.status = String::from("Native backend exited gracefully");
        }
        Ok(Err(err)) => {
            state.backend.last_error = Some(err.clone());
            state.status = format!("Native backend error: {err}");
        }
        Err(err) => {
            let msg = err.to_string();
            state.backend.last_error = Some(msg.clone());
            state.status = format!("Backend task join error: {msg}");
        }
    }
}

/// Refreshes monitor information when a manual or timed refresh is due.
async fn refresh_monitors_if_due(state: &mut AppState) {
    let timed_due = state
        .last_monitor_refresh
        .is_none_or(|instant| instant.elapsed() >= MONITOR_REFRESH_INTERVAL);

    if !state.needs_monitor_refresh && !timed_due {
        return;
    }

    match discover_monitors().await {
        Ok(monitors) => {
            let count = monitors.len();
            state.monitors = monitors;
            state.last_monitor_refresh = Some(Instant::now());
            state.needs_monitor_refresh = false;
            state.status = format!("Detected {count} monitor(s) from compositor");
        }
        Err(err) => {
            state.needs_monitor_refresh = false;
            state.status = format!("Monitor discovery failed: {err}");
        }
    }
}

/// Performs monitor discovery using compositor-native tools.
async fn discover_monitors() -> Result<Vec<MonitorEntry>, String> {
    match probe_hyprctl_monitors().await {
        Ok(monitors) if !monitors.is_empty() => return Ok(monitors),
        Ok(_) => {}
        Err(_err) => {}
    }

    probe_wlr_randr_monitors().await
}

/// Hyprland monitor representation from `hyprctl monitors -j`.
#[derive(Debug, Deserialize)]
struct HyprMonitor {
    /// Output name.
    name: String,
    /// Width in pixels.
    width: u32,
    /// Height in pixels.
    height: u32,
    /// X position.
    x: i32,
    /// Y position.
    y: i32,
    /// Focus status.
    #[serde(default)]
    focused: bool,
}

/// Attempts monitor discovery via `hyprctl monitors -j`.
async fn probe_hyprctl_monitors() -> Result<Vec<MonitorEntry>, String> {
    let json = command_json("hyprctl", &["monitors", "-j"]).await?;
    let monitors: Vec<HyprMonitor> =
        serde_json::from_value(json).map_err(|err| format!("invalid hyprctl JSON: {err}"))?;

    Ok(monitors
        .into_iter()
        .map(|monitor| MonitorEntry {
            name: monitor.name,
            width: monitor.width,
            height: monitor.height,
            x: monitor.x,
            y: monitor.y,
            focused: monitor.focused,
        })
        .collect())
}

/// Attempts monitor discovery via `wlr-randr --json`.
async fn probe_wlr_randr_monitors() -> Result<Vec<MonitorEntry>, String> {
    let json = command_json("wlr-randr", &["--json"]).await?;
    parse_wlr_randr_monitors(json)
}

/// Executes a command and parses stdout as JSON.
async fn command_json(binary: &str, args: &[&str]) -> Result<Value, String> {
    let output = Command::new(binary)
        .args(args)
        .output()
        .await
        .map_err(|err| format!("{binary} execution failed: {err}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "{binary} returned status {}: {}",
            output.status,
            stderr.trim()
        ));
    }

    serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("{binary} produced invalid JSON: {err}"))
}

/// Parses `wlr-randr --json` payload into monitor entries.
fn parse_wlr_randr_monitors(payload: Value) -> Result<Vec<MonitorEntry>, String> {
    let Value::Array(outputs) = payload else {
        return Err(String::from("wlr-randr JSON root must be an array"));
    };

    let monitors = outputs
        .into_iter()
        .filter_map(|output| {
            let obj = output.as_object()?;
            let name = obj.get("name")?.as_str()?.to_owned();

            let (width, height) = extract_wlr_dimensions(obj)?;
            let x = obj
                .get("x")
                .and_then(Value::as_i64)
                .and_then(|value| i32::try_from(value).ok())
                .unwrap_or(0);
            let y = obj
                .get("y")
                .and_then(Value::as_i64)
                .and_then(|value| i32::try_from(value).ok())
                .unwrap_or(0);

            Some(MonitorEntry {
                name,
                width,
                height,
                x,
                y,
                focused: false,
            })
        })
        .collect::<Vec<_>>();

    Ok(monitors)
}

/// Extracts active mode dimensions from a single wlr-randr output object.
fn extract_wlr_dimensions(object: &serde_json::Map<String, Value>) -> Option<(u32, u32)> {
    if let Some(mode) = object.get("current_mode").and_then(Value::as_object) {
        let width = mode
            .get("width")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())?;
        let height = mode
            .get("height")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())?;
        return Some((width, height));
    }

    object
        .get("modes")
        .and_then(Value::as_array)
        .and_then(|modes| {
            modes
                .iter()
                .find(|mode| mode.get("current").and_then(Value::as_bool) == Some(true))
        })
        .and_then(Value::as_object)
        .and_then(|mode| {
            let width = mode
                .get("width")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())?;
            let height = mode
                .get("height")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())?;
            Some((width, height))
        })
}

/// Renders the initial multi-pane Ratatui layout.
fn draw_ui(frame: &mut Frame<'_>, state: &AppState) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(frame.area());

    draw_header(frame, root[0]);
    draw_body(frame, root[1], state);
    draw_footer(frame, root[2], state);
}

/// Draws the top status header.
fn draw_header(frame: &mut Frame<'_>, area: Rect) {
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            "VELLUM",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  native wallpaper control surface"),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .title(" Session "),
    );
    frame.render_widget(title, area);
}

/// Draws the three core panes for the Phase 1 shell.
fn draw_body(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(area);

    let file_browser =
        Paragraph::new("File Browser\n- phase scaffold\n- no filesystem binding yet").block(
            Block::default()
                .title(" Browser ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        );

    let monitor_lines = if state.monitors.is_empty() {
        String::from("Monitor Preview\n- no monitor data yet\n- press r to refresh")
    } else {
        let mut lines = String::from("Monitor Preview\n");
        for monitor in &state.monitors {
            let focus = if monitor.focused { " *" } else { "" };
            let line = format!(
                "- {}: {}x{} @ ({}, {}){}\n",
                monitor.name, monitor.width, monitor.height, monitor.x, monitor.y, focus
            );
            lines.push_str(&line);
        }
        lines
    };

    let monitor_preview = Paragraph::new(monitor_lines).block(
        Block::default()
            .title(" Monitor ")
            .borders(Borders::ALL)
            .border_type(BorderType::Thick),
    );

    let backend_state = if state
        .backend
        .task
        .as_ref()
        .is_some_and(|task| !task.is_finished())
    {
        "running"
    } else {
        "stopped"
    };
    let backend_message = if let Some(error) = &state.backend.last_error {
        format!(
            "Backend\n- status: {backend_state}\n- namespace: {}\n- last error: {error}",
            state.backend.namespace
        )
    } else {
        format!(
            "Backend\n- status: {backend_state}\n- namespace: {}\n- integrated via vellum-core",
            state.backend.namespace
        )
    };

    let transition_panel = Paragraph::new(backend_message).block(
        Block::default()
            .title(" Native Backend ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded),
    );

    frame.render_widget(file_browser, columns[0]);
    frame.render_widget(monitor_preview, columns[1]);
    frame.render_widget(transition_panel, columns[2]);
}

/// Draws footer hints and runtime diagnostics.
fn draw_footer(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let frame_age_ms = state.last_frame.elapsed().as_millis();
    let diagnostics = format!(
        "{} | frames={} | frame_age={}ms | q/Esc quit | r refresh | b start backend",
        state.status, state.frame_count, frame_age_ms
    );
    let footer = Paragraph::new(diagnostics).block(
        Block::default()
            .title(" Controls ")
            .borders(Borders::ALL)
            .border_type(BorderType::Double),
    );
    frame.render_widget(footer, area);
}
