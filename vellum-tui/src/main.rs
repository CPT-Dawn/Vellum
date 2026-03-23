use std::{
    fs, io,
    num::NonZeroU8,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use common::ipc::{
    Answer, Coord, ImageRequestBuilder, ImgSend, IpcSocket, PixelFormat, Position, RequestSend,
    Transition, TransitionType,
};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use image::ImageReader;
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

/// Available transition easings shown in the transition pane.
const EASING_PRESETS: [&str; 4] = ["linear", "ease-in", "ease-out", "ease-in-out"];
/// Available transition effects shown in the transition pane.
const TRANSITION_EFFECTS: [&str; 4] = ["simple", "fade", "wipe", "grow"];

/// Active pane focus within the three-column layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaneFocus {
    /// File browser pane.
    Browser,
    /// Monitor preview pane.
    Monitor,
    /// Transition settings pane.
    Transition,
}

impl PaneFocus {
    /// Selects the next pane in the cycle.
    fn next(self) -> Self {
        match self {
            Self::Browser => Self::Monitor,
            Self::Monitor => Self::Transition,
            Self::Transition => Self::Browser,
        }
    }

    /// Selects the previous pane in the cycle.
    fn prev(self) -> Self {
        match self {
            Self::Browser => Self::Transition,
            Self::Monitor => Self::Browser,
            Self::Transition => Self::Monitor,
        }
    }

    /// Returns a human-readable pane name.
    fn as_str(self) -> &'static str {
        match self {
            Self::Browser => "Browser",
            Self::Monitor => "Monitor",
            Self::Transition => "Transition",
        }
    }
}

/// File-browser row metadata used in the scaffold list view.
#[derive(Debug, Clone)]
struct BrowserEntry {
    /// Display name shown in the browser list.
    name: String,
    /// Absolute path represented by this row.
    path: PathBuf,
    /// Whether this row represents a directory.
    is_dir: bool,
    /// Whether this row represents the synthetic parent directory action.
    is_parent: bool,
}

/// Editable transition settings displayed in the right pane.
#[derive(Debug, Clone)]
struct TransitionState {
    /// Current transition duration in milliseconds.
    duration_ms: u32,
    /// Current target frame rate.
    fps: u16,
    /// Selected easing preset index.
    easing_idx: usize,
    /// Selected effect preset index.
    effect_idx: usize,
    /// Selected field index for keyboard editing.
    selected_field: usize,
}

impl Default for TransitionState {
    /// Builds default transition settings for the phase scaffold.
    fn default() -> Self {
        Self {
            duration_ms: 750,
            fps: 60,
            easing_idx: 3,
            effect_idx: 1,
            selected_field: 0,
        }
    }
}

impl TransitionState {
    /// Moves field selection upward.
    fn select_prev_field(&mut self) {
        self.selected_field = self.selected_field.saturating_sub(1);
    }

    /// Moves field selection downward.
    fn select_next_field(&mut self) {
        self.selected_field = (self.selected_field + 1).min(3);
    }

    /// Increases selected field value.
    fn increase_selected(&mut self) {
        match self.selected_field {
            0 => self.duration_ms = (self.duration_ms + 25).min(15_000),
            1 => self.fps = (self.fps + 5).min(240),
            2 => self.easing_idx = (self.easing_idx + 1) % EASING_PRESETS.len(),
            3 => self.effect_idx = (self.effect_idx + 1) % TRANSITION_EFFECTS.len(),
            _ => {}
        }
    }

    /// Decreases selected field value.
    fn decrease_selected(&mut self) {
        match self.selected_field {
            0 => self.duration_ms = self.duration_ms.saturating_sub(25).max(25),
            1 => self.fps = self.fps.saturating_sub(5).max(10),
            2 => {
                self.easing_idx =
                    (self.easing_idx + EASING_PRESETS.len() - 1) % EASING_PRESETS.len();
            }
            3 => {
                self.effect_idx =
                    (self.effect_idx + TRANSITION_EFFECTS.len() - 1) % TRANSITION_EFFECTS.len();
            }
            _ => {}
        }
    }
}

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
    /// Active pane focus.
    focus: PaneFocus,
    /// Browser rows for phase scaffold.
    browser_entries: Vec<BrowserEntry>,
    /// Full unfiltered browser rows for current directory.
    browser_all_entries: Vec<BrowserEntry>,
    /// Current directory shown in the browser pane.
    browser_dir: PathBuf,
    /// Fuzzy query used to filter browser entries.
    browser_query: String,
    /// Selected browser row index.
    browser_selected: usize,
    /// Selected monitor index in preview pane.
    monitor_selected: usize,
    /// Editable transition controls.
    transition: TransitionState,
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
            focus: PaneFocus::Browser,
            browser_entries: Vec::new(),
            browser_all_entries: Vec::new(),
            browser_dir: preferred_initial_browser_dir(),
            browser_query: String::new(),
            browser_selected: 0,
            monitor_selected: 0,
            transition: TransitionState::default(),
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
    if let Err(err) = reload_browser_directory(&mut state) {
        state.status = format!("Browser initialization failed: {err}");
    }
    start_native_backend(&mut state);
    let mut tick = tokio::time::interval(Duration::from_millis(TICK_RATE_MS));

    while !state.should_quit {
        tick.tick().await;
        handle_input(&mut state).await?;
        refresh_monitors_if_due(&mut state).await;
        poll_backend_status(&mut state).await;
        terminal.draw(|frame| draw_ui(frame, &state))?;
        state.frame_count = state.frame_count.saturating_add(1);
        state.last_frame = Instant::now();
    }

    Ok(())
}

/// Reads keyboard events and mutates app state.
async fn handle_input(state: &mut AppState) -> io::Result<()> {
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
                    state.status = String::from(
                        "Help: Tab pane switch, arrows navigate, Enter apply/open, / filter",
                    );
                }
                KeyCode::Char('u') => {
                    if state.focus == PaneFocus::Browser {
                        if let Some(parent) = state.browser_dir.parent().map(Path::to_path_buf) {
                            state.browser_dir = parent;
                            if let Err(err) = reload_browser_directory(state) {
                                state.status = format!("Failed to open parent directory: {err}");
                            }
                        }
                    }
                }
                KeyCode::Tab => {
                    state.focus = state.focus.next();
                    state.status = format!("Focus moved to {} pane", state.focus.as_str());
                }
                KeyCode::BackTab => {
                    state.focus = state.focus.prev();
                    state.status = format!("Focus moved to {} pane", state.focus.as_str());
                }
                KeyCode::Up => match state.focus {
                    PaneFocus::Browser => {
                        state.browser_selected = state.browser_selected.saturating_sub(1);
                    }
                    PaneFocus::Monitor => {
                        state.monitor_selected = state.monitor_selected.saturating_sub(1);
                    }
                    PaneFocus::Transition => {
                        state.transition.select_prev_field();
                    }
                },
                KeyCode::Down => match state.focus {
                    PaneFocus::Browser => {
                        let max_idx = state.browser_entries.len().saturating_sub(1);
                        state.browser_selected = (state.browser_selected + 1).min(max_idx);
                    }
                    PaneFocus::Monitor => {
                        let max_idx = state.monitors.len().saturating_sub(1);
                        state.monitor_selected = (state.monitor_selected + 1).min(max_idx);
                    }
                    PaneFocus::Transition => {
                        state.transition.select_next_field();
                    }
                },
                KeyCode::Left => {
                    if state.focus == PaneFocus::Transition {
                        state.transition.decrease_selected();
                    }
                }
                KeyCode::Right => {
                    if state.focus == PaneFocus::Transition {
                        state.transition.increase_selected();
                    }
                }
                KeyCode::Backspace => {
                    if state.focus == PaneFocus::Browser && !state.browser_query.is_empty() {
                        state.browser_query.pop();
                        apply_browser_filter(state);
                    }
                }
                KeyCode::Enter => {
                    if state.focus == PaneFocus::Browser {
                        activate_browser_selection(state).await;
                    }
                }
                KeyCode::Char(c) => {
                    if state.focus == PaneFocus::Browser && !c.is_control() {
                        state.browser_query.push(c);
                        apply_browser_filter(state);
                    }
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
            state.monitor_selected = state
                .monitor_selected
                .min(state.monitors.len().saturating_sub(1));
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

/// Chooses the startup directory for the file browser.
fn preferred_initial_browser_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        let pictures = PathBuf::from(home).join("Pictures");
        if pictures.is_dir() {
            return pictures;
        }
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Reloads filesystem entries for the current browser directory.
fn reload_browser_directory(state: &mut AppState) -> io::Result<()> {
    let mut entries = Vec::new();

    if let Some(parent) = state.browser_dir.parent() {
        entries.push(BrowserEntry {
            name: String::from(".."),
            path: parent.to_path_buf(),
            is_dir: true,
            is_parent: true,
        });
    }

    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for dir_entry in fs::read_dir(&state.browser_dir)? {
        let entry = dir_entry?;
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();
        let meta = entry.metadata()?;

        if meta.is_dir() {
            dirs.push(BrowserEntry {
                name: format!("{file_name}/"),
                path,
                is_dir: true,
                is_parent: false,
            });
            continue;
        }

        if meta.is_file() && is_supported_image_path(&path) {
            files.push(BrowserEntry {
                name: file_name,
                path,
                is_dir: false,
                is_parent: false,
            });
        }
    }

    dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    entries.extend(dirs);
    entries.extend(files);

    state.browser_all_entries = entries;
    apply_browser_filter(state);
    Ok(())
}

/// Applies fuzzy filtering to browser rows based on the active query.
fn apply_browser_filter(state: &mut AppState) {
    if state.browser_query.is_empty() {
        state.browser_entries = state.browser_all_entries.clone();
    } else {
        let mut scored = state
            .browser_all_entries
            .iter()
            .cloned()
            .filter_map(|entry| {
                fuzzy_score(&state.browser_query, &entry.name).map(|score| (score, entry))
            })
            .collect::<Vec<_>>();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
        state.browser_entries = scored.into_iter().map(|(_, entry)| entry).collect();
    }

    state.browser_selected = state
        .browser_selected
        .min(state.browser_entries.len().saturating_sub(1));
}

/// Computes a lightweight subsequence-based fuzzy match score.
fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    let query = query.to_ascii_lowercase();
    let candidate = candidate.to_ascii_lowercase();

    let mut q_chars = query.chars();
    let mut current = q_chars.next()?;
    let mut score = 0_i32;
    let mut run = 0_i32;

    for (idx, ch) in candidate.chars().enumerate() {
        if ch == current {
            run += 1;
            score += 10 + run * 3 - i32::try_from(idx).unwrap_or(i32::MAX / 8);
            if let Some(next) = q_chars.next() {
                current = next;
            } else {
                score += 40;
                return Some(score);
            }
        } else {
            run = 0;
        }
    }

    None
}

/// Activates the selected browser row by opening directories or applying wallpaper files.
async fn activate_browser_selection(state: &mut AppState) {
    let Some(entry) = state.browser_entries.get(state.browser_selected).cloned() else {
        state.status = String::from("No browser entry selected");
        return;
    };

    if entry.is_dir {
        state.browser_dir = entry.path;
        state.browser_query.clear();
        match reload_browser_directory(state) {
            Ok(()) => {
                let action = if entry.is_parent {
                    "Moved to parent directory"
                } else {
                    "Opened directory"
                };
                state.status = format!("{}: {}", action, state.browser_dir.display());
            }
            Err(err) => {
                state.status = format!("Failed to open directory: {err}");
            }
        }
        return;
    }

    let transition = build_transition_from_state(&state.transition);
    let namespace = state.backend.namespace.clone();
    let file_path = entry.path;
    let outputs = if let Some(monitor) = state.monitors.get(state.monitor_selected) {
        vec![monitor.name.clone()]
    } else {
        Vec::new()
    };

    match apply_wallpaper_request(file_path.clone(), transition, namespace, outputs).await {
        Ok(message) => state.status = message,
        Err(err) => state.status = format!("Wallpaper apply failed: {err}"),
    }
}

/// Sends a native wallpaper image request through the daemon IPC channel.
async fn apply_wallpaper_request(
    file_path: PathBuf,
    transition: Transition,
    namespace: String,
    outputs: Vec<String>,
) -> Result<String, String> {
    let path_for_error = file_path.clone();
    tokio::task::spawn_blocking(move || {
        let decoded = ImageReader::open(&file_path)
            .map_err(|err| format!("cannot open image '{}': {err}", path_for_error.display()))?
            .decode()
            .map_err(|err| format!("cannot decode image '{}': {err}", path_for_error.display()))?
            .to_rgb8();
        let (width, height) = decoded.dimensions();

        let img_send = ImgSend {
            path: file_path.to_string_lossy().to_string(),
            dim: (width, height),
            format: PixelFormat::Rgb,
            img: decoded.into_raw().into_boxed_slice(),
        };

        let mut builder = ImageRequestBuilder::new(transition)
            .map_err(|err| format!("request mmap failed: {err}"))?;
        builder.push(img_send, &namespace, "fit", "lanczos3", &outputs, None);

        let socket = IpcSocket::client(&namespace).map_err(|err| err.to_string())?;
        RequestSend::Img(builder.build())
            .send(&socket)
            .map_err(|err| err.to_string())?;

        let answer = Answer::receive(socket.recv().map_err(|err| err.to_string())?);
        match answer {
            Answer::Ok => Ok(format!("Applied wallpaper: {}", file_path.display())),
            Answer::Ping(ready) => Ok(format!("Applied wallpaper (backend ready={ready})")),
            Answer::Info(_) => Ok(format!("Applied wallpaper: {}", file_path.display())),
        }
    })
    .await
    .map_err(|err| format!("background task error: {err}"))?
}

/// Builds a daemon transition configuration from current TUI controls.
fn build_transition_from_state(state: &TransitionState) -> Transition {
    let transition_type = match TRANSITION_EFFECTS[state.effect_idx] {
        "fade" => TransitionType::Fade,
        "wipe" => TransitionType::Wipe,
        "grow" => TransitionType::Grow,
        _ => TransitionType::Simple,
    };

    let bezier = match EASING_PRESETS[state.easing_idx] {
        "linear" => (0.0, 0.0, 1.0, 1.0),
        "ease-in" => (0.42, 0.0, 1.0, 1.0),
        "ease-out" => (0.0, 0.0, 0.58, 1.0),
        _ => (0.42, 0.0, 0.58, 1.0),
    };

    Transition {
        transition_type,
        duration: state.duration_ms as f32 / 1000.0,
        step: NonZeroU8::new(2).expect("step must be non-zero"),
        fps: state.fps,
        angle: 0.0,
        pos: Position::new(Coord::Percent(0.5), Coord::Percent(0.5)),
        bezier,
        wave: (10.0, 10.0),
        invert_y: false,
    }
}

/// Checks whether a path looks like a supported image file.
fn is_supported_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp"
            )
        })
        .unwrap_or(false)
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

    draw_header(frame, root[0], state);
    draw_body(frame, root[1], state);
    draw_footer(frame, root[2], state);
}

/// Draws the top status header.
fn draw_header(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            "VELLUM",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  native wallpaper control surface  |  active pane: "),
        Span::styled(
            state.focus.as_str(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .title(" Session "),
    );
    frame.render_widget(title, area);
}

/// Draws the three core panes for the interactive TUI shell.
fn draw_body(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(area);

    let file_browser = Paragraph::new(render_browser_lines(state)).block(
        Block::default()
            .title(" Browser ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style_for_focus(state.focus, PaneFocus::Browser)),
    );

    let monitor_lines = render_monitor_lines(state);

    let monitor_preview = Paragraph::new(monitor_lines).block(
        Block::default()
            .title(" Monitor ")
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(border_style_for_focus(state.focus, PaneFocus::Monitor)),
    );

    let transition_message = render_transition_lines(state);

    let transition_panel = Paragraph::new(transition_message).block(
        Block::default()
            .title(" Transition Settings ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style_for_focus(state.focus, PaneFocus::Transition)),
    );

    frame.render_widget(file_browser, columns[0]);
    frame.render_widget(monitor_preview, columns[1]);
    frame.render_widget(transition_panel, columns[2]);
}

/// Computes pane border style depending on focus state.
fn border_style_for_focus(active: PaneFocus, pane: PaneFocus) -> Style {
    if active == pane {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

/// Builds browser pane content with highlighted selection.
fn render_browser_lines(state: &AppState) -> String {
    let mut lines = format!(
        "Dir: {}\nFilter: {}\n\n",
        state.browser_dir.display(),
        if state.browser_query.is_empty() {
            "(none)"
        } else {
            state.browser_query.as_str()
        }
    );

    if state.browser_entries.is_empty() {
        lines.push_str("No matching entries\n");
        lines.push_str("Type to fuzzy filter or Backspace to clear");
        return lines;
    }

    for (idx, entry) in state.browser_entries.iter().enumerate() {
        let marker = if idx == state.browser_selected {
            ">"
        } else {
            " "
        };
        let kind = if entry.is_parent {
            "[U]"
        } else if entry.is_dir {
            "[D]"
        } else {
            "[I]"
        };
        let row = format!("{} {} {}\n", marker, kind, entry.name);
        lines.push_str(&row);
    }
    lines.push_str("\nEnter: open/apply | u: parent | Type: fuzzy filter");
    lines
}

/// Builds monitor pane content with selected monitor detail.
fn render_monitor_lines(state: &AppState) -> String {
    if state.monitors.is_empty() {
        return String::from("Outputs\n- no monitor data yet\n\nr: refresh monitors");
    }

    let mut lines = String::from("Outputs\n");
    for (idx, monitor) in state.monitors.iter().enumerate() {
        let marker = if idx == state.monitor_selected {
            ">"
        } else {
            " "
        };
        let focused = if monitor.focused { "*" } else { " " };
        let row = format!(
            "{} [{}] {} {}x{}\n",
            marker, focused, monitor.name, monitor.width, monitor.height
        );
        lines.push_str(&row);
    }

    if let Some(selected) = state.monitors.get(state.monitor_selected) {
        let detail = format!(
            "\nSelected\n- position: ({}, {})\n- resolution: {}x{}",
            selected.x, selected.y, selected.width, selected.height
        );
        lines.push_str(&detail);
    }

    lines
}

/// Builds transition settings pane content with editable fields.
fn render_transition_lines(state: &AppState) -> String {
    let rows = [
        format!("duration_ms: {}", state.transition.duration_ms),
        format!("fps: {}", state.transition.fps),
        format!("easing: {}", EASING_PRESETS[state.transition.easing_idx]),
        format!(
            "effect: {}",
            TRANSITION_EFFECTS[state.transition.effect_idx]
        ),
    ];

    let mut lines = String::from("Parameters\n");
    for (idx, row) in rows.iter().enumerate() {
        let marker = if idx == state.transition.selected_field {
            ">"
        } else {
            " "
        };
        let line = format!("{} {}\n", marker, row);
        lines.push_str(&line);
    }

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
    let backend_line = format!(
        "\nBackend\n- status: {}\n- namespace: {}\n\nLeft/Right: tweak value, Enter in Browser: apply",
        backend_state, state.backend.namespace
    );
    lines.push_str(&backend_line);
    lines
}

/// Draws footer hints and runtime diagnostics.
fn draw_footer(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let frame_age_ms = state.last_frame.elapsed().as_millis();
    let diagnostics = format!(
        "{} | frames={} | frame_age={}ms | q quit | Tab pane | arrows nav | Enter apply/open | type fuzzy | r refresh | b backend",
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
