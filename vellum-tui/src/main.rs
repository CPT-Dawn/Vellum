use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    fs, io,
    num::NonZeroU8,
    path::{Path, PathBuf},
    sync::Arc,
    sync::OnceLock,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
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
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Row, Table, Tabs},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    process::Command,
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    task::{JoinHandle, JoinSet},
    time,
};
use tokio_util::sync::CancellationToken;
use vellum_core::{VellumServer, VellumServerConfig};

const TICK_RATE_MS: u64 = 33;
const BROWSER_FILTER_DEBOUNCE_MS: u64 = 70;
const IPC_QUERY_TIMEOUT: Duration = Duration::from_millis(1600);
const IPC_APPLY_TIMEOUT: Duration = Duration::from_secs(8);
const MONITOR_REFRESH_INTERVAL: Duration = Duration::from_secs(10);
const DEFAULT_PROFILE_NAME: &str = "default";
const NAV_HALF_PAGE_STEP: isize = 4;
const LOG_CAPACITY: usize = 1200;

const EASING_PRESETS: [&str; 4] = ["linear", "ease-in", "ease-out", "ease-in-out"];
const TRANSITION_EFFECTS: [&str; 4] = ["simple", "fade", "wipe", "grow"];

// Tokyo Night inspired palette.
const COLOR_BG: Color = Color::Rgb(26, 27, 38);
const COLOR_PANEL: Color = Color::Rgb(31, 35, 53);
const COLOR_PANEL_DIM: Color = Color::Rgb(28, 31, 47);
const COLOR_TEXT: Color = Color::Rgb(192, 202, 245);
const COLOR_MUTED: Color = Color::Rgb(122, 134, 180);
const COLOR_ACCENT: Color = Color::Rgb(125, 207, 255);
const COLOR_ACCENT_ALT: Color = Color::Rgb(187, 154, 247);
const COLOR_SELECTION_BG: Color = Color::Rgb(59, 66, 97);
const COLOR_OK: Color = Color::Rgb(158, 206, 106);
const COLOR_WARN: Color = Color::Rgb(224, 175, 104);
const COLOR_ERR: Color = Color::Rgb(247, 118, 142);
const COLOR_DIR: Color = Color::Rgb(122, 162, 247);
const COLOR_FILE: Color = Color::Rgb(169, 177, 214);

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum AspectMode {
    Fit,
    Fill,
    Stretch,
}

impl AspectMode {
    fn next(self) -> Self {
        match self {
            Self::Fit => Self::Fill,
            Self::Fill => Self::Stretch,
            Self::Stretch => Self::Fit,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Fit => "fit",
            Self::Fill => "fill",
            Self::Stretch => "stretch",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaneFocus {
    Browser,
    Monitor,
    Transition,
    Logs,
}

impl PaneFocus {
    fn next(self) -> Self {
        match self {
            Self::Browser => Self::Monitor,
            Self::Monitor => Self::Transition,
            Self::Transition => Self::Logs,
            Self::Logs => Self::Browser,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::Browser => Self::Logs,
            Self::Monitor => Self::Browser,
            Self::Transition => Self::Monitor,
            Self::Logs => Self::Transition,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Browser => "Browser",
            Self::Monitor => "Monitor",
            Self::Transition => "Transition",
            Self::Logs => "Logs",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Normal,
    Search,
}

impl InputMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Search => "SEARCH",
        }
    }
}

#[derive(Debug, Clone)]
struct BrowserEntry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    is_parent: bool,
}

#[derive(Debug, Clone)]
struct ImagePreview {
    path: PathBuf,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone)]
struct TransitionState {
    duration_ms: u32,
    fps: u16,
    easing_idx: usize,
    effect_idx: usize,
    selected_field: usize,
}

impl Default for TransitionState {
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
    fn select_prev_field(&mut self) {
        self.selected_field = self.selected_field.saturating_sub(1);
    }

    fn select_next_field(&mut self) {
        self.selected_field = (self.selected_field + 1).min(3);
    }

    fn increase_selected(&mut self) {
        match self.selected_field {
            0 => self.duration_ms = (self.duration_ms + 25).min(15_000),
            1 => self.fps = (self.fps + 5).min(240),
            2 => self.easing_idx = (self.easing_idx + 1) % EASING_PRESETS.len(),
            3 => self.effect_idx = (self.effect_idx + 1) % TRANSITION_EFFECTS.len(),
            _ => {}
        }
    }

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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TransitionProfile {
    duration_ms: u32,
    fps: u16,
    easing_idx: usize,
    effect_idx: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProfileData {
    browser_dir: String,
    browser_query: String,
    transition: TransitionProfile,
    playlist: Vec<String>,
    playlist_interval_secs: u64,
    selected_monitor: Option<String>,
    aspect_mode: AspectMode,
}

#[derive(Debug, Clone)]
struct MonitorEntry {
    name: String,
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    focused: bool,
}

struct ApplyTarget {
    namespace: String,
    monitors: Vec<MonitorApplyTarget>,
}

#[derive(Debug, Clone)]
struct MonitorApplyTarget {
    output: String,
    dim: (u32, u32),
    format: PixelFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DaemonState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Crashed,
}

impl DaemonState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "STOPPED",
            Self::Starting => "STARTING",
            Self::Running => "RUNNING",
            Self::Stopping => "STOPPING",
            Self::Crashed => "CRASHED",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Stopped => COLOR_WARN,
            Self::Starting => COLOR_ACCENT,
            Self::Running => COLOR_OK,
            Self::Stopping => COLOR_WARN,
            Self::Crashed => COLOR_ERR,
        }
    }
}

#[derive(Debug)]
struct BackendRuntime {
    task: Option<JoinHandle<Result<()>>>,
    shutdown: Option<CancellationToken>,
    namespace: String,
    state: DaemonState,
    restart_requested: bool,
    last_error: Option<String>,
}

impl Default for BackendRuntime {
    fn default() -> Self {
        Self {
            task: None,
            shutdown: None,
            namespace: String::from("vellum-tui"),
            state: DaemonState::Stopped,
            restart_requested: false,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum UiLogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl UiLogLevel {
    fn from_common(level: common::log::Filter) -> Self {
        match level {
            common::log::Filter::Trace => Self::Trace,
            common::log::Filter::Debug => Self::Debug,
            common::log::Filter::Info => Self::Info,
            common::log::Filter::Warn => Self::Warn,
            common::log::Filter::Error | common::log::Filter::Fatal => Self::Error,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Trace => COLOR_MUTED,
            Self::Debug => COLOR_ACCENT_ALT,
            Self::Info => COLOR_TEXT,
            Self::Warn => COLOR_WARN,
            Self::Error => COLOR_ERR,
        }
    }
}

#[derive(Debug, Clone)]
struct LogEntry {
    at: Instant,
    level: UiLogLevel,
    message: String,
}

#[derive(Debug, Clone, Copy)]
enum StatusLevel {
    Info,
    Ok,
    Warn,
    Error,
}

impl StatusLevel {
    fn color(self) -> Color {
        match self {
            Self::Info => COLOR_TEXT,
            Self::Ok => COLOR_OK,
            Self::Warn => COLOR_WARN,
            Self::Error => COLOR_ERR,
        }
    }
}

enum AppEvent {
    ApplyBrowserFilter,
    ApplyFinished(std::result::Result<String, ApplyError>),
    DaemonExited(std::result::Result<(), String>),
    Log(UiLogLevel, String),
}

#[derive(Debug, thiserror::Error)]
enum ApplyError {
    #[error("IPC request timed out after {0}ms")]
    Timeout(u128),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

struct AppState {
    should_quit: bool,
    status: String,
    status_level: StatusLevel,
    frame_count: u64,
    last_frame: Instant,

    backend: BackendRuntime,
    monitors: Vec<MonitorEntry>,
    monitor_selected: usize,
    monitor_list_state: ListState,
    last_monitor_refresh: Option<Instant>,
    needs_monitor_refresh: bool,

    focus: PaneFocus,
    input_mode: InputMode,
    show_help: bool,
    pending_g: bool,

    browser_dir: PathBuf,
    browser_query: String,
    browser_all_entries: Vec<BrowserEntry>,
    browser_view_indices: Vec<usize>,
    browser_selected: usize,
    browser_list_state: ListState,
    filter_pending: bool,
    filter_cancel: Option<CancellationToken>,

    transition: TransitionState,
    aspect_mode: AspectMode,
    selected_preview: Option<ImagePreview>,
    image_dim_cache: HashMap<PathBuf, Option<(u32, u32)>>,

    playlist: Vec<PathBuf>,
    playlist_index: usize,
    playlist_running: bool,
    playlist_interval: Duration,
    last_playlist_tick: Instant,

    ipc_in_flight: bool,
    apply_all_monitors: bool,

    logs: VecDeque<LogEntry>,
    log_scroll: usize,
    event_tx: UnboundedSender<AppEvent>,
}

impl AppState {
    fn new(event_tx: UnboundedSender<AppEvent>) -> Self {
        let mut browser_list_state = ListState::default();
        browser_list_state.select(Some(0));
        let mut monitor_list_state = ListState::default();
        monitor_list_state.select(Some(0));

        Self {
            should_quit: false,
            status: String::from("Bootstrapping native backend"),
            status_level: StatusLevel::Info,
            frame_count: 0,
            last_frame: Instant::now(),
            backend: BackendRuntime::default(),
            monitors: Vec::new(),
            monitor_selected: 0,
            monitor_list_state,
            last_monitor_refresh: None,
            needs_monitor_refresh: true,
            focus: PaneFocus::Browser,
            input_mode: InputMode::Normal,
            show_help: false,
            pending_g: false,
            browser_dir: preferred_initial_browser_dir(),
            browser_query: String::new(),
            browser_all_entries: Vec::new(),
            browser_view_indices: Vec::new(),
            browser_selected: 0,
            browser_list_state,
            filter_pending: false,
            filter_cancel: None,
            transition: TransitionState::default(),
            aspect_mode: AspectMode::Fit,
            selected_preview: None,
            image_dim_cache: HashMap::new(),
            playlist: Vec::new(),
            playlist_index: 0,
            playlist_running: false,
            playlist_interval: Duration::from_secs(15),
            last_playlist_tick: Instant::now(),
            ipc_in_flight: false,
            apply_all_monitors: false,
            logs: VecDeque::with_capacity(LOG_CAPACITY),
            log_scroll: 0,
            event_tx,
        }
    }

    fn set_status(&mut self, level: StatusLevel, message: impl Into<String>) {
        self.status = message.into();
        self.status_level = level;
    }

    fn selected_browser_entry(&self) -> Option<&BrowserEntry> {
        let idx = *self.browser_view_indices.get(self.browser_selected)?;
        self.browser_all_entries.get(idx)
    }

    fn push_log(&mut self, level: UiLogLevel, message: impl Into<String>) {
        if self.logs.len() >= LOG_CAPACITY {
            self.logs.pop_front();
        }

        self.logs.push_back(LogEntry {
            at: Instant::now(),
            level,
            message: message.into(),
        });

        self.log_scroll = 0;
    }
}

static LOG_EVENT_TX: OnceLock<UnboundedSender<AppEvent>> = OnceLock::new();

fn forward_common_log(level: common::log::Filter, message: &str) {
    if let Some(tx) = LOG_EVENT_TX.get() {
        let _ = tx.send(AppEvent::Log(
            UiLogLevel::from_common(level),
            message.to_owned(),
        ));
    }
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, LeaveAlternateScreen);
    }
}

#[derive(Debug, Deserialize)]
struct HyprMonitor {
    name: String,
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    #[serde(default)]
    focused: bool,
}

#[derive(Debug)]
struct AspectSimulation {
    target_width: u32,
    target_height: u32,
    bars_x: u32,
    bars_y: u32,
    crop_x: u32,
    crop_y: u32,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> io::Result<()> {
    run_app().await
}

async fn run_app() -> io::Result<()> {
    enable_raw_mode()?;
    let _guard = TerminalGuard;

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let (event_tx, mut event_rx): (UnboundedSender<AppEvent>, UnboundedReceiver<AppEvent>) =
        mpsc::unbounded_channel();

    let _ = LOG_EVENT_TX.set(event_tx.clone());
    common::log::set_hook(Some(forward_common_log));

    let mut state = AppState::new(event_tx);

    if let Err(err) = reload_browser_directory(&mut state) {
        state.set_status(
            StatusLevel::Error,
            format!("Browser initialization failed: {err:#}"),
        );
    }

    start_native_backend(&mut state);

    let mut tick = time::interval(Duration::from_millis(TICK_RATE_MS));

    while !state.should_quit {
        tokio::select! {
            _ = tick.tick() => {
                handle_input(&mut state).await?;
                refresh_monitors_if_due(&mut state).await;
                poll_backend_status(&mut state).await;
                run_playlist_tick(&mut state).await;
                terminal.draw(|frame| draw_ui(frame, &mut state))?;
                state.frame_count = state.frame_count.saturating_add(1);
                state.last_frame = Instant::now();
            }
            Some(event) = event_rx.recv() => {
                handle_app_event(&mut state, event).await;
            }
        }
    }

    Ok(())
}

async fn handle_app_event(state: &mut AppState, event: AppEvent) {
    match event {
        AppEvent::ApplyBrowserFilter => {
            apply_browser_filter(state);
            state.filter_pending = false;
        }
        AppEvent::ApplyFinished(result) => {
            state.ipc_in_flight = false;
            match result {
                Ok(message) => {
                    state.push_log(UiLogLevel::Info, message.clone());
                    state.set_status(StatusLevel::Ok, message);
                }
                Err(err) => {
                    state.push_log(UiLogLevel::Error, format!("apply request failed: {err:#}"));
                    state.set_status(
                        StatusLevel::Error,
                        format!("IPC request failed: {err:#}. Press 's' to start/recover daemon."),
                    )
                }
            }
        }
        AppEvent::DaemonExited(result) => match result {
            Ok(()) => {
                let graceful = state
                    .backend
                    .shutdown
                    .as_ref()
                    .is_some_and(CancellationToken::is_cancelled);
                state.backend.shutdown = None;
                state.backend.task = None;
                if graceful {
                    state.backend.state = DaemonState::Stopped;
                    state.push_log(UiLogLevel::Info, "daemon stopped");
                    state.set_status(StatusLevel::Warn, "Daemon stopped");
                    if state.backend.restart_requested {
                        state.backend.restart_requested = false;
                        start_native_backend(state);
                    }
                } else {
                    state.backend.state = DaemonState::Crashed;
                    state.push_log(UiLogLevel::Error, "daemon exited unexpectedly");
                    state.set_status(StatusLevel::Error, "Daemon exited unexpectedly");
                }
            }
            Err(err) => {
                state.backend.shutdown = None;
                state.backend.task = None;
                state.backend.state = DaemonState::Crashed;
                state.backend.last_error = Some(err.clone());
                state.push_log(UiLogLevel::Error, format!("daemon crashed: {err}"));
                state.set_status(StatusLevel::Error, format!("Daemon crashed: {err}"));
            }
        },
        AppEvent::Log(level, message) => {
            state.push_log(level, message);
        }
    }
}

async fn handle_input(state: &mut AppState) -> io::Result<()> {
    while event::poll(Duration::from_millis(0))? {
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            if state.show_help {
                state.show_help = false;
                state.set_status(StatusLevel::Info, "Help overlay closed");
                continue;
            }

            if state.input_mode == InputMode::Search {
                handle_search_input(state, key.code, key.modifiers);
                continue;
            }

            let plain_g = matches!(key.code, KeyCode::Char('g')) && key.modifiers.is_empty();
            if !plain_g {
                state.pending_g = false;
            }

            match key.code {
                KeyCode::Char('q') => {
                    state.should_quit = true;
                    state.set_status(StatusLevel::Info, "Exiting Vellum");
                }
                KeyCode::Esc => {
                    state.input_mode = InputMode::Normal;
                    state.pending_g = false;
                    state.set_status(StatusLevel::Info, "Normal mode");
                }
                KeyCode::Char('?') => {
                    state.show_help = true;
                    state.set_status(StatusLevel::Info, "Help overlay opened");
                }
                KeyCode::Char('/') => {
                    if state.focus == PaneFocus::Browser {
                        state.input_mode = InputMode::Search;
                        state.set_status(StatusLevel::Info, "Search mode");
                    }
                }
                KeyCode::Char('b') | KeyCode::Char('s') => start_native_backend(state),
                KeyCode::Char('S') => stop_native_backend(state).await,
                KeyCode::Char('R') => restart_native_backend(state).await,
                KeyCode::Char('r') => {
                    state.needs_monitor_refresh = true;
                    state.set_status(StatusLevel::Info, "Manual monitor refresh requested");
                }
                KeyCode::Char('x') => {
                    state.aspect_mode = state.aspect_mode.next();
                    state.set_status(
                        StatusLevel::Info,
                        format!("Aspect mode: {}", state.aspect_mode.as_str()),
                    );
                }
                KeyCode::Char('a') => {
                    state.apply_all_monitors = !state.apply_all_monitors;
                    let mode = if state.apply_all_monitors {
                        "all monitors"
                    } else {
                        "selected monitor"
                    };
                    state.push_log(UiLogLevel::Info, format!("apply scope changed to {mode}"));
                    state.set_status(StatusLevel::Info, format!("Apply scope: {mode}"));
                }
                KeyCode::Char(' ') => {
                    state.playlist_running = !state.playlist_running;
                    state.last_playlist_tick = Instant::now();
                    if state.playlist_running {
                        state.set_status(StatusLevel::Ok, "Playlist enabled");
                    } else {
                        state.set_status(StatusLevel::Info, "Playlist paused");
                    }
                }
                KeyCode::Char('p') => add_selected_image_to_playlist(state),
                KeyCode::Char('c') => {
                    state.playlist.clear();
                    state.playlist_index = 0;
                    state.set_status(StatusLevel::Info, "Playlist cleared");
                }
                KeyCode::Char('+') | KeyCode::Char('=') => {
                    let secs = state.playlist_interval.as_secs();
                    state.playlist_interval = Duration::from_secs((secs + 5).min(600));
                    state.set_status(
                        StatusLevel::Info,
                        format!("Playlist interval: {}s", state.playlist_interval.as_secs()),
                    );
                }
                KeyCode::Char('-') => {
                    let secs = state.playlist_interval.as_secs();
                    state.playlist_interval = Duration::from_secs(secs.saturating_sub(5).max(5));
                    state.set_status(
                        StatusLevel::Info,
                        format!("Playlist interval: {}s", state.playlist_interval.as_secs()),
                    );
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    match state.focus {
                        PaneFocus::Browser => {
                            move_browser_selection(state, -NAV_HALF_PAGE_STEP);
                            refresh_selected_preview(state);
                        }
                        PaneFocus::Monitor => move_monitor_selection(state, -NAV_HALF_PAGE_STEP),
                        PaneFocus::Transition => {
                            state.transition.selected_field =
                                state.transition.selected_field.saturating_sub(2);
                        }
                        PaneFocus::Logs => {
                            state.log_scroll = (state.log_scroll + 8).min(state.logs.len());
                        }
                    }
                }
                KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    match state.focus {
                        PaneFocus::Browser => {
                            move_browser_selection(state, NAV_HALF_PAGE_STEP);
                            refresh_selected_preview(state);
                        }
                        PaneFocus::Monitor => move_monitor_selection(state, NAV_HALF_PAGE_STEP),
                        PaneFocus::Transition => {
                            state.transition.selected_field =
                                (state.transition.selected_field + 2).min(3);
                        }
                        PaneFocus::Logs => {
                            state.log_scroll = state.log_scroll.saturating_sub(8);
                        }
                    }
                }
                KeyCode::Tab | KeyCode::Char('l') => {
                    state.focus = state.focus.next();
                    state.set_status(
                        StatusLevel::Info,
                        format!("Focus: {}", state.focus.as_str()),
                    );
                }
                KeyCode::BackTab | KeyCode::Char('h') => {
                    state.focus = state.focus.prev();
                    state.set_status(
                        StatusLevel::Info,
                        format!("Focus: {}", state.focus.as_str()),
                    );
                }
                KeyCode::Up | KeyCode::Char('k') => match state.focus {
                    PaneFocus::Browser => {
                        move_browser_selection(state, -1);
                        refresh_selected_preview(state);
                    }
                    PaneFocus::Monitor => move_monitor_selection(state, -1),
                    PaneFocus::Transition => state.transition.select_prev_field(),
                    PaneFocus::Logs => {
                        state.log_scroll = (state.log_scroll + 1).min(state.logs.len());
                    }
                },
                KeyCode::Down | KeyCode::Char('j') => match state.focus {
                    PaneFocus::Browser => {
                        move_browser_selection(state, 1);
                        refresh_selected_preview(state);
                    }
                    PaneFocus::Monitor => move_monitor_selection(state, 1),
                    PaneFocus::Transition => state.transition.select_next_field(),
                    PaneFocus::Logs => {
                        state.log_scroll = state.log_scroll.saturating_sub(1);
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
                KeyCode::Char('u') => {
                    if state.focus == PaneFocus::Browser
                        && let Some(parent) = state.browser_dir.parent().map(Path::to_path_buf)
                    {
                        state.browser_dir = parent;
                        match reload_browser_directory(state) {
                            Ok(()) => {
                                state.set_status(StatusLevel::Info, "Moved to parent directory")
                            }
                            Err(err) => state.set_status(
                                StatusLevel::Error,
                                format!("Directory load failed: {err:#}"),
                            ),
                        }
                    }
                }
                KeyCode::Home => match state.focus {
                    PaneFocus::Browser => {
                        state.browser_selected = 0;
                        sync_browser_list_state(state);
                        refresh_selected_preview(state);
                    }
                    PaneFocus::Monitor => {
                        state.monitor_selected = 0;
                        sync_monitor_list_state(state);
                    }
                    PaneFocus::Transition => state.transition.selected_field = 0,
                    PaneFocus::Logs => state.log_scroll = state.logs.len(),
                },
                KeyCode::Char('g') if key.modifiers.is_empty() => {
                    if state.pending_g {
                        state.pending_g = false;
                        match state.focus {
                            PaneFocus::Browser => {
                                state.browser_selected = 0;
                                sync_browser_list_state(state);
                                refresh_selected_preview(state);
                            }
                            PaneFocus::Monitor => {
                                state.monitor_selected = 0;
                                sync_monitor_list_state(state);
                            }
                            PaneFocus::Transition => state.transition.selected_field = 0,
                            PaneFocus::Logs => state.log_scroll = state.logs.len(),
                        }
                    } else {
                        state.pending_g = true;
                    }
                }
                KeyCode::End | KeyCode::Char('G') => match state.focus {
                    PaneFocus::Browser => {
                        state.browser_selected = state.browser_view_indices.len().saturating_sub(1);
                        sync_browser_list_state(state);
                        refresh_selected_preview(state);
                    }
                    PaneFocus::Monitor => {
                        state.monitor_selected = state.monitors.len().saturating_sub(1);
                        sync_monitor_list_state(state);
                    }
                    PaneFocus::Transition => state.transition.selected_field = 3,
                    PaneFocus::Logs => state.log_scroll = 0,
                },
                KeyCode::Enter => {
                    if state.focus == PaneFocus::Browser {
                        activate_browser_selection(state).await;
                    }
                }
                KeyCode::F(8) => match save_profile(state, DEFAULT_PROFILE_NAME) {
                    Ok(path) => state.set_status(
                        StatusLevel::Ok,
                        format!("Saved profile: {}", path.display()),
                    ),
                    Err(err) => {
                        state.set_status(StatusLevel::Error, format!("Save failed: {err:#}"))
                    }
                },
                KeyCode::F(9) => match load_profile(state, DEFAULT_PROFILE_NAME) {
                    Ok(()) => state.set_status(StatusLevel::Ok, "Loaded profile: default"),
                    Err(err) => {
                        state.set_status(StatusLevel::Error, format!("Load failed: {err:#}"))
                    }
                },
                _ => {}
            }
        }
    }

    Ok(())
}

fn handle_search_input(state: &mut AppState, code: KeyCode, modifiers: KeyModifiers) {
    match code {
        KeyCode::Esc => {
            state.input_mode = InputMode::Normal;
            state.set_status(StatusLevel::Info, "Search canceled");
        }
        KeyCode::Enter => {
            state.input_mode = InputMode::Normal;
            state.set_status(
                StatusLevel::Info,
                format!("Filter: {}", state.browser_query),
            );
        }
        KeyCode::Backspace => {
            if !state.browser_query.is_empty() {
                state.browser_query.pop();
                schedule_browser_filter(state);
            }
        }
        KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
            state.browser_query.clear();
            schedule_browser_filter(state);
        }
        KeyCode::Char(ch) => {
            if !ch.is_control() {
                state.browser_query.push(ch);
                schedule_browser_filter(state);
            }
        }
        _ => {}
    }
}

fn move_browser_selection(state: &mut AppState, delta: isize) {
    if state.browser_view_indices.is_empty() {
        state.browser_selected = 0;
        state.browser_list_state.select(None);
        return;
    }

    let max = state.browser_view_indices.len().saturating_sub(1) as isize;
    state.browser_selected = (state.browser_selected as isize + delta).clamp(0, max) as usize;
    sync_browser_list_state(state);
}

fn move_monitor_selection(state: &mut AppState, delta: isize) {
    if state.monitors.is_empty() {
        state.monitor_selected = 0;
        state.monitor_list_state.select(None);
        return;
    }

    let max = state.monitors.len().saturating_sub(1) as isize;
    state.monitor_selected = (state.monitor_selected as isize + delta).clamp(0, max) as usize;
    sync_monitor_list_state(state);
}

fn sync_browser_list_state(state: &mut AppState) {
    if state.browser_view_indices.is_empty() {
        state.browser_selected = 0;
        state.browser_list_state.select(None);
        return;
    }

    state.browser_selected = state
        .browser_selected
        .min(state.browser_view_indices.len().saturating_sub(1));
    state
        .browser_list_state
        .select(Some(state.browser_selected));
}

fn sync_monitor_list_state(state: &mut AppState) {
    if state.monitors.is_empty() {
        state.monitor_selected = 0;
        state.monitor_list_state.select(None);
        return;
    }

    state.monitor_selected = state
        .monitor_selected
        .min(state.monitors.len().saturating_sub(1));
    state
        .monitor_list_state
        .select(Some(state.monitor_selected));
}

// Debounce uses a cancellable Tokio task instead of frame-time polling.
fn schedule_browser_filter(state: &mut AppState) {
    if let Some(token) = state.filter_cancel.take() {
        token.cancel();
    }

    if state.browser_query.is_empty() {
        apply_browser_filter(state);
        state.filter_pending = false;
        return;
    }

    state.filter_pending = true;
    let token = CancellationToken::new();
    let cancel = token.clone();
    let tx = state.event_tx.clone();
    state.filter_cancel = Some(token);

    tokio::spawn(async move {
        tokio::select! {
            _ = cancel.cancelled() => {}
            _ = time::sleep(Duration::from_millis(BROWSER_FILTER_DEBOUNCE_MS)) => {
                let _ = tx.send(AppEvent::ApplyBrowserFilter);
            }
        }
    });
}

fn start_native_backend(state: &mut AppState) {
    if matches!(
        state.backend.state,
        DaemonState::Starting | DaemonState::Running | DaemonState::Stopping
    ) {
        state.set_status(StatusLevel::Warn, "Daemon already active");
        return;
    }

    let namespace = state.backend.namespace.clone();
    let cancel = CancellationToken::new();
    state.backend.last_error = None;
    state.backend.restart_requested = false;
    state.backend.shutdown = Some(cancel.clone());
    state.backend.state = DaemonState::Starting;
    state.push_log(
        UiLogLevel::Info,
        format!("starting daemon in namespace '{namespace}'"),
    );
    state.set_status(
        StatusLevel::Info,
        format!("Starting daemon in namespace '{namespace}'"),
    );

    state.backend.task = Some(tokio::task::spawn_blocking(move || {
        let config = VellumServerConfig {
            namespace,
            quiet: true,
            ..VellumServerConfig::default()
        };
        let server = VellumServer::new(config);
        server.run().map_err(anyhow::Error::new)
    }));
    state.backend.state = DaemonState::Running;

    let tx = state.event_tx.clone();
    let namespace = state.backend.namespace.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = cancel.cancelled() => {
                let result = request_daemon_kill(namespace).await;
                if let Err(err) = result {
                    let _ = tx.send(AppEvent::Log(UiLogLevel::Warn, format!("daemon stop signal failed: {err:#}")));
                }
            }
            else => {}
        }
    });
}

async fn stop_native_backend(state: &mut AppState) {
    if !matches!(
        state.backend.state,
        DaemonState::Starting | DaemonState::Running
    ) {
        state.set_status(StatusLevel::Warn, "Daemon is not running");
        return;
    }

    if let Some(cancel) = state.backend.shutdown.as_ref() {
        cancel.cancel();
    }

    state.backend.state = DaemonState::Stopping;
    state.push_log(UiLogLevel::Info, "stopping daemon");
    state.set_status(StatusLevel::Warn, "Stopping daemon...");
}

async fn restart_native_backend(state: &mut AppState) {
    if matches!(
        state.backend.state,
        DaemonState::Starting | DaemonState::Running
    ) {
        state.backend.restart_requested = true;
        stop_native_backend(state).await;
        return;
    }

    start_native_backend(state);
}

async fn request_daemon_kill(namespace: String) -> Result<()> {
    run_blocking_with_timeout(IPC_QUERY_TIMEOUT, move || {
        let socket = IpcSocket::client(&namespace)
            .or_else(|_| IpcSocket::client(""))
            .map_err(anyhow::Error::new)
            .context("cannot connect to daemon socket for stop")?;

        RequestSend::Kill
            .send(&socket)
            .map_err(anyhow::Error::new)
            .context("failed to send Kill request")?;
        Ok(())
    })
    .await
    .map_err(|err| anyhow!(err.to_string()))
}

async fn poll_backend_status(state: &mut AppState) {
    if state
        .backend
        .task
        .as_ref()
        .is_none_or(|task| !task.is_finished())
    {
        return;
    }

    let Some(task) = state.backend.task.take() else {
        return;
    };

    let event = match task.await {
        Ok(Ok(())) => AppEvent::DaemonExited(Ok(())),
        Ok(Err(err)) => AppEvent::DaemonExited(Err(err.to_string())),
        Err(err) => AppEvent::DaemonExited(Err(err.to_string())),
    };
    let _ = state.event_tx.send(event);
}

async fn refresh_monitors_if_due(state: &mut AppState) {
    let timed_due = state
        .last_monitor_refresh
        .is_none_or(|at| at.elapsed() >= MONITOR_REFRESH_INTERVAL);
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
            sync_monitor_list_state(state);
            state.last_monitor_refresh = Some(Instant::now());
            state.needs_monitor_refresh = false;
            state.set_status(StatusLevel::Ok, format!("Detected {count} monitor(s)"));
        }
        Err(err) => {
            state.needs_monitor_refresh = false;
            state.set_status(
                StatusLevel::Error,
                format!("Monitor discovery failed: {err:#}"),
            );
        }
    }
}

async fn discover_monitors() -> Result<Vec<MonitorEntry>> {
    match probe_hyprctl_monitors().await {
        Ok(monitors) if !monitors.is_empty() => return Ok(monitors),
        Ok(_) => {}
        Err(_) => {}
    }

    probe_wlr_randr_monitors().await
}

async fn probe_hyprctl_monitors() -> Result<Vec<MonitorEntry>> {
    let json = command_json("hyprctl", &["monitors", "-j"]).await?;
    let monitors: Vec<HyprMonitor> =
        serde_json::from_value(json).context("invalid hyprctl monitors JSON")?;

    Ok(monitors
        .into_iter()
        .map(|m| MonitorEntry {
            name: m.name,
            width: m.width,
            height: m.height,
            x: m.x,
            y: m.y,
            focused: m.focused,
        })
        .collect())
}

async fn probe_wlr_randr_monitors() -> Result<Vec<MonitorEntry>> {
    let json = command_json("wlr-randr", &["--json"]).await?;
    parse_wlr_randr_monitors(json)
}

async fn command_json(binary: &str, args: &[&str]) -> Result<Value> {
    let output = Command::new(binary)
        .args(args)
        .output()
        .await
        .with_context(|| format!("failed to execute {binary}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{binary} returned {}: {}", output.status, stderr.trim());
    }

    serde_json::from_slice(&output.stdout).with_context(|| format!("invalid JSON from {binary}"))
}

fn parse_wlr_randr_monitors(payload: Value) -> Result<Vec<MonitorEntry>> {
    let Value::Array(outputs) = payload else {
        bail!("wlr-randr JSON root must be an array");
    };

    let monitors = outputs
        .into_iter()
        .filter_map(|item| {
            let obj = item.as_object()?;
            let name = obj.get("name")?.as_str()?.to_owned();
            let (width, height) = extract_wlr_dimensions(obj)?;
            let x = obj
                .get("x")
                .and_then(Value::as_i64)
                .and_then(|v| i32::try_from(v).ok())
                .unwrap_or(0);
            let y = obj
                .get("y")
                .and_then(Value::as_i64)
                .and_then(|v| i32::try_from(v).ok())
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

fn extract_wlr_dimensions(object: &serde_json::Map<String, Value>) -> Option<(u32, u32)> {
    if let Some(mode) = object.get("current_mode").and_then(Value::as_object) {
        let w = mode
            .get("width")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())?;
        let h = mode
            .get("height")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())?;
        return Some((w, h));
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
            let w = mode
                .get("width")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())?;
            let h = mode
                .get("height")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())?;
            Some((w, h))
        })
}

fn preferred_initial_browser_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        let pictures = PathBuf::from(home).join("Pictures");
        if pictures.is_dir() {
            return pictures;
        }
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn reload_browser_directory(state: &mut AppState) -> Result<()> {
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

    for entry in fs::read_dir(&state.browser_dir)
        .with_context(|| format!("cannot read '{}'", state.browser_dir.display()))?
    {
        let entry = entry?;
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
    state.filter_pending = false;

    Ok(())
}

// Filter stores indices to avoid cloning BrowserEntry values during every query.
fn apply_browser_filter(state: &mut AppState) {
    if state.browser_query.is_empty() {
        state.browser_view_indices = (0..state.browser_all_entries.len()).collect();
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
            .enumerate()
            .filter_map(|(idx, entry)| {
                pattern
                    .score(
                        Utf32Str::new(entry.name.as_str(), &mut utf32_buf),
                        &mut matcher,
                    )
                    .map(|score| (score, idx))
            })
            .collect::<Vec<_>>();

        scored.sort_by(|a, b| {
            b.0.cmp(&a.0).then_with(|| {
                let lhs = &state.browser_all_entries[a.1].name;
                let rhs = &state.browser_all_entries[b.1].name;
                lhs.cmp(rhs)
            })
        });

        state.browser_view_indices = scored.into_iter().map(|(_, idx)| idx).collect();
    }

    state.browser_selected = state
        .browser_selected
        .min(state.browser_view_indices.len().saturating_sub(1));
    sync_browser_list_state(state);
    refresh_selected_preview(state);
}

fn refresh_selected_preview(state: &mut AppState) {
    let Some(entry) = state.selected_browser_entry() else {
        state.selected_preview = None;
        return;
    };

    if entry.is_dir {
        state.selected_preview = None;
        return;
    }

    let image_path = entry.path.clone();

    let dims = if let Some(cached) = state.image_dim_cache.get(&image_path) {
        *cached
    } else {
        let probed = image::image_dimensions(&image_path).ok();
        state.image_dim_cache.insert(image_path.clone(), probed);
        probed
    };

    state.selected_preview = dims.map(|(width, height)| ImagePreview {
        path: image_path,
        width,
        height,
    });
}

fn add_selected_image_to_playlist(state: &mut AppState) {
    let Some(entry) = state.selected_browser_entry() else {
        state.set_status(StatusLevel::Warn, "No browser entry selected");
        return;
    };

    if entry.is_dir {
        state.set_status(StatusLevel::Warn, "Playlist accepts image files only");
        return;
    }

    if state.playlist.iter().any(|path| path == &entry.path) {
        state.set_status(StatusLevel::Warn, "Selected image already in playlist");
        return;
    }

    state.playlist.push(entry.path.clone());
    state.set_status(
        StatusLevel::Ok,
        format!("Playlist size: {}", state.playlist.len()),
    );
}

async fn run_playlist_tick(state: &mut AppState) {
    if state.ipc_in_flight || !state.playlist_running || state.playlist.is_empty() {
        return;
    }

    if state.last_playlist_tick.elapsed() < state.playlist_interval {
        return;
    }

    let index = state.playlist_index % state.playlist.len();
    let path = state.playlist[index].clone();
    state.playlist_index = (state.playlist_index + 1) % state.playlist.len();
    state.last_playlist_tick = Instant::now();

    request_apply_for_path(state, path);
}

async fn activate_browser_selection(state: &mut AppState) {
    let Some(entry) = state.selected_browser_entry().cloned() else {
        state.set_status(StatusLevel::Warn, "No browser entry selected");
        return;
    };

    if entry.is_dir {
        state.browser_dir = entry.path;
        state.browser_query.clear();
        if let Some(token) = state.filter_cancel.take() {
            token.cancel();
        }

        match reload_browser_directory(state) {
            Ok(()) => {
                if entry.is_parent {
                    state.set_status(StatusLevel::Info, "Moved to parent directory");
                } else {
                    state.set_status(StatusLevel::Info, "Opened directory");
                }
            }
            Err(err) => {
                state.set_status(
                    StatusLevel::Error,
                    format!("Directory open failed: {err:#}"),
                );
            }
        }
        return;
    }

    request_apply_for_path(state, entry.path);
}

fn request_apply_for_path(state: &mut AppState, file_path: PathBuf) {
    if state.ipc_in_flight {
        state.set_status(StatusLevel::Warn, "Previous IPC operation still running");
        return;
    }

    state.ipc_in_flight = true;
    state.set_status(
        StatusLevel::Info,
        format!(
            "Applying {}...",
            file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("image")
        ),
    );

    let namespace = state.backend.namespace.clone();
    let preferred_output = state
        .monitors
        .get(state.monitor_selected)
        .map(|m| m.name.clone());
    let apply_all = state.apply_all_monitors;
    let transition = build_transition_from_state(&state.transition);
    let tx = state.event_tx.clone();

    // IPC is isolated from the draw/input loop and returns via channel event.
    tokio::spawn(async move {
        let result = perform_apply_request(
            file_path,
            transition,
            namespace,
            preferred_output,
            apply_all,
            tx.clone(),
        )
        .await;
        let _ = tx.send(AppEvent::ApplyFinished(result));
    });
}

async fn perform_apply_request(
    file_path: PathBuf,
    transition: Transition,
    namespace: String,
    preferred_output: Option<String>,
    apply_all: bool,
    event_tx: UnboundedSender<AppEvent>,
) -> std::result::Result<String, ApplyError> {
    let target = query_apply_target(&namespace, preferred_output, apply_all).await?;
    let batch = apply_wallpaper_request(file_path.clone(), transition, target).await?;

    for failure in &batch.failures {
        let _ = event_tx.send(AppEvent::Log(UiLogLevel::Warn, failure.clone()));
    }

    if batch.succeeded == 0 {
        return Err(ApplyError::Other(anyhow!(
            "failed to apply wallpaper to every monitor"
        )));
    }

    Ok(format!(
        "Applied {} monitor(s), {} failed: {}",
        batch.succeeded,
        batch.failures.len(),
        file_path.display()
    ))
}

async fn query_apply_target(
    namespace: &str,
    preferred_output: Option<String>,
    apply_all: bool,
) -> std::result::Result<ApplyTarget, ApplyError> {
    let namespace = namespace.to_owned();

    run_blocking_with_timeout(IPC_QUERY_TIMEOUT, move || {
        let (resolved_namespace, socket) = connect_daemon_socket(&namespace)?;
        RequestSend::Query
            .send(&socket)
            .map_err(anyhow::Error::new)
            .context("failed to send query request")?;

        let answer = Answer::receive(
            socket
                .recv()
                .map_err(anyhow::Error::new)
                .context("failed to receive daemon query response")?,
        );

        let Answer::Info(outputs) = answer else {
            bail!("unexpected daemon response to query request");
        };

        let selected_outputs = if apply_all {
            outputs.iter().collect::<Vec<_>>()
        } else if let Some(name) = preferred_output {
            outputs
                .iter()
                .filter(|output| output.name.as_ref() == name)
                .collect::<Vec<_>>()
        } else {
            outputs.iter().take(1).collect::<Vec<_>>()
        };

        if selected_outputs.is_empty() {
            bail!("no output information available from daemon");
        }

        Ok(ApplyTarget {
            namespace: resolved_namespace,
            monitors: selected_outputs
                .into_iter()
                .map(|output| MonitorApplyTarget {
                    output: output.name.to_string(),
                    dim: output.real_dim(),
                    format: output.pixel_format,
                })
                .collect(),
        })
    })
    .await
}

fn connect_daemon_socket(preferred_namespace: &str) -> Result<(String, IpcSocket)> {
    if let Ok(socket) = IpcSocket::client(preferred_namespace) {
        return Ok((preferred_namespace.to_owned(), socket));
    }

    if preferred_namespace.is_empty() {
        bail!("daemon socket unavailable in default namespace");
    }

    if let Ok(socket) = IpcSocket::client("") {
        return Ok((String::new(), socket));
    }

    if let Ok(namespaces) = IpcSocket::all_namespaces() {
        for namespace in namespaces {
            if let Ok(socket) = IpcSocket::client(&namespace) {
                return Ok((namespace, socket));
            }
        }
    }

    bail!(
        "daemon socket not found for namespace '{preferred_namespace}' and no fallback namespace was reachable"
    )
}

async fn apply_wallpaper_request(
    file_path: PathBuf,
    transition: Transition,
    target: ApplyTarget,
) -> std::result::Result<ApplyBatchResult, ApplyError> {
    let image_path = Arc::new(file_path);
    let namespace = Arc::new(target.namespace);
    let mode = Arc::new(String::from("fit"));
    let filter = Arc::new(String::from("lanczos3"));

    let mut build_jobs = JoinSet::new();
    for monitor in target.monitors {
        let image_path = image_path.clone();
        let output = monitor.output;
        let dim = monitor.dim;
        let pixel_format = monitor.format;

        build_jobs.spawn(async move {
            run_blocking_with_timeout(IPC_APPLY_TIMEOUT, move || {
                let decoded = ImageReader::open(image_path.as_ref())
                    .map_err(anyhow::Error::new)
                    .with_context(|| format!("cannot open image '{}'", image_path.display()))?
                    .decode()
                    .map_err(anyhow::Error::new)
                    .with_context(|| format!("cannot decode image '{}'", image_path.display()))?;

                let resized = decoded.resize_exact(dim.0, dim.1, FilterType::Lanczos3);
                let (img_bytes, format) =
                    convert_dynamic_image_for_pixel_format(resized, pixel_format);

                Ok::<PreparedMonitorApply, anyhow::Error>(PreparedMonitorApply {
                    output,
                    img: ImgSend {
                        path: image_path.to_string_lossy().to_string(),
                        dim,
                        format,
                        img: img_bytes.into_boxed_slice(),
                    },
                })
            })
            .await
        });
    }

    let mut prepared = Vec::new();
    let mut failures = Vec::new();

    while let Some(result) = build_jobs.join_next().await {
        match result {
            Ok(Ok(apply)) => prepared.push(apply),
            Ok(Err(err)) => failures.push(format!("monitor build failed: {err:#}")),
            Err(err) => failures.push(format!("monitor build task failed: {err}")),
        }
    }

    let mut send_jobs = JoinSet::new();
    for prepared in prepared {
        let namespace = namespace.clone();
        let mode = mode.clone();
        let filter = filter.clone();
        let transition = duplicate_transition(&transition);

        send_jobs.spawn(async move {
            let output_name = prepared.output.clone();
            let output_for_builder = output_name.clone();
            let output_for_result = output_name.clone();
            let send_result = run_blocking_with_timeout(IPC_APPLY_TIMEOUT, move || {
                let mut builder = ImageRequestBuilder::new(transition)
                    .map_err(anyhow::Error::new)
                    .context("request mmap failed")?;

                builder.push(
                    prepared.img,
                    namespace.as_ref(),
                    mode.as_ref(),
                    filter.as_ref(),
                    &[output_for_builder.clone()],
                    None,
                );

                let socket = IpcSocket::client(namespace.as_ref())
                    .map_err(anyhow::Error::new)
                    .context("failed to connect to daemon IPC socket")?;

                RequestSend::Img(builder.build())
                    .send(&socket)
                    .map_err(anyhow::Error::new)
                    .context("failed to send image request")?;

                let _ = Answer::receive(
                    socket
                        .recv()
                        .map_err(anyhow::Error::new)
                        .context("failed to receive daemon apply response")?,
                );

                Ok::<String, anyhow::Error>(output_for_result)
            })
            .await;

            match send_result {
                Ok(name) => Ok(name),
                Err(err) => Err(format!("output '{output_name}' failed: {err:#}")),
            }
        });
    }

    let mut succeeded = 0usize;
    while let Some(result) = send_jobs.join_next().await {
        match result {
            Ok(Ok(_name)) => succeeded += 1,
            Ok(Err(err)) => failures.push(err),
            Err(err) => failures.push(format!("output send task failed: {err}")),
        }
    }

    Ok(ApplyBatchResult {
        succeeded,
        failures,
    })
}

struct PreparedMonitorApply {
    output: String,
    img: ImgSend,
}

#[derive(Debug)]
struct ApplyBatchResult {
    succeeded: usize,
    failures: Vec<String>,
}

fn duplicate_transition(transition: &Transition) -> Transition {
    Transition {
        transition_type: transition.transition_type,
        duration: transition.duration,
        step: transition.step,
        fps: transition.fps,
        angle: transition.angle,
        pos: transition.pos.clone(),
        bezier: transition.bezier,
        wave: transition.wave,
        invert_y: transition.invert_y,
    }
}

async fn run_blocking_with_timeout<T, F>(
    timeout: Duration,
    f: F,
) -> std::result::Result<T, ApplyError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    let handle = tokio::task::spawn_blocking(f);
    match time::timeout(timeout, handle).await {
        Ok(joined) => {
            let inner = joined
                .map_err(anyhow::Error::new)
                .map_err(ApplyError::from)?;
            inner.map_err(ApplyError::from)
        }
        Err(_) => Err(ApplyError::Timeout(timeout.as_millis())),
    }
}

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
        step: NonZeroU8::new(2).unwrap_or(NonZeroU8::MIN),
        fps: state.fps,
        angle: 0.0,
        pos: Position::new(Coord::Percent(0.5), Coord::Percent(0.5)),
        bezier,
        wave: (10.0, 10.0),
        invert_y: false,
    }
}

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

fn profile_path(profile_name: &str) -> Result<PathBuf> {
    let dirs = ProjectDirs::from("com", "vellum", "vellum")
        .ok_or_else(|| anyhow!("cannot resolve profile directory"))?;
    let dir = dirs.config_dir().join("profiles");
    fs::create_dir_all(&dir)
        .with_context(|| format!("cannot create profile directory '{}'", dir.display()))?;
    Ok(dir.join(format!("{profile_name}.json")))
}

fn save_profile(state: &AppState, profile_name: &str) -> Result<PathBuf> {
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
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
        playlist_interval_secs: state.playlist_interval.as_secs(),
        selected_monitor: state
            .monitors
            .get(state.monitor_selected)
            .map(|m| m.name.clone()),
        aspect_mode: state.aspect_mode,
    };

    let json = serde_json::to_string_pretty(&data).context("serialize profile")?;
    fs::write(&path, json).with_context(|| format!("write profile '{}'", path.display()))?;
    Ok(path)
}

fn load_profile(state: &mut AppState, profile_name: &str) -> Result<()> {
    let path = profile_path(profile_name)?;
    let bytes = fs::read(&path).with_context(|| format!("read profile '{}'", path.display()))?;
    let data: ProfileData = serde_json::from_slice(&bytes).context("parse profile JSON")?;

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

    reload_browser_directory(state)?;

    if let Some(selected_name) = data.selected_monitor
        && let Some(idx) = state.monitors.iter().position(|m| m.name == selected_name)
    {
        state.monitor_selected = idx;
    }

    sync_monitor_list_state(state);

    Ok(())
}

fn draw_ui(frame: &mut Frame<'_>, state: &mut AppState) {
    frame.render_widget(
        Block::default().style(Style::default().bg(COLOR_BG)),
        frame.area(),
    );

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(frame.area());

    draw_header(frame, layout[0], state);
    draw_body(frame, layout[1], state);
    draw_footer(frame, layout[2], state);

    if state.show_help {
        draw_help_overlay(frame, state);
    }
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);

    let tabs = Tabs::new(vec![" Browser ", " Monitor ", " Transition ", " Logs "])
        .select(match state.focus {
            PaneFocus::Browser => 0,
            PaneFocus::Monitor => 1,
            PaneFocus::Transition => 2,
            PaneFocus::Logs => 3,
        })
        .style(Style::default().fg(COLOR_MUTED).bg(COLOR_PANEL))
        .highlight_style(
            Style::default()
                .fg(COLOR_BG)
                .bg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD),
        )
        .divider(" ")
        .block(
            Block::default()
                .title(Line::from(vec![
                    Span::styled(
                        " VELLUM ",
                        Style::default()
                            .fg(COLOR_ACCENT_ALT)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        " keyboard wallpaper manager ",
                        Style::default().fg(COLOR_MUTED),
                    ),
                ]))
                .style(Style::default().bg(COLOR_PANEL))
                .borders(Borders::ALL)
                .border_set(border::ROUNDED)
                .border_style(Style::default().fg(COLOR_MUTED)),
        );

    frame.render_widget(tabs, columns[0]);

    let right = Line::from(vec![
        Span::styled(" MODE ", Style::default().fg(COLOR_MUTED)),
        Span::styled(
            format!("{} ", state.input_mode.as_str()),
            Style::default()
                .fg(if state.input_mode == InputMode::Search {
                    COLOR_WARN
                } else {
                    COLOR_OK
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("| BACKEND ", Style::default().fg(COLOR_MUTED)),
        Span::styled(
            state.backend.state.as_str(),
            Style::default()
                .fg(state.backend.state.color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | OUT ", Style::default().fg(COLOR_MUTED)),
        Span::styled(
            state.monitors.len().to_string(),
            Style::default().fg(COLOR_TEXT).add_modifier(Modifier::BOLD),
        ),
    ]);

    frame.render_widget(
        Paragraph::new(right).block(
            Block::default()
                .style(Style::default().bg(COLOR_PANEL))
                .borders(Borders::ALL)
                .border_set(border::ROUNDED)
                .border_style(Style::default().fg(COLOR_MUTED)),
        ),
        columns[1],
    );
}

fn draw_body(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(28),
            Constraint::Percentage(24),
            Constraint::Percentage(24),
            Constraint::Percentage(24),
        ])
        .split(area);

    draw_browser_pane(frame, columns[0], state);
    draw_monitor_pane(frame, columns[1], state);
    draw_transition_pane(frame, columns[2], state);
    draw_logs_pane(frame, columns[3], state);
}

fn border_style_for_focus(active: PaneFocus, pane: PaneFocus) -> Style {
    if active == pane {
        Style::default()
            .fg(COLOR_ACCENT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Rgb(70, 78, 104))
    }
}

fn padded(inner: Rect) -> Rect {
    inner.inner(Margin {
        vertical: 1,
        horizontal: 1,
    })
}

fn draw_browser_pane(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Browser ",
                Style::default().fg(COLOR_TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} items", state.browser_view_indices.len()),
                Style::default().fg(COLOR_MUTED),
            ),
        ]))
        .style(Style::default().bg(COLOR_PANEL))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(border_style_for_focus(state.focus, PaneFocus::Browser));

    frame.render_widget(block.clone(), area);

    let inner = padded(block.inner(area));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(1),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Dir: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(
                state.browser_dir.display().to_string(),
                Style::default().fg(COLOR_TEXT),
            ),
        ]))
        .style(Style::default().bg(COLOR_PANEL)),
        rows[0],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Find: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(
                if state.browser_query.is_empty() {
                    Cow::Borrowed("(none)")
                } else {
                    Cow::Borrowed(state.browser_query.as_str())
                },
                Style::default().fg(COLOR_ACCENT),
            ),
            Span::raw("  "),
            Span::styled(
                if state.filter_pending {
                    "syncing"
                } else {
                    "ready"
                },
                Style::default().fg(if state.filter_pending {
                    COLOR_WARN
                } else {
                    COLOR_OK
                }),
            ),
        ]))
        .style(Style::default().bg(COLOR_PANEL_DIM)),
        rows[1],
    );

    let items = if state.browser_view_indices.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No matching entries",
            Style::default().fg(COLOR_MUTED),
        )))]
    } else {
        state
            .browser_view_indices
            .iter()
            .filter_map(|idx| state.browser_all_entries.get(*idx))
            .map(|entry| {
                let icon = if entry.is_parent {
                    ".."
                } else if entry.is_dir {
                    "dir"
                } else {
                    "img"
                };

                ListItem::new(Line::from(vec![
                    Span::styled(format!("{icon:>3}  "), Style::default().fg(COLOR_MUTED)),
                    Span::styled(
                        entry.name.as_str(),
                        if entry.is_dir {
                            Style::default().fg(COLOR_DIR).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(COLOR_FILE)
                        },
                    ),
                ]))
            })
            .collect::<Vec<_>>()
    };

    let list = List::new(items)
        .style(Style::default().bg(COLOR_PANEL))
        .highlight_symbol(">> ")
        .highlight_style(
            Style::default()
                .bg(COLOR_SELECTION_BG)
                .fg(COLOR_TEXT)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_stateful_widget(list, rows[2], &mut state.browser_list_state);
}

fn draw_monitor_pane(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Monitor ",
                Style::default().fg(COLOR_TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled("outputs", Style::default().fg(COLOR_MUTED)),
        ]))
        .style(Style::default().bg(COLOR_PANEL))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(border_style_for_focus(state.focus, PaneFocus::Monitor));

    frame.render_widget(block.clone(), area);

    let inner = padded(block.inner(area));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(8)])
        .split(inner);

    let items = if state.monitors.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No monitor data yet (press r)",
            Style::default().fg(COLOR_MUTED),
        )))]
    } else {
        state
            .monitors
            .iter()
            .map(|monitor| {
                let focus = if monitor.focused { "*" } else { " " };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{focus} "), Style::default().fg(COLOR_ACCENT_ALT)),
                    Span::styled(
                        format!("{}  {}x{}", monitor.name, monitor.width, monitor.height),
                        Style::default().fg(COLOR_TEXT),
                    ),
                ]))
            })
            .collect()
    };

    let list = List::new(items)
        .style(Style::default().bg(COLOR_PANEL))
        .highlight_symbol(">> ")
        .highlight_style(
            Style::default()
                .bg(COLOR_SELECTION_BG)
                .fg(COLOR_TEXT)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_stateful_widget(list, rows[0], &mut state.monitor_list_state);

    frame.render_widget(
        Paragraph::new(render_monitor_details(state))
            .style(Style::default().fg(COLOR_TEXT).bg(COLOR_PANEL_DIM)),
        rows[1],
    );
}

fn draw_transition_pane(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Transition ",
                Style::default().fg(COLOR_TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled("runtime", Style::default().fg(COLOR_MUTED)),
        ]))
        .style(Style::default().bg(COLOR_PANEL))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(border_style_for_focus(state.focus, PaneFocus::Transition));

    frame.render_widget(block.clone(), area);

    let inner = padded(block.inner(area));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(7), Constraint::Min(5)])
        .split(inner);

    let data = [
        (
            "duration",
            format!("{} ms", state.transition.duration_ms),
            0,
        ),
        ("fps", state.transition.fps.to_string(), 1),
        (
            "easing",
            EASING_PRESETS[state.transition.easing_idx].to_owned(),
            2,
        ),
        (
            "effect",
            TRANSITION_EFFECTS[state.transition.effect_idx].to_owned(),
            3,
        ),
    ];

    let rows_data = data.into_iter().map(|(field, value, idx)| {
        let selected =
            state.focus == PaneFocus::Transition && state.transition.selected_field == idx;
        Row::new(vec![
            if selected {
                String::from(">>")
            } else {
                String::from("  ")
            },
            field.to_owned(),
            value,
        ])
        .style(if selected {
            Style::default()
                .bg(COLOR_SELECTION_BG)
                .fg(COLOR_TEXT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(COLOR_TEXT)
        })
    });

    let table = Table::new(
        rows_data,
        [
            Constraint::Length(3),
            Constraint::Length(10),
            Constraint::Min(10),
        ],
    )
    .header(
        Row::new(vec!["", "Field", "Value"]).style(
            Style::default()
                .fg(COLOR_MUTED)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .column_spacing(1)
    .style(Style::default().bg(COLOR_PANEL_DIM));

    frame.render_widget(table, rows[0]);

    let runtime = Paragraph::new(Line::from(vec![
        Span::styled("Backend: ", Style::default().fg(COLOR_MUTED)),
        Span::styled(
            state.backend.state.as_str(),
            Style::default()
                .fg(state.backend.state.color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | NS: ", Style::default().fg(COLOR_MUTED)),
        Span::styled(
            state.backend.namespace.as_str(),
            Style::default().fg(COLOR_TEXT),
        ),
        Span::raw("\n"),
        Span::styled("Playlist: ", Style::default().fg(COLOR_MUTED)),
        Span::styled(
            if state.playlist_running {
                "running"
            } else {
                "paused"
            },
            Style::default().fg(if state.playlist_running {
                COLOR_OK
            } else {
                COLOR_WARN
            }),
        ),
        Span::styled(" | size ", Style::default().fg(COLOR_MUTED)),
        Span::styled(
            state.playlist.len().to_string(),
            Style::default().fg(COLOR_TEXT),
        ),
        Span::styled(" | every ", Style::default().fg(COLOR_MUTED)),
        Span::styled(
            format!("{}s", state.playlist_interval.as_secs()),
            Style::default().fg(COLOR_TEXT),
        ),
        Span::raw("\n"),
        Span::styled(
            "s start | S stop | R restart | a all outputs | F8/F9 profile",
            Style::default().fg(COLOR_ACCENT),
        ),
    ]))
    .style(Style::default().bg(COLOR_PANEL_DIM));

    frame.render_widget(runtime, rows[1]);
}

fn draw_logs_pane(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Logs ",
                Style::default().fg(COLOR_TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} entries", state.logs.len()),
                Style::default().fg(COLOR_MUTED),
            ),
        ]))
        .style(Style::default().bg(COLOR_PANEL))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(border_style_for_focus(state.focus, PaneFocus::Logs));

    frame.render_widget(block.clone(), area);

    let inner = padded(block.inner(area));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(1)])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("source: common::log hook", Style::default().fg(COLOR_MUTED)),
            Span::styled(
                " | scroll: j/k, Ctrl+u/d",
                Style::default().fg(COLOR_ACCENT),
            ),
        ]))
        .style(Style::default().bg(COLOR_PANEL_DIM)),
        rows[0],
    );

    let viewport = rows[1].height.max(1) as usize;
    let len = state.logs.len();
    let end = len.saturating_sub(state.log_scroll);
    let start = end.saturating_sub(viewport);

    let lines = if len == 0 {
        vec![Line::from(Span::styled(
            "No log entries yet",
            Style::default().fg(COLOR_MUTED),
        ))]
    } else {
        state
            .logs
            .iter()
            .skip(start)
            .take(end.saturating_sub(start))
            .map(|entry| {
                let age_ms = entry.at.elapsed().as_millis();
                Line::from(vec![
                    Span::styled(
                        format!("[{age_ms:>6}ms] "),
                        Style::default().fg(COLOR_MUTED),
                    ),
                    Span::styled(
                        format!("{:>5} ", entry.level.as_str()),
                        Style::default()
                            .fg(entry.level.color())
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(entry.message.as_str(), Style::default().fg(COLOR_TEXT)),
                ])
            })
            .collect::<Vec<_>>()
    };

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(COLOR_PANEL_DIM)),
        rows[1],
    );
}

fn render_monitor_details(state: &AppState) -> String {
    let Some(monitor) = state.monitors.get(state.monitor_selected) else {
        return String::from("No selection");
    };

    let mut text = format!(
        "selected: {}\npos: ({}, {})\nresolution: {}x{}\nmode: {}",
        monitor.name,
        monitor.x,
        monitor.y,
        monitor.width,
        monitor.height,
        state.aspect_mode.as_str()
    );

    if let Some(preview) = &state.selected_preview {
        let sim = simulate_aspect(
            (preview.width, preview.height),
            (monitor.width, monitor.height),
            state.aspect_mode,
        );

        text.push_str(&format!(
            "\n\nimg: {}\nsrc: {}x{}\nsim: {}x{}\nbars: {}x{}\ncrop: {}x{}",
            preview.path.display(),
            preview.width,
            preview.height,
            sim.target_width,
            sim.target_height,
            sim.bars_x,
            sim.bars_y,
            sim.crop_x,
            sim.crop_y
        ));
    } else {
        text.push_str("\n\nselect an image in Browser");
    }

    text
}

fn simulate_aspect(image: (u32, u32), monitor: (u32, u32), mode: AspectMode) -> AspectSimulation {
    let (iw, ih) = (image.0.max(1) as f64, image.1.max(1) as f64);
    let (mw, mh) = (monitor.0.max(1) as f64, monitor.1.max(1) as f64);

    let scale = match mode {
        AspectMode::Fit => (mw / iw).min(mh / ih),
        AspectMode::Fill => (mw / iw).max(mh / ih),
        AspectMode::Stretch => 0.0,
    };

    let (target_width, target_height) = if matches!(mode, AspectMode::Stretch) {
        (monitor.0.max(1), monitor.1.max(1))
    } else {
        (
            (iw * scale).round().max(1.0) as u32,
            (ih * scale).round().max(1.0) as u32,
        )
    };

    AspectSimulation {
        target_width,
        target_height,
        bars_x: monitor.0.saturating_sub(target_width) / 2,
        bars_y: monitor.1.saturating_sub(target_height) / 2,
        crop_x: target_width.saturating_sub(monitor.0) / 2,
        crop_y: target_height.saturating_sub(monitor.1) / 2,
    }
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let frame_age_ms = state.last_frame.elapsed().as_millis();

    let line = Line::from(vec![
        Span::styled(
            state.status.as_str(),
            Style::default().fg(state.status_level.color()),
        ),
        Span::styled("  |  ", Style::default().fg(COLOR_MUTED)),
        Span::styled(
            format!("frames {}", state.frame_count),
            Style::default().fg(COLOR_MUTED),
        ),
        Span::styled("  ", Style::default().fg(COLOR_MUTED)),
        Span::styled(
            format!("{}ms", frame_age_ms),
            Style::default().fg(COLOR_MUTED),
        ),
        Span::styled("  |  ", Style::default().fg(COLOR_MUTED)),
        Span::styled(
            "/ search  h/l pane  j/k move  gg/G  Ctrl+u/d  Enter apply",
            Style::default().fg(COLOR_ACCENT),
        ),
    ]);

    frame.render_widget(
        Paragraph::new(line).block(
            Block::default()
                .style(Style::default().bg(COLOR_PANEL))
                .borders(Borders::ALL)
                .border_set(border::ROUNDED)
                .border_style(Style::default().fg(COLOR_MUTED)),
        ),
        area,
    );
}

fn draw_help_overlay(frame: &mut Frame<'_>, state: &AppState) {
    let popup = centered_rect(78, 74, frame.area());
    let help = format!(
        "VELLUM HELP\n\nMode: {} | Focus: {}\n\nGlobal\n- q quit\n- ? close/open help\n- b restart backend\n- r refresh monitors\n\nNavigation (Normal mode)\n- h / l focus pane\n- j / k move selection\n- gg / G jump top/bottom\n- Ctrl+u / Ctrl+d half page\n\nBrowser\n- Enter open/apply\n- / search mode\n- Esc leave search\n- u parent directory\n\nTransition\n- Left/Right adjust selected field\n\nPlaylist\n- Space toggle\n- p add selected image\n- c clear\n- +/- interval\n\nPress ?, Esc, Enter, or q to close",
        state.input_mode.as_str(),
        state.focus.as_str()
    );

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(help).block(
            Block::default()
                .title(" Quick Help ")
                .style(Style::default().bg(COLOR_PANEL))
                .borders(Borders::ALL)
                .border_set(border::ROUNDED)
                .border_style(Style::default().fg(COLOR_ACCENT_ALT)),
        ),
        popup,
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let v = Layout::default()
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
        .split(v[1])[1]
}
