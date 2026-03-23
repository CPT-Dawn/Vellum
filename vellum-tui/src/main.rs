use std::{
    collections::HashMap,
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
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use directories::ProjectDirs;
use image::ImageReader;
use image::imageops::FilterType;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config as NucleoConfig, Matcher as NucleoMatcher, Utf32Str};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{process::Command, task::JoinHandle};
use vellum_core::{VellumServer, VellumServerConfig};

/// UI refresh cadence in milliseconds.
const TICK_RATE_MS: u64 = 33;
/// Delay before re-running fuzzy filter after typing.
const BROWSER_FILTER_DEBOUNCE_MS: u64 = 60;
/// Minimum interval between automatic monitor re-probes.
const MONITOR_REFRESH_INTERVAL: Duration = Duration::from_secs(10);
/// Retry interval used while waiting for native backend socket readiness.
const BACKEND_CONNECT_RETRY_INTERVAL: Duration = Duration::from_millis(100);
/// Maximum number of native backend socket readiness checks.
const BACKEND_CONNECT_RETRIES: usize = 25;

/// Available transition easings shown in the transition pane.
const EASING_PRESETS: [&str; 4] = ["linear", "ease-in", "ease-out", "ease-in-out"];
/// Available transition effects shown in the transition pane.
const TRANSITION_EFFECTS: [&str; 4] = ["simple", "fade", "wipe", "grow"];

/// Profile filename used by the default save/load commands.
const DEFAULT_PROFILE_NAME: &str = "default";

/// Number of visible rows moved by page-style navigation.
const NAV_PAGE_STEP: usize = 8;

/// Aspect-ratio simulation strategy for monitor preview.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum AspectMode {
    /// Preserve aspect ratio and fit within monitor bounds.
    Fit,
    /// Preserve aspect ratio and fill monitor bounds via cropping.
    Fill,
    /// Stretch image to monitor bounds.
    Stretch,
}

impl AspectMode {
    /// Cycles to the next simulation mode.
    fn next(self) -> Self {
        match self {
            Self::Fit => Self::Fill,
            Self::Fill => Self::Stretch,
            Self::Stretch => Self::Fit,
        }
    }

    /// Returns a printable mode label.
    fn as_str(self) -> &'static str {
        match self {
            Self::Fit => "fit",
            Self::Fill => "fill",
            Self::Stretch => "stretch",
        }
    }
}

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

/// Active keyboard interaction mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputMode {
    /// Standard command/navigation mode.
    Normal,
    /// Browser filter query editing mode.
    Search,
}

impl InputMode {
    /// Returns a short printable mode label.
    fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Search => "SEARCH",
        }
    }
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

/// Cached metadata for selected browser image.
#[derive(Debug, Clone)]
struct ImagePreview {
    /// Image path currently represented.
    path: PathBuf,
    /// Source image width in pixels.
    width: u32,
    /// Source image height in pixels.
    height: u32,
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

/// Serializable transition section for profile persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TransitionProfile {
    /// Current transition duration in milliseconds.
    duration_ms: u32,
    /// Current target frame rate.
    fps: u16,
    /// Selected easing preset index.
    easing_idx: usize,
    /// Selected effect preset index.
    effect_idx: usize,
}

/// Serializable profile data persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProfileData {
    /// Browser directory path.
    browser_dir: String,
    /// Browser fuzzy query.
    browser_query: String,
    /// Transition configuration.
    transition: TransitionProfile,
    /// Playlist file paths.
    playlist: Vec<String>,
    /// Playlist cycle interval in seconds.
    playlist_interval_secs: u64,
    /// Last selected monitor name.
    selected_monitor: Option<String>,
    /// Active aspect ratio simulator mode.
    aspect_mode: AspectMode,
}

/// Daemon output target selected for an apply request.
struct ApplyTarget {
    /// Namespace used for the daemon socket.
    namespace: String,
    /// Output names targeted by this apply request.
    outputs: Vec<String>,
    /// Required image dimensions for the output buffer.
    dim: (u32, u32),
    /// Expected pixel format for the output buffer.
    format: PixelFormat,
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
    /// Active keyboard mode.
    input_mode: InputMode,
    /// Whether a help overlay is currently visible.
    show_help: bool,
    /// Browser rows for phase scaffold.
    browser_entries: Vec<BrowserEntry>,
    /// Full unfiltered browser rows for current directory.
    browser_all_entries: Vec<BrowserEntry>,
    /// Current directory shown in the browser pane.
    browser_dir: PathBuf,
    /// Fuzzy query used to filter browser entries.
    browser_query: String,
    /// Whether browser filter recomputation is pending.
    browser_filter_dirty: bool,
    /// Last timestamp when browser query changed.
    browser_filter_last_input: Instant,
    /// Selected browser row index.
    browser_selected: usize,
    /// Selected monitor index in preview pane.
    monitor_selected: usize,
    /// Editable transition controls.
    transition: TransitionState,
    /// Active aspect-ratio simulation mode.
    aspect_mode: AspectMode,
    /// Selected image preview metadata for simulator.
    selected_preview: Option<ImagePreview>,
    /// Cache for image dimension lookups used by preview rendering.
    image_dim_cache: HashMap<PathBuf, Option<(u32, u32)>>,
    /// Playlist of images for auto-cycling.
    playlist: Vec<PathBuf>,
    /// Current index in the playlist.
    playlist_index: usize,
    /// Whether playlist auto-cycle is enabled.
    playlist_running: bool,
    /// Interval used between playlist image switches.
    playlist_interval: Duration,
    /// Timestamp of the previous playlist switch.
    last_playlist_tick: Instant,
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
            input_mode: InputMode::Normal,
            show_help: false,
            browser_entries: Vec::new(),
            browser_all_entries: Vec::new(),
            browser_dir: preferred_initial_browser_dir(),
            browser_query: String::new(),
            browser_filter_dirty: false,
            browser_filter_last_input: Instant::now(),
            browser_selected: 0,
            monitor_selected: 0,
            transition: TransitionState::default(),
            aspect_mode: AspectMode::Fit,
            selected_preview: None,
            image_dim_cache: HashMap::new(),
            playlist: Vec::new(),
            playlist_index: 0,
            playlist_running: false,
            playlist_interval: Duration::from_secs(15),
            last_playlist_tick: Instant::now(),
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
        flush_browser_filter_if_due(&mut state);
        refresh_monitors_if_due(&mut state).await;
        poll_backend_status(&mut state).await;
        run_playlist_tick(&mut state).await;
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
            if state.show_help {
                match key.code {
                    KeyCode::Char('?') | KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                        state.show_help = false;
                        state.status = String::from("Help overlay closed");
                    }
                    _ => {
                        state.show_help = false;
                        state.status = String::from("Help overlay closed");
                    }
                }
                continue;
            }

            if state.input_mode == InputMode::Search {
                match key.code {
                    KeyCode::Esc => {
                        state.input_mode = InputMode::Normal;
                        state.status = String::from("Search canceled");
                    }
                    KeyCode::Enter => {
                        flush_browser_filter_if_due(state);
                        state.input_mode = InputMode::Normal;
                        state.status = format!("Filter applied: {}", state.browser_query);
                    }
                    KeyCode::Backspace => {
                        if !state.browser_query.is_empty() {
                            state.browser_query.pop();
                            schedule_browser_filter(state);
                        }
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        state.browser_query.clear();
                        schedule_browser_filter(state);
                    }
                    KeyCode::Char(c) => {
                        if !c.is_control() {
                            state.browser_query.push(c);
                            schedule_browser_filter(state);
                        }
                    }
                    _ => {}
                }
                continue;
            }

            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    state.status = String::from("Exiting Vellum");
                    state.should_quit = true;
                }
                KeyCode::Char('?') => {
                    state.show_help = true;
                    state.status = String::from("Help overlay opened");
                }
                KeyCode::Char('/') => {
                    if state.focus == PaneFocus::Browser {
                        state.input_mode = InputMode::Search;
                        state.status = String::from("Search mode: type to filter, Enter to apply");
                    }
                }
                KeyCode::Char('r') => {
                    state.status = String::from("Manual monitor refresh requested");
                    state.needs_monitor_refresh = true;
                }
                KeyCode::Char('b') => {
                    start_native_backend(state);
                }
                KeyCode::Char('x') => {
                    state.aspect_mode = state.aspect_mode.next();
                    state.status = format!("Aspect simulator mode: {}", state.aspect_mode.as_str());
                }
                KeyCode::Char(' ') => {
                    state.playlist_running = !state.playlist_running;
                    state.last_playlist_tick = Instant::now();
                    state.status = if state.playlist_running {
                        String::from("Playlist auto-cycle enabled")
                    } else {
                        String::from("Playlist auto-cycle paused")
                    };
                }
                KeyCode::Char('p') => {
                    add_selected_image_to_playlist(state);
                }
                KeyCode::Char('c') => {
                    state.playlist.clear();
                    state.playlist_index = 0;
                    state.status = String::from("Playlist cleared");
                }
                KeyCode::Char('+') | KeyCode::Char('=') => {
                    let current = state.playlist_interval.as_secs();
                    state.playlist_interval = Duration::from_secs((current + 5).min(600));
                    state.status =
                        format!("Playlist interval: {}s", state.playlist_interval.as_secs());
                }
                KeyCode::Char('-') => {
                    let current = state.playlist_interval.as_secs();
                    state.playlist_interval = Duration::from_secs(current.saturating_sub(5).max(5));
                    state.status =
                        format!("Playlist interval: {}s", state.playlist_interval.as_secs());
                }
                KeyCode::Char('u') => {
                    if state.focus == PaneFocus::Browser
                        && let Some(parent) = state.browser_dir.parent().map(Path::to_path_buf)
                    {
                        state.browser_dir = parent;
                        if let Err(err) = reload_browser_directory(state) {
                            state.status = format!("Failed to open parent directory: {err}");
                        }
                    }
                }
                KeyCode::Tab => {
                    state.focus = state.focus.next();
                    state.status = format!("Focus moved to {} pane", state.focus.as_str());
                }
                KeyCode::Char('l') => {
                    state.focus = state.focus.next();
                    state.status = format!("Focus moved to {} pane", state.focus.as_str());
                }
                KeyCode::BackTab => {
                    state.focus = state.focus.prev();
                    state.status = format!("Focus moved to {} pane", state.focus.as_str());
                }
                KeyCode::Char('h') => match state.focus {
                    PaneFocus::Transition => state.transition.decrease_selected(),
                    _ => {
                        state.focus = state.focus.prev();
                        state.status = format!("Focus moved to {} pane", state.focus.as_str());
                    }
                },
                KeyCode::Up => match state.focus {
                    PaneFocus::Browser => {
                        move_browser_selection(state, -1);
                        refresh_selected_preview(state);
                    }
                    PaneFocus::Monitor => {
                        move_monitor_selection(state, -1);
                    }
                    PaneFocus::Transition => {
                        state.transition.select_prev_field();
                    }
                },
                KeyCode::Char('k') => match state.focus {
                    PaneFocus::Browser => {
                        move_browser_selection(state, -1);
                        refresh_selected_preview(state);
                    }
                    PaneFocus::Monitor => {
                        move_monitor_selection(state, -1);
                    }
                    PaneFocus::Transition => {
                        state.transition.select_prev_field();
                    }
                },
                KeyCode::Down => match state.focus {
                    PaneFocus::Browser => {
                        move_browser_selection(state, 1);
                        refresh_selected_preview(state);
                    }
                    PaneFocus::Monitor => {
                        move_monitor_selection(state, 1);
                    }
                    PaneFocus::Transition => {
                        state.transition.select_next_field();
                    }
                },
                KeyCode::Char('j') => match state.focus {
                    PaneFocus::Browser => {
                        move_browser_selection(state, 1);
                        refresh_selected_preview(state);
                    }
                    PaneFocus::Monitor => {
                        move_monitor_selection(state, 1);
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
                    if state.focus == PaneFocus::Browser {
                        state.browser_query.clear();
                        schedule_browser_filter(state);
                        state.status = String::from("Browser filter cleared");
                    }
                }
                KeyCode::Enter => {
                    if state.focus == PaneFocus::Browser {
                        activate_browser_selection(state).await;
                    }
                }
                KeyCode::PageUp => match state.focus {
                    PaneFocus::Browser => {
                        move_browser_selection(state, -(NAV_PAGE_STEP as isize));
                        refresh_selected_preview(state);
                    }
                    PaneFocus::Monitor => {
                        move_monitor_selection(state, -(NAV_PAGE_STEP as isize));
                    }
                    PaneFocus::Transition => {
                        for _ in 0..NAV_PAGE_STEP {
                            state.transition.select_prev_field();
                        }
                    }
                },
                KeyCode::PageDown => match state.focus {
                    PaneFocus::Browser => {
                        move_browser_selection(state, NAV_PAGE_STEP as isize);
                        refresh_selected_preview(state);
                    }
                    PaneFocus::Monitor => {
                        move_monitor_selection(state, NAV_PAGE_STEP as isize);
                    }
                    PaneFocus::Transition => {
                        for _ in 0..NAV_PAGE_STEP {
                            state.transition.select_next_field();
                        }
                    }
                },
                KeyCode::Home | KeyCode::Char('g') => match state.focus {
                    PaneFocus::Browser => {
                        state.browser_selected = 0;
                        refresh_selected_preview(state);
                    }
                    PaneFocus::Monitor => {
                        state.monitor_selected = 0;
                    }
                    PaneFocus::Transition => {
                        state.transition.selected_field = 0;
                    }
                },
                KeyCode::End | KeyCode::Char('G') => match state.focus {
                    PaneFocus::Browser => {
                        state.browser_selected = state.browser_entries.len().saturating_sub(1);
                        refresh_selected_preview(state);
                    }
                    PaneFocus::Monitor => {
                        state.monitor_selected = state.monitors.len().saturating_sub(1);
                    }
                    PaneFocus::Transition => {
                        state.transition.selected_field = 3;
                    }
                },
                KeyCode::F(5) => {
                    state.playlist_running = !state.playlist_running;
                    state.last_playlist_tick = Instant::now();
                    state.status = if state.playlist_running {
                        String::from("Playlist auto-cycle enabled")
                    } else {
                        String::from("Playlist auto-cycle paused")
                    };
                }
                KeyCode::F(6) => {
                    add_selected_image_to_playlist(state);
                }
                KeyCode::F(7) => {
                    state.playlist.clear();
                    state.playlist_index = 0;
                    state.status = String::from("Playlist cleared");
                }
                KeyCode::F(8) => match save_profile(state, DEFAULT_PROFILE_NAME) {
                    Ok(path) => state.status = format!("Saved profile: {}", path.display()),
                    Err(err) => state.status = format!("Save profile failed: {err}"),
                },
                KeyCode::F(9) => match load_profile(state, DEFAULT_PROFILE_NAME) {
                    Ok(()) => state.status = String::from("Loaded profile: default"),
                    Err(err) => state.status = format!("Load profile failed: {err}"),
                },
                _ => {}
            }
        }
    }

    Ok(())
}

/// Moves browser selection by signed offset while clamping bounds.
fn move_browser_selection(state: &mut AppState, delta: isize) {
    if state.browser_entries.is_empty() {
        state.browser_selected = 0;
        return;
    }

    let max_idx = state.browser_entries.len().saturating_sub(1) as isize;
    let next = (state.browser_selected as isize + delta).clamp(0, max_idx) as usize;
    state.browser_selected = next;
}

/// Moves monitor selection by signed offset while clamping bounds.
fn move_monitor_selection(state: &mut AppState, delta: isize) {
    if state.monitors.is_empty() {
        state.monitor_selected = 0;
        return;
    }

    let max_idx = state.monitors.len().saturating_sub(1) as isize;
    let next = (state.monitor_selected as isize + delta).clamp(0, max_idx) as usize;
    state.monitor_selected = next;
}

/// Marks browser filtering as dirty and schedules a debounced recompute.
fn schedule_browser_filter(state: &mut AppState) {
    state.browser_filter_dirty = true;
    state.browser_filter_last_input = Instant::now();

    if state.browser_query.is_empty() {
        apply_browser_filter(state);
        state.browser_filter_dirty = false;
    }
}

/// Recomputes browser fuzzy results once the debounce threshold passes.
fn flush_browser_filter_if_due(state: &mut AppState) {
    if !state.browser_filter_dirty {
        return;
    }

    if state.browser_filter_last_input.elapsed()
        >= Duration::from_millis(BROWSER_FILTER_DEBOUNCE_MS)
    {
        apply_browser_filter(state);
        state.browser_filter_dirty = false;
    }
}

/// Starts the native Vellum backend daemon in a dedicated blocking task.
fn start_native_backend(state: &mut AppState) {
    if IpcSocket::client(&state.backend.namespace).is_ok() {
        state.status = String::from("Native backend already reachable");
        return;
    }

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
            quiet: true,
            ..VellumServerConfig::default()
        };
        let server = VellumServer::new(config);
        server.run().map_err(|err| err.to_string())
    }));
}

/// Ensures the embedded native backend socket is reachable before IPC calls.
async fn ensure_native_backend_ready(state: &mut AppState) -> Result<(), String> {
    if IpcSocket::client(&state.backend.namespace).is_ok() {
        return Ok(());
    }

    start_native_backend(state);

    for _ in 0..BACKEND_CONNECT_RETRIES {
        if IpcSocket::client(&state.backend.namespace).is_ok() {
            return Ok(());
        }
        tokio::time::sleep(BACKEND_CONNECT_RETRY_INTERVAL).await;
    }

    Err(format!(
        "native backend is not reachable in namespace '{}'",
        state.backend.namespace
    ))
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

    dirs.sort_by_cached_key(|entry| entry.name.to_ascii_lowercase());
    files.sort_by_cached_key(|entry| entry.name.to_ascii_lowercase());

    entries.extend(dirs);
    entries.extend(files);

    state.browser_all_entries = entries;
    apply_browser_filter(state);
    state.browser_filter_dirty = false;
    Ok(())
}

/// Applies fuzzy filtering to browser rows based on the active query.
fn apply_browser_filter(state: &mut AppState) {
    if state.browser_query.is_empty() {
        state.browser_entries = state.browser_all_entries.clone();
    } else {
        let pattern = Pattern::parse(
            &state.browser_query,
            CaseMatching::Ignore,
            Normalization::Smart,
        );
        let mut matcher = NucleoMatcher::new(NucleoConfig::DEFAULT.match_paths());
        let mut utf32_buf = Vec::new();

        let mut scored = state
            .browser_all_entries
            .iter()
            .cloned()
            .filter_map(|entry| {
                pattern
                    .score(
                        Utf32Str::new(entry.name.as_str(), &mut utf32_buf),
                        &mut matcher,
                    )
                    .map(|score| (score, entry))
            })
            .collect::<Vec<_>>();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
        state.browser_entries = scored.into_iter().map(|(_, entry)| entry).collect();
    }

    state.browser_selected = state
        .browser_selected
        .min(state.browser_entries.len().saturating_sub(1));
    refresh_selected_preview(state);
}

/// Refreshes the selected-image metadata used by aspect ratio simulation.
fn refresh_selected_preview(state: &mut AppState) {
    let Some(entry) = state.browser_entries.get(state.browser_selected) else {
        state.selected_preview = None;
        return;
    };

    if entry.is_dir {
        state.selected_preview = None;
        return;
    }

    let dims = if let Some(cached) = state.image_dim_cache.get(&entry.path) {
        *cached
    } else {
        let probed = image::image_dimensions(&entry.path).ok();
        state.image_dim_cache.insert(entry.path.clone(), probed);
        probed
    };

    state.selected_preview = dims.map(|(width, height)| ImagePreview {
        path: entry.path.clone(),
        width,
        height,
    });
}

/// Adds the currently selected browser image to the playlist.
fn add_selected_image_to_playlist(state: &mut AppState) {
    let Some(entry) = state.browser_entries.get(state.browser_selected) else {
        state.status = String::from("No browser entry selected");
        return;
    };

    if entry.is_dir {
        state.status = String::from("Playlist accepts image files only");
        return;
    }

    if state.playlist.iter().any(|path| path == &entry.path) {
        state.status = String::from("Selected image already in playlist");
        return;
    }

    state.playlist.push(entry.path.clone());
    state.status = format!("Playlist size: {}", state.playlist.len());
}

/// Applies the next playlist image when auto-cycling is active.
async fn run_playlist_tick(state: &mut AppState) {
    if !state.playlist_running || state.playlist.is_empty() {
        return;
    }

    if state.last_playlist_tick.elapsed() < state.playlist_interval {
        return;
    }

    let index = state.playlist_index % state.playlist.len();
    let file_path = state.playlist[index].clone();
    state.playlist_index = (state.playlist_index + 1) % state.playlist.len();
    state.last_playlist_tick = Instant::now();

    let namespace = state.backend.namespace.clone();
    let selected_output = state
        .monitors
        .get(state.monitor_selected)
        .map(|m| m.name.clone());

    if let Err(err) = ensure_native_backend_ready(state).await {
        state.playlist_running = false;
        state.status = format!("Playlist paused: {err}");
        return;
    }

    let target = match query_apply_target(&namespace, selected_output) {
        Ok(target) => target,
        Err(err) => {
            state.status = format!("Playlist target query failed: {err}");
            return;
        }
    };

    let transition = build_transition_from_state(&state.transition);

    match apply_wallpaper_request(file_path.clone(), transition, target).await {
        Ok(_) => {
            state.status = format!("Playlist applied: {}", file_path.display());
        }
        Err(err) => {
            state.status = format!("Playlist apply failed: {err}");
        }
    }
}

/// Resolves the profile storage path for a profile name.
fn profile_path(profile_name: &str) -> Result<PathBuf, String> {
    let Some(project_dirs) = ProjectDirs::from("com", "vellum", "vellum") else {
        return Err(String::from("cannot resolve config directory"));
    };

    let config_dir = project_dirs.config_dir().join("profiles");
    fs::create_dir_all(&config_dir).map_err(|err| {
        format!(
            "cannot create profile directory '{}': {err}",
            config_dir.display()
        )
    })?;
    Ok(config_dir.join(format!("{profile_name}.json")))
}

/// Saves current UI runtime configuration as a named profile.
fn save_profile(state: &AppState, profile_name: &str) -> Result<PathBuf, String> {
    let path = profile_path(profile_name)?;
    let data = ProfileData {
        browser_dir: state.browser_dir.to_string_lossy().to_string(),
        browser_query: state.browser_query.clone(),
        transition: TransitionProfile {
            duration_ms: state.transition.duration_ms,
            fps: state.transition.fps,
            easing_idx: state.transition.easing_idx,
            effect_idx: state.transition.effect_idx,
        },
        playlist: state
            .playlist
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect(),
        playlist_interval_secs: state.playlist_interval.as_secs(),
        selected_monitor: state
            .monitors
            .get(state.monitor_selected)
            .map(|m| m.name.clone()),
        aspect_mode: state.aspect_mode,
    };

    let json = serde_json::to_string_pretty(&data)
        .map_err(|err| format!("cannot serialize profile: {err}"))?;
    fs::write(&path, json)
        .map_err(|err| format!("cannot write profile '{}': {err}", path.display()))?;
    Ok(path)
}

/// Loads a named profile and applies it to the current TUI state.
fn load_profile(state: &mut AppState, profile_name: &str) -> Result<(), String> {
    let path = profile_path(profile_name)?;
    let bytes = fs::read(&path)
        .map_err(|err| format!("cannot read profile '{}': {err}", path.display()))?;
    let data: ProfileData =
        serde_json::from_slice(&bytes).map_err(|err| format!("invalid profile JSON: {err}"))?;

    state.browser_dir = PathBuf::from(&data.browser_dir);
    if !state.browser_dir.exists() {
        state.browser_dir = preferred_initial_browser_dir();
    }
    state.browser_query = data.browser_query;
    state.transition.duration_ms = data.transition.duration_ms;
    state.transition.fps = data.transition.fps;
    state.transition.easing_idx = data.transition.easing_idx.min(EASING_PRESETS.len() - 1);
    state.transition.effect_idx = data.transition.effect_idx.min(TRANSITION_EFFECTS.len() - 1);
    state.aspect_mode = data.aspect_mode;

    state.playlist = data.playlist.iter().map(PathBuf::from).collect();
    state.playlist.retain(|path| path.is_file());
    state.playlist_index = 0;
    state.playlist_interval = Duration::from_secs(data.playlist_interval_secs.max(5));

    reload_browser_directory(state)
        .map_err(|err| format!("cannot reload browser after profile load: {err}"))?;

    if let Some(selected_monitor_name) = data.selected_monitor
        && let Some(index) = state
            .monitors
            .iter()
            .position(|monitor| monitor.name == selected_monitor_name)
    {
        state.monitor_selected = index;
    }

    Ok(())
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

    let namespace = state.backend.namespace.clone();
    let file_path = entry.path;
    let selected_output = state
        .monitors
        .get(state.monitor_selected)
        .map(|m| m.name.clone());

    if let Err(err) = ensure_native_backend_ready(state).await {
        state.status = format!("Native backend unavailable: {err}");
        return;
    }

    let target = match query_apply_target(&namespace, selected_output) {
        Ok(target) => target,
        Err(err) => {
            state.status = format!("Output target query failed: {err}");
            return;
        }
    };

    let transition = build_transition_from_state(&state.transition);

    match apply_wallpaper_request(file_path.clone(), transition, target).await {
        Ok(message) => state.status = message,
        Err(err) => state.status = format!("Wallpaper apply failed: {err}"),
    }
}

/// Queries daemon output metadata and resolves the exact target for an image apply request.
fn query_apply_target(
    namespace: &str,
    preferred_output: Option<String>,
) -> Result<ApplyTarget, String> {
    let socket = connect_daemon_socket(namespace)?;
    RequestSend::Query
        .send(&socket)
        .map_err(|err| err.to_string())?;

    let answer = Answer::receive(socket.recv().map_err(|err| err.to_string())?);
    let Answer::Info(outputs) = answer else {
        return Err(String::from("unexpected daemon response to query request"));
    };

    let selected = if let Some(name) = preferred_output {
        outputs.iter().find(|output| output.name.as_ref() == name)
    } else {
        outputs.first()
    }
    .ok_or_else(|| String::from("no output information available from daemon"))?;

    Ok(ApplyTarget {
        namespace: namespace.to_string(),
        outputs: vec![selected.name.to_string()],
        dim: selected.real_dim(),
        format: selected.pixel_format,
    })
}

/// Connects only to the native backend socket namespace owned by this TUI.
fn connect_daemon_socket(namespace: &str) -> Result<IpcSocket, String> {
    IpcSocket::client(namespace)
        .map_err(|err| format!("native backend socket '{}' unavailable: {err}", namespace))
}

/// Sends a native wallpaper image request through the daemon IPC channel.
async fn apply_wallpaper_request(
    file_path: PathBuf,
    transition: Transition,
    target: ApplyTarget,
) -> Result<String, String> {
    let path_for_error = file_path.clone();
    tokio::task::spawn_blocking(move || {
        let decoded = ImageReader::open(&file_path)
            .map_err(|err| format!("cannot open image '{}': {err}", path_for_error.display()))?
            .decode()
            .map_err(|err| format!("cannot decode image '{}': {err}", path_for_error.display()))?;

        let resized = decoded.resize_exact(target.dim.0, target.dim.1, FilterType::Lanczos3);
        let (img_bytes, pixel_format) =
            convert_dynamic_image_for_pixel_format(resized, target.format);

        let img_send = ImgSend {
            path: file_path.to_string_lossy().to_string(),
            dim: target.dim,
            format: pixel_format,
            img: img_bytes.into_boxed_slice(),
        };

        let mut builder = ImageRequestBuilder::new(transition)
            .map_err(|err| format!("request mmap failed: {err}"))?;
        builder.push(
            img_send,
            &target.namespace,
            "fit",
            "lanczos3",
            &target.outputs,
            None,
        );

        let socket = IpcSocket::client(&target.namespace).map_err(|err| err.to_string())?;
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

/// Converts a decoded image into byte layout expected by daemon pixel format.
fn convert_dynamic_image_for_pixel_format(
    image: image::DynamicImage,
    format: PixelFormat,
) -> (Vec<u8>, PixelFormat) {
    match format {
        PixelFormat::Bgr => {
            let rgb = image.to_rgb8();
            (rgb.into_raw(), PixelFormat::Bgr)
        }
        PixelFormat::Rgb => {
            let mut rgb = image.to_rgb8().into_raw();
            for chunk in rgb.chunks_exact_mut(3) {
                chunk.swap(0, 2);
            }
            (rgb, PixelFormat::Rgb)
        }
        PixelFormat::Abgr => {
            let rgba = image.to_rgba8();
            (rgba.into_raw(), PixelFormat::Abgr)
        }
        PixelFormat::Argb => {
            let mut rgba = image.to_rgba8().into_raw();
            for chunk in rgba.chunks_exact_mut(4) {
                chunk.swap(0, 2);
            }
            (rgba, PixelFormat::Argb)
        }
    }
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

    if state.show_help {
        draw_help_overlay(frame, state);
    }
}

/// Draws the top status header.
fn draw_header(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let backend_running = state
        .backend
        .task
        .as_ref()
        .is_some_and(|task| !task.is_finished());

    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            "VELLUM",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  wallpaper control surface  "),
        Span::styled(
            if backend_running {
                "[backend: running]"
            } else {
                "[backend: stopped]"
            },
            Style::default()
                .fg(if backend_running {
                    Color::LightGreen
                } else {
                    Color::Yellow
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("[outputs: {}]", state.monitors.len()),
            Style::default().fg(Color::Gray),
        ),
        Span::raw("  "),
        Span::styled(
            format!("[mode: {}]", state.input_mode.as_str()),
            Style::default()
                .fg(if state.input_mode == InputMode::Search {
                    Color::Yellow
                } else {
                    Color::Green
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  active pane: "),
        Span::styled(
            state.focus.as_str(),
            Style::default()
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(Color::Gray))
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
            .title(Line::from(vec![
                Span::styled(" Browser ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("({})", state.browser_entries.len()),
                    Style::default().fg(Color::Gray),
                ),
            ]))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style_for_focus(state.focus, PaneFocus::Browser)),
    );

    let monitor_lines = render_monitor_lines(state);

    let monitor_preview = Paragraph::new(monitor_lines).block(
        Block::default()
            .title(" Monitor Grid ")
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(border_style_for_focus(state.focus, PaneFocus::Monitor)),
    );

    let transition_message = render_transition_lines(state);

    let transition_panel = Paragraph::new(transition_message).block(
        Block::default()
            .title(" Transition + Runtime ")
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
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    }
}

/// Builds browser pane content with highlighted selection.
fn render_browser_lines(state: &AppState) -> String {
    let mut lines = format!(
        "Dir: {}\nFilter: {}{}\n\n",
        state.browser_dir.display(),
        if state.browser_query.is_empty() {
            "(none)"
        } else {
            state.browser_query.as_str()
        },
        if state.browser_filter_dirty {
            "  [updating]"
        } else {
            ""
        }
    );

    if state.browser_entries.is_empty() {
        lines.push_str("No matching entries\n");
        lines.push_str("Press / to search or Backspace to clear");
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
    lines.push_str("\nEnter open/apply | / search | j/k move | g/G top-bottom | u parent");
    lines
}

/// Builds monitor pane content with selected monitor detail.
fn render_monitor_lines(state: &AppState) -> String {
    if state.monitors.is_empty() {
        return String::from("Outputs\n- no monitor data yet\n\nr refresh monitors");
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

        let aspect_line = format!("\nAspect Simulator ({})", state.aspect_mode.as_str());
        lines.push_str(&aspect_line);
        if let Some(preview) = &state.selected_preview {
            let sim = simulate_aspect(
                (preview.width, preview.height),
                (selected.width, selected.height),
                state.aspect_mode,
            );
            let sim_details = format!(
                "\n- image: {}\n- source: {}x{}\n- simulated: {}x{}\n- bars: {}x{}\n- crop: {}x{}",
                preview.path.display(),
                preview.width,
                preview.height,
                sim.target_width,
                sim.target_height,
                sim.bars_x,
                sim.bars_y,
                sim.crop_x,
                sim.crop_y
            );
            lines.push_str(&sim_details);
        } else {
            lines.push_str("\n- select an image file in Browser to preview");
        }
    }

    lines
}

/// Result of aspect-ratio simulation against a monitor target.
struct AspectSimulation {
    /// Simulated target width.
    target_width: u32,
    /// Simulated target height.
    target_height: u32,
    /// Horizontal letterbox/pillarbox size in pixels.
    bars_x: u32,
    /// Vertical letterbox/pillarbox size in pixels.
    bars_y: u32,
    /// Horizontal crop size in pixels.
    crop_x: u32,
    /// Vertical crop size in pixels.
    crop_y: u32,
}

/// Simulates how an image maps to monitor dimensions for a selected aspect mode.
fn simulate_aspect(image: (u32, u32), monitor: (u32, u32), mode: AspectMode) -> AspectSimulation {
    let (iw, ih) = (image.0.max(1) as f64, image.1.max(1) as f64);
    let (mw, mh) = (monitor.0.max(1) as f64, monitor.1.max(1) as f64);

    let scale = match mode {
        AspectMode::Fit => (mw / iw).min(mh / ih),
        AspectMode::Fill => (mw / iw).max(mh / ih),
        AspectMode::Stretch => 0.0,
    };

    let (tw, th) = if matches!(mode, AspectMode::Stretch) {
        (monitor.0.max(1), monitor.1.max(1))
    } else {
        (
            (iw * scale).round().max(1.0) as u32,
            (ih * scale).round().max(1.0) as u32,
        )
    };

    let bars_x = monitor.0.saturating_sub(tw) / 2;
    let bars_y = monitor.1.saturating_sub(th) / 2;
    let crop_x = tw.saturating_sub(monitor.0) / 2;
    let crop_y = th.saturating_sub(monitor.1) / 2;

    AspectSimulation {
        target_width: tw,
        target_height: th,
        bars_x,
        bars_y,
        crop_x,
        crop_y,
    }
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
        "\nBackend\n- status: {}\n- namespace: {}\n\nPlaylist\n- running: {}\n- size: {}\n- interval: {}s\n\nSpace toggle | p add | c clear | +/- interval | F8 save | F9 load",
        backend_state,
        state.backend.namespace,
        state.playlist_running,
        state.playlist.len(),
        state.playlist_interval.as_secs()
    );
    lines.push_str(&backend_line);
    lines
}

/// Draws footer hints and runtime diagnostics.
fn draw_footer(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let frame_age_ms = state.last_frame.elapsed().as_millis();
    let diagnostics = format!(
        "{} | frames={} | frame_age={}ms | ? help | q quit | Tab/h/l panes | arrows or j/k nav | / search | Enter apply/open",
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

/// Draws a centered quick-reference panel for keyboard controls.
fn draw_help_overlay(frame: &mut Frame<'_>, state: &AppState) {
    let popup = centered_rect(78, 72, frame.area());
    let help = format!(
        "VELLUM HELP\n\nMode: {}\nPane: {}\n\nGlobal\n- ? toggle help\n- q or Esc quit\n- Tab / Shift+Tab cycle pane\n- h/l previous/next pane\n\nNavigation\n- arrows or j/k move selection\n- g / G jump top/bottom\n- PageUp/PageDown jump by {}\n\nBrowser\n- Enter open directory / apply image\n- / enter search mode\n- Search mode: Enter apply, Esc cancel, Ctrl+u clear\n- u go to parent directory\n\nMonitor\n- r refresh monitor list\n- x cycle aspect simulation\n\nPlaylist/Profile\n- Space toggle playlist\n- p add selected image\n- c clear playlist\n- +/- adjust interval\n- F8 save profile, F9 load profile\n\nPress ?, Enter, Esc, or q to close.",
        state.input_mode.as_str(),
        state.focus.as_str(),
        NAV_PAGE_STEP
    );

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(help).block(
            Block::default()
                .title(" Quick Help ")
                .borders(Borders::ALL)
                .border_type(BorderType::Double)
                .border_style(Style::default().fg(Color::Yellow)),
        ),
        popup,
    );
}

/// Returns a rectangle centered within the provided area using percentages.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}
