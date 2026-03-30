use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use common::ipc::BgInfo;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use fuzzy_matcher::{FuzzyMatcher, skim::SkimMatcherV2};

use crate::backend::Backend;
use crate::backend::DaemonResourceUsage;
use crate::preview::{self, PreviewImage, PreviewRequest, PreviewResult};

const LOG_CAPACITY: usize = 128;
const BACKEND_SYNC_INTERVAL_TICKS: u8 = 5;
const PLAYLIST_INTERVAL_MIN_SECS: u64 = 10;
const PLAYLIST_INTERVAL_MAX_SECS: u64 = 99 * 3600;
const PLAYLIST_INTERVAL_DEFAULT_SECS: u64 = 30 * 60;
const PLAYLIST_ITEM_COUNT: usize = 3;
const PLAYLIST_STATE_FILENAME: &str = "playlist-state-v1.txt";
const FAVORITES_STATE_FILENAME: &str = "favorites-v1.txt";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalingMode {
    Fill,
    Fit,
    Crop,
    Center,
    Tile,
}

impl ScalingMode {
    pub const ALL: [Self; 5] = [Self::Fill, Self::Fit, Self::Crop, Self::Center, Self::Tile];
}

impl fmt::Display for ScalingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Fill => "Fill",
            Self::Fit => "Fit",
            Self::Crop => "Crop",
            Self::Center => "Center",
            Self::Tile => "Tile",
        };

        f.write_str(label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DaemonStatus {
    Running,
    #[default]
    Stopped,
    Crashed,
}

impl fmt::Display for DaemonStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Running => "Running",
            Self::Stopped => "Stopped",
            Self::Crashed => "Crashed",
        };

        f.write_str(label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Files,
    Scaling,
    Playlist,
}

impl Focus {
    fn next(self) -> Self {
        match self {
            Self::Files => Self::Scaling,
            Self::Scaling => Self::Playlist,
            Self::Playlist => Self::Files,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Files => Self::Scaling,
            Self::Scaling => Self::Files,
            Self::Playlist => Self::Scaling,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistSource {
    Workspace,
    Favorites,
}

impl PlaylistSource {
    pub fn toggle(self) -> Self {
        match self {
            Self::Workspace => Self::Favorites,
            Self::Favorites => Self::Workspace,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Workspace => "Workspace",
            Self::Favorites => "Favorites",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlaylistConfig {
    pub source: PlaylistSource,
    pub interval_secs: u64,
    pub running: bool,
    pub next_shuffle_at: Option<Instant>,
    pub last_wallpaper: Option<PathBuf>,
}

impl Default for PlaylistConfig {
    fn default() -> Self {
        Self {
            source: PlaylistSource::Favorites,
            interval_secs: PLAYLIST_INTERVAL_DEFAULT_SECS,
            running: false,
            next_shuffle_at: None,
            last_wallpaper: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlaylistDraft {
    source: PlaylistSource,
    interval_secs: u64,
    running: bool,
}

impl From<&PlaylistConfig> for PlaylistDraft {
    fn from(value: &PlaylistConfig) -> Self {
        Self {
            source: value.source,
            interval_secs: value.interval_secs,
            running: value.running,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Directory,
    File,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub kind: FileKind,
    pub supported: bool,
    pub favorite: bool,
}

impl FileEntry {
    fn new(path: PathBuf, kind: FileKind, supported: bool, favorite: bool) -> Self {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());

        Self {
            path,
            name,
            kind,
            supported,
            favorite,
        }
    }

    fn supports_filters(&self) -> bool {
        self.kind == FileKind::Directory || self.supported
    }
}

#[derive(Debug, Clone)]
pub struct Monitor {
    pub name: String,
    pub width: u16,
    pub height: u16,
    pub wallpaper: Option<PathBuf>,
}

impl Monitor {
    fn new(name: &str, width: u16, height: u16) -> Self {
        Self {
            name: name.to_string(),
            width,
            height,
            wallpaper: None,
        }
    }

    pub fn aspect_ratio(&self) -> f64 {
        self.width as f64 / self.height.max(1) as f64
    }
}

impl From<BgInfo> for Monitor {
    fn from(value: BgInfo) -> Self {
        let (width, height) = value.dim;
        Self {
            name: value.name.into(),
            width: width.min(u16::MAX as u32) as u16,
            height: height.min(u16::MAX as u32) as u16,
            wallpaper: match value.img {
                common::ipc::BgImg::Img(path) => Some(PathBuf::from(path.as_ref())),
                common::ipc::BgImg::Color(_) => None,
            },
        }
    }
}

pub struct App {
    pictures_root: PathBuf,
    pub current_path: PathBuf,
    pub browser_entries: Vec<FileEntry>,
    pub browser_filtered_indices: Vec<usize>,
    pub browser_selected: usize,
    pub favorites: HashSet<PathBuf>,
    pub search_active: bool,
    pub search_buffer: String,
    pub hide_unsupported: bool,
    pub favorites_only: bool,
    pub monitors: Vec<Monitor>,
    pub selected_monitor: usize,
    pub scaling_modes: Vec<ScalingMode>,
    pub selected_scaling_mode: usize,
    pub applied_scaling_mode: usize,
    pub daemon_status: DaemonStatus,
    pub daemon_resources: Option<DaemonResourceUsage>,
    pub logs: Vec<String>,
    pub focus: Focus,
    playlist_by_monitor: HashMap<String, PlaylistConfig>,
    pub playlist_selected: usize,
    playlist_draft: Option<PlaylistDraft>,
    playlist_draft_monitor: Option<String>,
    playlist_draft_dirty: bool,
    matcher: SkimMatcherV2,
    preview_request_tx: Sender<PreviewRequest>,
    preview_result_rx: Receiver<PreviewResult>,
    preview_request_seq: u64,
    preview_last_key: Option<PreviewRequestKey>,
    preview_status: String,
    preview_image: Option<PreviewImage>,
    backend_sync_tick: u8,
    awaiting_second_g: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreviewRequestKey {
    path: PathBuf,
    scaling: ScalingMode,
    width: u16,
    height: u16,
    monitor_name: String,
    monitor_width: u16,
    monitor_height: u16,
}

impl fmt::Debug for App {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("App")
            .field("current_path", &self.current_path)
            .field("browser_entries", &self.browser_entries.len())
            .field("browser_filtered_indices", &self.browser_filtered_indices)
            .field("browser_selected", &self.browser_selected)
            .field("favorites", &self.favorites)
            .field("search_active", &self.search_active)
            .field("search_buffer", &self.search_buffer)
            .field("hide_unsupported", &self.hide_unsupported)
            .field("favorites_only", &self.favorites_only)
            .field("monitors", &self.monitors)
            .field("selected_monitor", &self.selected_monitor)
            .field("scaling_modes", &self.scaling_modes)
            .field("selected_scaling_mode", &self.selected_scaling_mode)
            .field("applied_scaling_mode", &self.applied_scaling_mode)
            .field("daemon_status", &self.daemon_status)
            .field("daemon_resources", &self.daemon_resources)
            .field("logs", &self.logs)
            .field("focus", &self.focus)
            .field("playlist_by_monitor", &self.playlist_by_monitor)
            .field("playlist_selected", &self.playlist_selected)
            .field("playlist_draft", &self.playlist_draft)
            .field("playlist_draft_monitor", &self.playlist_draft_monitor)
            .field("playlist_draft_dirty", &self.playlist_draft_dirty)
            .field("preview_request_seq", &self.preview_request_seq)
            .field("preview_last_key", &self.preview_last_key)
            .field("preview_status", &self.preview_status)
            .field(
                "preview_image",
                &self.preview_image.as_ref().map(|_| "ready"),
            )
            .field("backend_sync_tick", &self.backend_sync_tick)
            .field("awaiting_second_g", &self.awaiting_second_g)
            .finish()
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let pictures_root = pictures_dir();
        let (preview_request_tx, preview_request_rx) = mpsc::channel();
        let (preview_result_tx, preview_result_rx) = mpsc::channel();
        preview::spawn_preview_worker(preview_request_rx, preview_result_tx);

        let mut app = Self {
            pictures_root: pictures_root.clone(),
            current_path: pictures_root,
            browser_entries: Vec::new(),
            browser_filtered_indices: Vec::new(),
            browser_selected: 0,
            favorites: HashSet::new(),
            search_active: false,
            search_buffer: String::new(),
            hide_unsupported: false,
            favorites_only: false,
            monitors: vec![
                Monitor::new("eDP-1", 2560, 1600),
                Monitor::new("DP-1", 1920, 1080),
            ],
            selected_monitor: 0,
            scaling_modes: ScalingMode::ALL.to_vec(),
            selected_scaling_mode: 1,
            applied_scaling_mode: 1,
            daemon_status: DaemonStatus::Stopped,
            daemon_resources: None,
            logs: vec!["[INFO] Vellum TUI ready".to_string()],
            focus: Focus::Files,
            playlist_by_monitor: HashMap::new(),
            playlist_selected: 0,
            playlist_draft: None,
            playlist_draft_monitor: None,
            playlist_draft_dirty: false,
            matcher: SkimMatcherV2::default(),
            preview_request_tx,
            preview_result_rx,
            preview_request_seq: 0,
            preview_last_key: None,
            preview_status: "Select an image file to preview".to_string(),
            preview_image: None,
            backend_sync_tick: 0,
            awaiting_second_g: false,
        };

        app.refresh_browser_entries();
        app
    }

    pub fn load_or_default() -> Self {
        let mut app = Self::new();
        app.load_favorites_state();
        app.load_playlist_state();
        app.refresh_browser_entries();
        app
    }

    pub fn handle_event(&mut self, event: Event, backend: &mut Backend) -> bool {
        match event {
            Event::Key(key) => self.handle_key_event(key, backend),
            Event::Resize(_, _) => {
                self.push_log("[INFO] Terminal resized".to_string());
                false
            }
            _ => false,
        }
    }

    pub fn handle_key_event(&mut self, key: KeyEvent, backend: &mut Backend) -> bool {
        if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
            return false;
        }

        if self.awaiting_second_g && key.code != KeyCode::Char('g') {
            self.awaiting_second_g = false;
        }

        if key.code == KeyCode::Char('q') {
            if self.focus == Focus::Playlist {
                self.apply_playlist_draft_if_dirty();
            }
            return true;
        }

        if self.search_active {
            return self.handle_search_key(key);
        }

        match key.code {
            KeyCode::Char('/') => {
                self.search_active = true;
                self.search_buffer.clear();
                self.focus = Focus::Files;
                self.push_log("[INFO] Search activated".to_string());
                false
            }
            KeyCode::Char('f') => {
                self.toggle_favorite_current();
                false
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                self.select_monitor_hotkey(c);
                false
            }
            KeyCode::Char('c') => {
                self.clear_selected_monitor(backend);
                false
            }
            KeyCode::Char('p') => {
                self.toggle_pause(backend);
                false
            }
            KeyCode::Char('v') => {
                self.hide_unsupported = !self.hide_unsupported;
                self.push_log(format!(
                    "[INFO] Unsupported formats {}",
                    if self.hide_unsupported {
                        "hidden"
                    } else {
                        "visible"
                    }
                ));
                self.refresh_browser_entries();
                false
            }
            KeyCode::Char('o') => {
                self.favorites_only = !self.favorites_only;
                self.push_log(format!(
                    "[INFO] Favorites filter {}",
                    if self.favorites_only {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ));
                self.refresh_browser_entries();
                false
            }
            KeyCode::Char('s') => {
                self.start_or_refresh_daemon(backend);
                false
            }
            KeyCode::Char('r') => {
                if self.focus == Focus::Playlist {
                    self.toggle_selected_playlist_running_draft();
                }
                false
            }
            KeyCode::Char('m') => {
                if self.focus == Focus::Playlist {
                    self.toggle_selected_playlist_source_draft();
                }
                false
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                if self.focus == Focus::Playlist {
                    self.adjust_selected_playlist_interval_draft(1);
                }
                false
            }
            KeyCode::Char('-') => {
                if self.focus == Focus::Playlist {
                    self.adjust_selected_playlist_interval_draft(-1);
                }
                false
            }
            KeyCode::Char('n') => {
                if self.focus == Focus::Playlist {
                    self.apply_playlist_draft_if_dirty();
                }
                self.trigger_selected_playlist_now();
                false
            }
            KeyCode::Tab => {
                if self.focus == Focus::Playlist {
                    self.apply_playlist_draft_if_dirty();
                }
                self.focus = self.focus.next();
                if self.focus == Focus::Playlist {
                    self.ensure_playlist_draft_loaded();
                }
                false
            }
            KeyCode::BackTab => {
                if self.focus == Focus::Playlist {
                    self.apply_playlist_draft_if_dirty();
                }
                self.focus = self.focus.previous();
                if self.focus == Focus::Playlist {
                    self.ensure_playlist_draft_loaded();
                }
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_previous();
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_next();
                false
            }
            KeyCode::Left => {
                match self.focus {
                    Focus::Files => self.go_to_parent_directory(),
                    Focus::Scaling => self.previous_scaling_mode(),
                    Focus::Playlist => self.handle_playlist_left_action(),
                }
                false
            }
            KeyCode::Right => {
                match self.focus {
                    Focus::Files => self.handle_files_right_action(),
                    Focus::Scaling => self.next_scaling_mode(),
                    Focus::Playlist => self.handle_playlist_right_action(),
                }
                false
            }
            KeyCode::Char('h') => {
                match self.focus {
                    Focus::Files => self.go_to_parent_directory(),
                    Focus::Scaling => self.previous_scaling_mode(),
                    Focus::Playlist => self.handle_playlist_left_action(),
                }
                false
            }
            KeyCode::Char('l') => {
                match self.focus {
                    Focus::Files => self.activate_selection(backend),
                    Focus::Scaling => self.next_scaling_mode(),
                    Focus::Playlist => self.handle_playlist_right_action(),
                }
                false
            }
            KeyCode::Backspace => {
                self.go_to_parent_directory();
                false
            }
            KeyCode::Char('g') => {
                if self.focus == Focus::Files {
                    if self.awaiting_second_g {
                        self.move_to_start();
                        self.awaiting_second_g = false;
                    } else {
                        self.awaiting_second_g = true;
                    }
                } else {
                    self.move_to_start();
                    self.awaiting_second_g = false;
                }
                false
            }
            KeyCode::Char('G') => {
                self.move_to_end();
                false
            }
            KeyCode::Home => {
                self.move_to_start();
                false
            }
            KeyCode::End => {
                self.move_to_end();
                false
            }
            KeyCode::PageUp => {
                self.page_up();
                false
            }
            KeyCode::PageDown => {
                self.page_down();
                false
            }
            KeyCode::Enter => {
                self.activate_selection(backend);
                false
            }
            KeyCode::Char('[') => {
                self.previous_scaling_mode();
                false
            }
            KeyCode::Char(']') => {
                self.next_scaling_mode();
                false
            }
            _ => false,
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.search_active = false;
                self.push_log(format!("[INFO] Search applied: {}", self.search_buffer));
                self.refresh_browser_entries();
                false
            }
            KeyCode::Backspace => {
                self.search_buffer.pop();
                self.refresh_browser_entries();
                false
            }
            KeyCode::Char(c)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.search_buffer.push(c);
                self.refresh_browser_entries();
                false
            }
            _ => false,
        }
    }

    fn move_previous(&mut self) {
        match self.focus {
            Focus::Files => {
                if self.browser_selected > 0 {
                    self.browser_selected -= 1;
                    self.sync_browser_state();
                }
            }
            Focus::Scaling => {
                if self.selected_scaling_mode > 0 {
                    self.selected_scaling_mode -= 1;
                }
            }
            Focus::Playlist => {
                if self.playlist_selected > 0 {
                    self.apply_playlist_draft_if_dirty();
                    self.playlist_selected -= 1;
                }
            }
        }
    }

    fn move_next(&mut self) {
        match self.focus {
            Focus::Files => {
                if self.browser_selected + 1 < self.browser_filtered_indices.len() {
                    self.browser_selected += 1;
                    self.sync_browser_state();
                }
            }
            Focus::Scaling => {
                if self.selected_scaling_mode + 1 < self.scaling_modes.len() {
                    self.selected_scaling_mode += 1;
                }
            }
            Focus::Playlist => {
                if self.playlist_selected + 1 < PLAYLIST_ITEM_COUNT {
                    self.apply_playlist_draft_if_dirty();
                    self.playlist_selected += 1;
                }
            }
        }
    }

    fn move_to_start(&mut self) {
        match self.focus {
            Focus::Files => {
                self.browser_selected = 0;
                self.sync_browser_state();
            }
            Focus::Scaling => {
                self.selected_scaling_mode = 0;
            }
            Focus::Playlist => {
                self.apply_playlist_draft_if_dirty();
                self.playlist_selected = 0;
            }
        }
    }

    fn move_to_end(&mut self) {
        match self.focus {
            Focus::Files => {
                self.browser_selected = self.browser_filtered_indices.len().saturating_sub(1);
                self.sync_browser_state();
            }
            Focus::Scaling => {
                self.selected_scaling_mode = self.scaling_modes.len().saturating_sub(1);
            }
            Focus::Playlist => {
                self.apply_playlist_draft_if_dirty();
                self.playlist_selected = PLAYLIST_ITEM_COUNT.saturating_sub(1);
            }
        }
    }

    fn page_up(&mut self) {
        const PAGE_STEP: usize = 8;
        match self.focus {
            Focus::Files => {
                self.browser_selected = self.browser_selected.saturating_sub(PAGE_STEP);
                self.sync_browser_state();
            }
            Focus::Scaling => {
                self.selected_scaling_mode = self.selected_scaling_mode.saturating_sub(PAGE_STEP);
            }
            Focus::Playlist => {
                self.apply_playlist_draft_if_dirty();
                self.playlist_selected = self.playlist_selected.saturating_sub(1);
            }
        }
    }

    fn page_down(&mut self) {
        const PAGE_STEP: usize = 8;
        match self.focus {
            Focus::Files => {
                let max_index = self.browser_filtered_indices.len().saturating_sub(1);
                self.browser_selected = (self.browser_selected + PAGE_STEP).min(max_index);
                self.sync_browser_state();
            }
            Focus::Scaling => {
                let max_index = self.scaling_modes.len().saturating_sub(1);
                self.selected_scaling_mode =
                    (self.selected_scaling_mode + PAGE_STEP).min(max_index);
            }
            Focus::Playlist => {
                let max_index = PLAYLIST_ITEM_COUNT.saturating_sub(1);
                self.apply_playlist_draft_if_dirty();
                self.playlist_selected = (self.playlist_selected + 1).min(max_index);
            }
        }
    }

    fn activate_selection(&mut self, backend: &mut Backend) {
        if self.focus == Focus::Files {
            if let Some(entry) = self.selected_browser_entry().cloned() {
                match entry.kind {
                    FileKind::Directory => {
                        if self.is_within_root(&entry.path) {
                            self.current_path = entry.path;
                            self.refresh_browser_entries();
                        } else {
                            self.push_log(format!(
                                "[WARN] Refusing to open directory outside Pictures: {}",
                                entry.path.display()
                            ));
                        }
                    }
                    FileKind::File => {
                        self.apply_wallpaper(backend, entry.path);
                    }
                }
            }
            return;
        }

        if self.focus == Focus::Playlist {
            self.activate_playlist_selection();
            return;
        }

        self.apply_wallpaper_from_selection(backend);
    }

    fn activate_playlist_selection(&mut self) {
        self.apply_playlist_draft_if_dirty();
        if self.playlist_selected + 1 < PLAYLIST_ITEM_COUNT {
            self.playlist_selected += 1;
        } else {
            self.playlist_selected = 0;
        }
    }

    fn handle_playlist_left_action(&mut self) {
        match self.playlist_selected {
            0 => self.set_selected_playlist_running_draft(false),
            1 => self.set_selected_playlist_source_draft(PlaylistSource::Workspace),
            _ => self.adjust_selected_playlist_interval_draft(-1),
        }
    }

    fn handle_playlist_right_action(&mut self) {
        match self.playlist_selected {
            0 => self.set_selected_playlist_running_draft(true),
            1 => self.set_selected_playlist_source_draft(PlaylistSource::Favorites),
            _ => self.adjust_selected_playlist_interval_draft(1),
        }
    }

    fn handle_files_right_action(&mut self) {
        if let Some(entry) = self.selected_browser_entry().cloned() {
            match entry.kind {
                FileKind::Directory => {
                    if self.is_within_root(&entry.path) {
                        self.current_path = entry.path;
                        self.push_log(format!("[INFO] Opened {}", self.current_path.display()));
                        self.refresh_browser_entries();
                    } else {
                        self.push_log(format!(
                            "[WARN] Refusing to open directory outside Pictures: {}",
                            entry.path.display()
                        ));
                    }
                }
                FileKind::File => {
                    if !entry.supported {
                        self.push_log(format!(
                            "[WARN] Unsupported file cannot be favorited: {}",
                            entry.path.display()
                        ));
                        return;
                    }

                    let changed = if self.favorites.insert(entry.path.clone()) {
                        self.push_log(format!("[INFO] Favorited {}", entry.path.display()));
                        true
                    } else {
                        self.favorites.remove(&entry.path);
                        self.push_log(format!("[INFO] Removed favorite {}", entry.path.display()));
                        true
                    };

                    if changed {
                        self.refresh_browser_entries();

                        if let Err(error) = self.save_favorites_state() {
                            self.push_log(format!("[WARN] Failed to save favorites: {error}"));
                        }
                    }
                }
            }
        }
    }

    fn apply_wallpaper_from_selection(&mut self, backend: &mut Backend) {
        if let Some(entry) = self.selected_browser_entry().cloned()
            && entry.kind == FileKind::File
        {
            self.apply_wallpaper(backend, entry.path);
        }
    }

    fn apply_wallpaper(&mut self, backend: &mut Backend, wallpaper: PathBuf) {
        if self.monitors.is_empty() {
            self.push_log("[WARN] No monitors available".to_string());
            return;
        }

        let mode = self.current_scaling_mode();
        let monitor_name = self.monitors[self.selected_monitor].name.clone();
        self.push_action_log(
            "ACTION",
            format!(
                "Apply requested: {} -> {} ({})",
                wallpaper.display(),
                monitor_name,
                mode
            ),
        );

        match backend.apply_wallpaper(&wallpaper, &monitor_name, mode) {
            Ok(()) => {
                self.applied_scaling_mode = self.selected_scaling_mode;
                self.stop_playlist_for_monitor(&monitor_name);
                self.push_action_log(
                    "SUCCESS",
                    format!(
                        "Daemon accepted: {} on {} ({})",
                        wallpaper.display(),
                        monitor_name,
                        mode
                    ),
                );
                self.sync_from_backend(backend);
            }
            Err(error) => {
                self.push_log(format!("[ERROR] Failed to apply wallpaper: {error}"));
            }
        }
    }

    fn stop_playlist_for_monitor(&mut self, monitor_name: &str) {
        let mut stopped = false;

        if let Some(config) = self.playlist_by_monitor.get_mut(monitor_name)
            && config.running
        {
            config.running = false;
            config.next_shuffle_at = None;
            stopped = true;
        }

        if !stopped {
            return;
        }

        self.push_log(format!(
            "[INFO] Playlist stopped for {} due to manual wallpaper selection",
            monitor_name
        ));

        if let Err(error) = self.save_playlist_state() {
            self.push_log(format!("[WARN] Failed to save playlist state: {error}"));
        }
    }

    fn clear_selected_monitor(&mut self, backend: &mut Backend) {
        if self.monitors.is_empty() {
            self.push_log("[WARN] No monitors available".to_string());
            return;
        }

        let monitor_name = self.monitors[self.selected_monitor].name.clone();
        match backend.clear_wallpaper(&monitor_name) {
            Ok(()) => {
                self.push_log(format!("[INFO] Cleared {}", monitor_name));
                self.sync_from_backend(backend);
            }
            Err(error) => {
                self.push_log(format!("[ERROR] Failed to clear wallpaper: {error}"));
            }
        }
    }

    fn toggle_pause(&mut self, backend: &mut Backend) {
        match backend.toggle_pause() {
            Ok(()) => {
                self.push_log("[INFO] Daemon pause toggled".to_string());
                self.sync_from_backend(backend);
            }
            Err(error) => {
                self.push_log(format!("[ERROR] Failed to toggle pause: {error}"));
            }
        }
    }

    fn start_or_refresh_daemon(&mut self, backend: &mut Backend) {
        match backend.status() {
            DaemonStatus::Running => {
                self.daemon_status = DaemonStatus::Running;
                self.sync_from_backend(backend);
                self.push_log("[INFO] Daemon refreshed".to_string());
            }
            DaemonStatus::Stopped | DaemonStatus::Crashed => match backend.start_daemon() {
                Ok(status) => {
                    self.daemon_status = status;
                    self.sync_from_backend(backend);
                    self.push_log(format!("[INFO] Daemon {}", self.daemon_status));
                }
                Err(error) => {
                    self.daemon_status = DaemonStatus::Crashed;
                    self.push_log(format!("[ERROR] Failed to start daemon: {error}"));
                }
            },
        }
    }

    fn toggle_favorite_current(&mut self) {
        if let Some(entry) = self.selected_browser_entry().cloned() {
            if !self.favorites.insert(entry.path.clone()) {
                self.favorites.remove(&entry.path);
                self.push_log(format!("[INFO] Removed favorite {}", entry.path.display()));
            } else {
                self.push_log(format!("[INFO] Favorited {}", entry.path.display()));
            }

            self.refresh_browser_entries();

            if let Err(error) = self.save_favorites_state() {
                self.push_log(format!("[WARN] Failed to save favorites: {error}"));
            }
        }
    }

    fn go_to_parent_directory(&mut self) {
        if let Some(parent) = self.current_path.parent()
            && self.is_within_root(parent)
        {
            self.current_path = parent.to_path_buf();
            self.push_log(format!("[INFO] Opened {}", self.current_path.display()));
            self.refresh_browser_entries();
        } else {
            self.push_log("[INFO] Already at Pictures root".to_string());
        }
    }

    pub fn refresh_browser_entries(&mut self) {
        let previous_selection = self
            .selected_browser_entry()
            .map(|entry| entry.path.clone());

        self.browser_entries = match fs::read_dir(&self.current_path) {
            Ok(read_dir) => {
                let mut entries = read_dir
                    .filter_map(|entry| entry.ok())
                    .filter_map(|entry| self.build_file_entry(entry.path()))
                    .collect::<Vec<_>>();

                entries.sort_by(|left, right| {
                    let left_rank = match left.kind {
                        FileKind::Directory => 0,
                        FileKind::File => 1,
                    };
                    let right_rank = match right.kind {
                        FileKind::Directory => 0,
                        FileKind::File => 1,
                    };

                    left_rank
                        .cmp(&right_rank)
                        .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
                });

                entries
            }
            Err(error) => {
                self.push_log(format!(
                    "[ERROR] Cannot read {}: {}",
                    self.current_path.display(),
                    error
                ));
                Vec::new()
            }
        };

        self.apply_browser_filters();

        if let Some(previous_path) = previous_selection {
            if let Some(index) = self
                .browser_filtered_indices
                .iter()
                .position(|browser_index| {
                    self.browser_entries[*browser_index].path == previous_path
                })
            {
                self.browser_selected = index;
            } else {
                self.browser_selected = 0;
            }
        } else {
            self.browser_selected = 0;
        }

        self.sync_browser_state();
    }

    fn apply_browser_filters(&mut self) {
        let query = self.search_buffer.trim();

        self.browser_filtered_indices = self
            .browser_entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                if self.favorites_only && !self.favorites.contains(&entry.path) {
                    return false;
                }

                if self.hide_unsupported && !entry.supports_filters() {
                    return false;
                }

                if query.is_empty() {
                    return true;
                }

                self.matcher.fuzzy_match(&entry.name, query).is_some()
            })
            .map(|(index, _)| index)
            .collect();

        if self.browser_selected >= self.browser_filtered_indices.len() {
            self.browser_selected = self.browser_filtered_indices.len().saturating_sub(1);
        }
    }

    fn sync_browser_state(&mut self) {
        if self.browser_filtered_indices.is_empty() {
            self.browser_selected = 0;
        } else if self.browser_selected >= self.browser_filtered_indices.len() {
            self.browser_selected = self.browser_filtered_indices.len() - 1;
        }
    }

    fn build_file_entry(&self, path: PathBuf) -> Option<FileEntry> {
        if !self.is_within_root(&path) {
            return None;
        }

        let metadata = fs::metadata(&path).ok()?;
        let kind = if metadata.is_dir() {
            FileKind::Directory
        } else {
            FileKind::File
        };
        let supported = kind == FileKind::Directory || is_supported_media(&path);
        let favorite = self.favorites.contains(&path);

        Some(FileEntry::new(path, kind, supported, favorite))
    }

    pub fn selected_browser_entry(&self) -> Option<&FileEntry> {
        self.browser_filtered_indices
            .get(self.browser_selected)
            .and_then(|browser_index| self.browser_entries.get(*browser_index))
    }

    pub fn current_scaling_mode(&self) -> ScalingMode {
        self.scaling_modes
            .get(self.selected_scaling_mode)
            .copied()
            .unwrap_or(ScalingMode::Fill)
    }

    pub fn applied_scaling_mode(&self) -> ScalingMode {
        self.scaling_modes
            .get(self.applied_scaling_mode)
            .copied()
            .unwrap_or(ScalingMode::Fill)
    }

    pub fn selected_monitor_ref(&self) -> Option<&Monitor> {
        self.monitors.get(self.selected_monitor)
    }

    pub fn preview_image(&self) -> Option<&PreviewImage> {
        self.preview_image.as_ref()
    }

    pub fn handle_tick(&mut self, backend: &mut Backend) {
        self.poll_preview_results();

        self.backend_sync_tick = self.backend_sync_tick.saturating_add(1);
        if self.backend_sync_tick >= BACKEND_SYNC_INTERVAL_TICKS {
            self.backend_sync_tick = 0;
            self.sync_from_backend(backend);
        }

        self.run_playlists_tick(backend);
    }

    pub fn update_preview_request(&mut self, target_width: u16, target_height_rows: u16) {
        self.poll_preview_results();

        if target_width < 1 || target_height_rows < 1 {
            self.preview_status = "Preview area too small".to_string();
            self.preview_image = None;
            self.preview_last_key = None;
            return;
        }

        let Some((entry_path, entry_name, entry_kind, entry_supported)) =
            self.selected_browser_entry().map(|entry| {
                (
                    entry.path.clone(),
                    entry.name.clone(),
                    entry.kind,
                    entry.supported,
                )
            })
        else {
            self.preview_status = "Select an image file to preview".to_string();
            self.preview_image = None;
            self.preview_last_key = None;
            return;
        };

        if entry_kind != FileKind::File {
            self.preview_status = "Directories cannot be previewed".to_string();
            self.preview_image = None;
            self.preview_last_key = None;
            return;
        }

        if !entry_supported {
            self.preview_status = "Unsupported format for preview".to_string();
            self.preview_image = None;
            self.preview_last_key = None;
            return;
        }

        let (monitor_name, monitor_width, monitor_height) = self
            .selected_monitor_ref()
            .map(|monitor| (monitor.name.clone(), monitor.width, monitor.height))
            .unwrap_or_else(|| (String::new(), 0, 0));

        let key = PreviewRequestKey {
            path: entry_path,
            scaling: self.current_scaling_mode(),
            width: target_width,
            height: target_height_rows,
            monitor_name,
            monitor_width,
            monitor_height,
        };

        if self.preview_last_key.as_ref() == Some(&key) {
            return;
        }

        self.preview_request_seq = self.preview_request_seq.saturating_add(1);
        self.preview_last_key = Some(key.clone());
        self.preview_status = format!("Loading {}", entry_name);

        let request = PreviewRequest {
            seq: self.preview_request_seq,
            path: key.path,
            scaling: key.scaling,
            target_width: key.width,
            target_height_rows: key.height,
            monitor_name: key.monitor_name,
            monitor_width: key.monitor_width,
            monitor_height: key.monitor_height,
        };

        if let Err(error) = self.preview_request_tx.send(request) {
            self.preview_status = format!("Preview worker unavailable: {error}");
        }
    }

    pub fn poll_preview_results(&mut self) {
        while let Ok(result) = self.preview_result_rx.try_recv() {
            if result.seq != self.preview_request_seq {
                continue;
            }

            match result.image {
                Ok(image) => {
                    self.preview_status = "Preview ready".to_string();
                    self.preview_image = Some(image);
                }
                Err(error) => {
                    self.preview_status = format!("Preview error: {error}");
                    self.preview_image = None;
                }
            }
        }
    }

    pub fn selected_monitor_label(&self) -> String {
        self.selected_monitor_ref()
            .map(|monitor| monitor.name.clone())
            .unwrap_or_else(|| "No monitors".to_string())
    }

    pub fn daemon_resource_label(&self) -> String {
        self.daemon_resources
            .map(|resources| {
                let used_mib = resources.memory_kib as f64 / 1024.0;
                let total_mib = resources.total_memory_kib as f64 / 1024.0;
                let percent = if resources.total_memory_kib > 0 {
                    resources.memory_kib as f64 * 100.0 / resources.total_memory_kib as f64
                } else {
                    0.0
                };

                format!(
                    "PID {} • RAM {:.1}/{:.0} MiB ({:.1}%)",
                    resources.pid, used_mib, total_mib, percent
                )
            })
            .unwrap_or_else(|| "PID -- • RAM --/-- MiB".to_string())
    }

    pub fn visible_browser_items(&self) -> impl Iterator<Item = (usize, &FileEntry)> {
        self.browser_filtered_indices.iter().enumerate().map(
            move |(visible_index, browser_index)| {
                (visible_index, &self.browser_entries[*browser_index])
            },
        )
    }

    pub fn sync_from_backend(&mut self, backend: &mut Backend) {
        self.daemon_status = backend.status();
        self.daemon_resources = backend.resource_snapshot();

        if let Ok(monitors) = backend.refresh_monitors() {
            let selected_name = self
                .selected_monitor_ref()
                .map(|monitor| monitor.name.clone());
            self.monitors = monitors;

            if let Some(selected_name) = selected_name {
                if let Some(index) = self
                    .monitors
                    .iter()
                    .position(|monitor| monitor.name == selected_name)
                {
                    self.selected_monitor = index;
                } else if self.selected_monitor >= self.monitors.len() {
                    self.selected_monitor = 0;
                }
            } else if self.selected_monitor >= self.monitors.len() {
                self.selected_monitor = 0;
            }

            self.ensure_playlist_states();
        }
    }

    pub fn selected_playlist_running(&self) -> bool {
        if let Some(draft) = self.selected_playlist_draft() {
            return draft.running;
        }

        self.selected_playlist_config()
            .map(|config| config.running)
            .unwrap_or(false)
    }

    pub fn selected_playlist_source(&self) -> PlaylistSource {
        if let Some(draft) = self.selected_playlist_draft() {
            return draft.source;
        }

        self.selected_playlist_config()
            .map(|config| config.source)
            .unwrap_or(PlaylistSource::Workspace)
    }

    pub fn selected_playlist_interval_secs(&self) -> u64 {
        if let Some(draft) = self.selected_playlist_draft() {
            return draft.interval_secs;
        }

        self.selected_playlist_config()
            .map(|config| config.interval_secs)
            .unwrap_or(PLAYLIST_INTERVAL_DEFAULT_SECS)
    }

    pub fn selected_playlist_pool_size(&self) -> usize {
        self.playlist_candidates(self.selected_playlist_source())
            .len()
    }

    pub fn selected_playlist_next_eta_secs(&self) -> Option<u64> {
        let now = Instant::now();
        self.selected_playlist_config()
            .and_then(|config| config.next_shuffle_at)
            .and_then(|next| next.checked_duration_since(now))
            .map(|dur| dur.as_secs())
    }

    pub fn has_running_playlists(&self) -> bool {
        self.playlist_by_monitor
            .values()
            .any(|config| config.running)
    }

    fn push_log(&mut self, entry: String) {
        if self.logs.len() >= LOG_CAPACITY {
            self.logs.remove(0);
        }

        self.logs.push(entry);
    }

    fn ensure_playlist_states(&mut self) {
        self.apply_playlist_draft_if_dirty();

        self.playlist_by_monitor
            .retain(|name, _| self.monitors.iter().any(|monitor| monitor.name == *name));

        for monitor in &self.monitors {
            self.playlist_by_monitor
                .entry(monitor.name.clone())
                .or_default();
        }

        if let Err(error) = self.save_playlist_state() {
            self.push_log(format!("[WARN] Failed to save playlist state: {error}"));
        }

        self.ensure_playlist_draft_loaded();
    }

    fn selected_monitor_name(&self) -> Option<&str> {
        self.selected_monitor_ref()
            .map(|monitor| monitor.name.as_str())
    }

    fn selected_playlist_config(&self) -> Option<&PlaylistConfig> {
        let monitor_name = self.selected_monitor_name()?;
        self.playlist_by_monitor.get(monitor_name)
    }

    fn selected_playlist_config_mut(&mut self) -> Option<&mut PlaylistConfig> {
        let monitor_name = self.selected_monitor_name()?.to_string();
        self.playlist_by_monitor.get_mut(&monitor_name)
    }

    fn ensure_playlist_draft_loaded(&mut self) {
        let Some(monitor_name) = self.selected_monitor_name().map(ToString::to_string) else {
            self.playlist_draft = None;
            self.playlist_draft_monitor = None;
            self.playlist_draft_dirty = false;
            return;
        };

        if self.playlist_draft_monitor.as_deref() == Some(monitor_name.as_str()) {
            return;
        }

        let draft = self
            .playlist_by_monitor
            .get(&monitor_name)
            .map(PlaylistDraft::from)
            .unwrap_or_else(|| PlaylistDraft::from(&PlaylistConfig::default()));

        self.playlist_draft = Some(draft);
        self.playlist_draft_monitor = Some(monitor_name);
        self.playlist_draft_dirty = false;
    }

    fn selected_playlist_draft(&self) -> Option<PlaylistDraft> {
        let monitor_name = self.selected_monitor_name()?;
        if self.playlist_draft_monitor.as_deref() == Some(monitor_name) {
            self.playlist_draft
        } else {
            None
        }
    }

    fn with_selected_playlist_draft_mut<F>(&mut self, mutator: F)
    where
        F: FnOnce(&mut PlaylistDraft) -> bool,
    {
        self.ensure_playlist_draft_loaded();

        let Some(draft) = self.playlist_draft.as_mut() else {
            self.push_log("[WARN] No monitor selected for playlist".to_string());
            return;
        };

        if mutator(draft) {
            self.playlist_draft_dirty = true;
        }
    }

    fn apply_playlist_draft_if_dirty(&mut self) {
        if !self.playlist_draft_dirty {
            return;
        }

        let Some(monitor_name) = self.playlist_draft_monitor.clone() else {
            self.playlist_draft_dirty = false;
            return;
        };

        let Some(draft) = self.playlist_draft else {
            self.playlist_draft_dirty = false;
            return;
        };

        let mut running_log = None;
        let mut source_log = None;
        let mut interval_log = None;

        {
            let Some(config) = self.playlist_by_monitor.get_mut(&monitor_name) else {
                self.playlist_draft_dirty = false;
                return;
            };

            let next_running = draft.running;
            let next_source = draft.source;
            let next_interval = draft
                .interval_secs
                .clamp(PLAYLIST_INTERVAL_MIN_SECS, PLAYLIST_INTERVAL_MAX_SECS);

            if config.running != next_running {
                config.running = next_running;
                config.next_shuffle_at = if next_running {
                    Some(Instant::now())
                } else {
                    None
                };
                running_log = Some(format!(
                    "[INFO] Playlist {} for {}",
                    if next_running { "started" } else { "stopped" },
                    monitor_name
                ));
            }

            if config.source != next_source {
                config.source = next_source;
                config.last_wallpaper = None;
                source_log = Some(format!("[INFO] Playlist source: {}", next_source.label()));
            }

            if config.interval_secs != next_interval {
                config.interval_secs = next_interval;
                interval_log = Some(format!(
                    "[INFO] Playlist interval set to {}s",
                    next_interval
                ));
            }
        }

        if let Some(entry) = running_log {
            self.push_log(entry);
        }
        if let Some(entry) = source_log {
            self.push_log(entry);
        }
        if let Some(entry) = interval_log {
            self.push_log(entry);
        }

        if let Err(error) = self.save_playlist_state() {
            self.push_log(format!("[WARN] Failed to save playlist state: {error}"));
        }

        self.playlist_draft_dirty = false;
    }

    fn set_selected_playlist_running_draft(&mut self, running: bool) {
        self.with_selected_playlist_draft_mut(|draft| {
            if draft.running == running {
                return false;
            }
            draft.running = running;
            true
        });
    }

    fn toggle_selected_playlist_running_draft(&mut self) {
        self.with_selected_playlist_draft_mut(|draft| {
            draft.running = !draft.running;
            true
        });
    }

    fn set_selected_playlist_source_draft(&mut self, source: PlaylistSource) {
        self.with_selected_playlist_draft_mut(|draft| {
            if draft.source == source {
                return false;
            }
            draft.source = source;
            true
        });
    }

    fn toggle_selected_playlist_source_draft(&mut self) {
        self.with_selected_playlist_draft_mut(|draft| {
            draft.source = draft.source.toggle();
            true
        });
    }

    fn adjust_selected_playlist_interval_draft(&mut self, direction: i8) {
        self.with_selected_playlist_draft_mut(|draft| {
            let current = draft
                .interval_secs
                .clamp(PLAYLIST_INTERVAL_MIN_SECS, PLAYLIST_INTERVAL_MAX_SECS);
            let next = match direction.cmp(&0) {
                std::cmp::Ordering::Greater => next_playlist_interval_step(current),
                std::cmp::Ordering::Less => previous_playlist_interval_step(current),
                std::cmp::Ordering::Equal => current,
            };

            if next == draft.interval_secs {
                return false;
            }

            draft.interval_secs = next;
            true
        });
    }

    fn trigger_selected_playlist_now(&mut self) {
        let Some(config) = self.selected_playlist_config_mut() else {
            self.push_log("[WARN] No monitor selected for playlist".to_string());
            return;
        };

        config.next_shuffle_at = Some(Instant::now());
        self.push_log("[INFO] Playlist shuffled now".to_string());
    }

    fn run_playlists_tick(&mut self, backend: &mut Backend) {
        let now = Instant::now();
        let monitor_names = self
            .monitors
            .iter()
            .map(|monitor| monitor.name.clone())
            .collect::<Vec<_>>();

        for monitor_name in monitor_names {
            let Some(config) = self.playlist_by_monitor.get(&monitor_name) else {
                continue;
            };

            if !config.running {
                continue;
            }

            let due = config
                .next_shuffle_at
                .map(|next| next <= now)
                .unwrap_or(true);
            if !due {
                continue;
            }

            let source = config.source;
            let interval_secs = config.interval_secs;
            let last_wallpaper = config.last_wallpaper.clone();

            let candidates = self.playlist_candidates(source);
            if candidates.is_empty() {
                self.push_log(format!(
                    "[WARN] Playlist {} has no {} images",
                    monitor_name,
                    source.label().to_ascii_lowercase()
                ));
                if let Some(config) = self.playlist_by_monitor.get_mut(&monitor_name) {
                    config.next_shuffle_at = Some(now + Duration::from_secs(interval_secs));
                }
                continue;
            }

            let mut selected_index = fastrand::usize(..candidates.len());
            if candidates.len() > 1
                && last_wallpaper
                    .as_ref()
                    .is_some_and(|last| last == &candidates[selected_index])
            {
                selected_index = (selected_index + 1) % candidates.len();
            }

            let selected = candidates[selected_index].clone();
            let mode = self.current_scaling_mode();

            match backend.apply_wallpaper(&selected, &monitor_name, mode) {
                Ok(()) => {
                    if let Some(monitor) = self.monitors.iter_mut().find(|m| m.name == monitor_name)
                    {
                        monitor.wallpaper = Some(selected.clone());
                    }

                    self.push_action_log(
                        "PLAYLIST",
                        format!(
                            "{} -> {} ({}, every {}s)",
                            selected.display(),
                            monitor_name,
                            source.label(),
                            interval_secs
                        ),
                    );

                    if let Some(config) = self.playlist_by_monitor.get_mut(&monitor_name) {
                        config.last_wallpaper = Some(selected);
                        config.next_shuffle_at = Some(now + Duration::from_secs(interval_secs));
                    }
                }
                Err(error) => {
                    self.push_log(format!(
                        "[ERROR] Playlist apply failed on {}: {}",
                        monitor_name, error
                    ));
                    if let Some(config) = self.playlist_by_monitor.get_mut(&monitor_name) {
                        config.next_shuffle_at = Some(now + Duration::from_secs(5));
                    }
                }
            }
        }
    }

    fn playlist_candidates(&self, source: PlaylistSource) -> Vec<PathBuf> {
        match source {
            PlaylistSource::Workspace => self
                .visible_browser_items()
                .filter(|(_, entry)| entry.kind == FileKind::File && entry.supported)
                .map(|(_, entry)| entry.path.clone())
                .collect(),
            PlaylistSource::Favorites => self
                .favorites
                .iter()
                .filter(|path| path.is_file() && is_supported_media(path.as_path()))
                .cloned()
                .collect(),
        }
    }

    fn load_playlist_state(&mut self) {
        let path = playlist_state_file_path();
        let data = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                self.push_log(format!("[WARN] Failed to read playlist state: {error}"));
                return;
            }
        };

        for line in data.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let mut fields = line.splitn(4, '\t');
            let Some(monitor_name) = fields.next() else {
                continue;
            };
            let Some(source_field) = fields.next() else {
                continue;
            };
            let Some(interval_field) = fields.next() else {
                continue;
            };
            let Some(running_field) = fields.next() else {
                continue;
            };

            let source = match source_field {
                "workspace" => PlaylistSource::Workspace,
                "favorites" => PlaylistSource::Favorites,
                _ => continue,
            };

            let interval_secs = match interval_field.parse::<u64>() {
                Ok(value) => value.clamp(PLAYLIST_INTERVAL_MIN_SECS, PLAYLIST_INTERVAL_MAX_SECS),
                Err(_) => continue,
            };

            let running = match running_field {
                "1" => true,
                "0" => false,
                _ => continue,
            };

            self.playlist_by_monitor.insert(
                monitor_name.to_string(),
                PlaylistConfig {
                    source,
                    interval_secs,
                    running,
                    next_shuffle_at: if running { Some(Instant::now()) } else { None },
                    last_wallpaper: None,
                },
            );
        }

        if !self.playlist_by_monitor.is_empty() {
            self.push_log("[INFO] Loaded playlist settings".to_string());
        }
    }

    fn load_favorites_state(&mut self) {
        let path = favorites_state_file_path();
        let data = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                self.push_log(format!("[WARN] Failed to read favorites: {error}"));
                return;
            }
        };

        let mut loaded = 0usize;
        for line in data.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let path = PathBuf::from(line);
            if self.is_within_root(&path) {
                self.favorites.insert(path);
                loaded += 1;
            }
        }

        if loaded > 0 {
            self.push_log(format!("[INFO] Loaded {} favorite(s)", loaded));
        }
    }

    fn save_favorites_state(&self) -> std::io::Result<()> {
        let path = favorites_state_file_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut values = self
            .favorites
            .iter()
            .map(|path| {
                path.canonicalize()
                    .unwrap_or_else(|_| path.to_path_buf())
                    .display()
                    .to_string()
            })
            .collect::<Vec<_>>();
        values.sort();

        let mut body = String::from("# absolute favorite paths, one per line\n");
        if !values.is_empty() {
            body.push_str(&values.join("\n"));
            body.push('\n');
        }

        fs::write(path, body)
    }

    fn save_playlist_state(&self) -> std::io::Result<()> {
        let path = playlist_state_file_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut lines = Vec::with_capacity(self.playlist_by_monitor.len() + 1);
        lines.push("# monitor\tsource\tinterval_secs\trunning".to_string());

        let mut entries = self.playlist_by_monitor.iter().collect::<Vec<_>>();
        entries.sort_by(|(left, _), (right, _)| left.cmp(right));

        for (monitor_name, config) in entries {
            let source = match config.source {
                PlaylistSource::Workspace => "workspace",
                PlaylistSource::Favorites => "favorites",
            };

            let running = if config.running { "1" } else { "0" };

            lines.push(format!(
                "{}\t{}\t{}\t{}",
                monitor_name,
                source,
                config
                    .interval_secs
                    .clamp(PLAYLIST_INTERVAL_MIN_SECS, PLAYLIST_INTERVAL_MAX_SECS),
                running
            ));
        }

        fs::write(path, lines.join("\n"))
    }

    fn push_action_log(&mut self, tag: &str, message: String) {
        self.push_log(format!(
            "[{}] [{}] {}",
            Self::log_timestamp_seconds(),
            tag,
            message
        ));
    }

    fn log_timestamp_seconds() -> u64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => duration.as_secs(),
            Err(_) => 0,
        }
    }

    fn is_within_root(&self, path: &Path) -> bool {
        let root = self
            .pictures_root
            .canonicalize()
            .unwrap_or_else(|_| self.pictures_root.clone());
        match path.canonicalize() {
            Ok(candidate) => candidate.starts_with(&root),
            Err(_) => false,
        }
    }

    fn next_scaling_mode(&mut self) {
        if self.selected_scaling_mode + 1 < self.scaling_modes.len() {
            self.selected_scaling_mode += 1;
        }
    }

    fn previous_scaling_mode(&mut self) {
        if self.selected_scaling_mode > 0 {
            self.selected_scaling_mode -= 1;
        }
    }

    fn select_monitor_hotkey(&mut self, key: char) -> bool {
        self.apply_playlist_draft_if_dirty();

        let Some(digit) = key.to_digit(10) else {
            return false;
        };

        let Some(target_index) = digit.checked_sub(1).map(|value| value as usize) else {
            return false;
        };

        if target_index < self.monitors.len() {
            self.selected_monitor = target_index;
            self.push_log(format!(
                "[INFO] Selected monitor {}",
                self.monitors[target_index].name
            ));
            self.ensure_playlist_draft_loaded();
            return true;
        }

        false
    }
}

fn next_playlist_interval_step(current: u64) -> u64 {
    if current < 59 {
        (current + 1).min(59)
    } else if current < 59 * 60 {
        (current + 60).min(59 * 60)
    } else {
        (current + 3600).min(PLAYLIST_INTERVAL_MAX_SECS)
    }
}

fn previous_playlist_interval_step(current: u64) -> u64 {
    if current > 3600 {
        current.saturating_sub(3600).max(59 * 60)
    } else if current > 60 {
        current.saturating_sub(60).max(60)
    } else if current > PLAYLIST_INTERVAL_MIN_SECS {
        current.saturating_sub(1).max(PLAYLIST_INTERVAL_MIN_SECS)
    } else {
        PLAYLIST_INTERVAL_MIN_SECS
    }
}

fn is_supported_media(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png"
                    | "jpg"
                    | "jpeg"
                    | "gif"
                    | "bmp"
                    | "webp"
                    | "tif"
                    | "tiff"
                    | "svg"
                    | "avif"
                    | "heic"
                    | "heif"
            )
        })
        .unwrap_or(false)
}

fn pictures_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Pictures")
}

fn playlist_state_file_path() -> PathBuf {
    state_file_path(PLAYLIST_STATE_FILENAME)
}

fn favorites_state_file_path() -> PathBuf {
    state_file_path(FAVORITES_STATE_FILENAME)
}

fn state_file_path(filename: &str) -> PathBuf {
    if let Some(path) = env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(path).join("vellum").join(filename);
    }

    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("vellum")
            .join(filename);
    }

    PathBuf::from(filename)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_file_detection_is_case_insensitive() {
        assert!(is_supported_media(Path::new("wallpaper.PNG")));
        assert!(is_supported_media(Path::new("animation.gif")));
        assert!(!is_supported_media(Path::new("notes.txt")));
    }
}
