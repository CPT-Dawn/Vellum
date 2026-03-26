use std::collections::HashSet;
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use common::ipc::BgInfo;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use fuzzy_matcher::{FuzzyMatcher, skim::SkimMatcherV2};

use crate::backend::Backend;

const LOG_CAPACITY: usize = 128;

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
    Monitors,
    Scaling,
}

impl Focus {
    fn next(self) -> Self {
        match self {
            Self::Files => Self::Monitors,
            Self::Monitors => Self::Scaling,
            Self::Scaling => Self::Files,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Files => Self::Scaling,
            Self::Monitors => Self::Files,
            Self::Scaling => Self::Monitors,
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
    pub scale: f32,
    pub wallpaper: Option<PathBuf>,
}

impl Monitor {
    fn new(name: &str, width: u16, height: u16) -> Self {
        Self {
            name: name.to_string(),
            width,
            height,
            scale: 1.0,
            wallpaper: None,
        }
    }

    pub fn aspect_ratio(&self) -> f64 {
        self.width as f64 / self.height.max(1) as f64
    }
}

impl From<BgInfo> for Monitor {
    fn from(value: BgInfo) -> Self {
        let scale = value.scale_factor.to_f32();
        let (width, height) = value.dim;
        Self {
            name: value.name.into(),
            width: width.min(u16::MAX as u32) as u16,
            height: height.min(u16::MAX as u32) as u16,
            scale,
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
    pub daemon_status: DaemonStatus,
    pub logs: Vec<String>,
    pub focus: Focus,
    matcher: SkimMatcherV2,
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
            .field("daemon_status", &self.daemon_status)
            .field("logs", &self.logs)
            .field("focus", &self.focus)
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
            daemon_status: DaemonStatus::Stopped,
            logs: vec!["[INFO] Vellum TUI ready".to_string()],
            focus: Focus::Files,
            matcher: SkimMatcherV2::default(),
        };

        app.refresh_browser_entries();
        app
    }

    pub fn load_or_default() -> Self {
        Self::new()
    }

    pub fn handle_event(&mut self, event: Event, backend: &mut Backend) -> bool {
        match event {
            Event::Key(key)
                if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat =>
            {
                self.handle_key_event(key, backend)
            }
            Event::Resize(_, _) => {
                self.push_log("[INFO] Terminal resized".to_string());
                false
            }
            _ => false,
        }
    }

    pub fn handle_key_event(&mut self, key: KeyEvent, backend: &mut Backend) -> bool {
        if self.search_active {
            return self.handle_search_key(key);
        }

        match key.code {
            KeyCode::Char('q') => true,
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
            KeyCode::Tab => {
                self.focus = self.focus.next();
                false
            }
            KeyCode::BackTab => {
                self.focus = self.focus.previous();
                false
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Left | KeyCode::Char('h') => {
                self.move_previous();
                false
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Right | KeyCode::Char('l') => {
                self.move_next();
                false
            }
            KeyCode::Backspace => {
                self.go_to_parent_directory();
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
            Focus::Monitors => {
                if self.selected_monitor > 0 {
                    self.selected_monitor -= 1;
                }
            }
            Focus::Scaling => {
                if self.selected_scaling_mode > 0 {
                    self.selected_scaling_mode -= 1;
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
            Focus::Monitors => {
                if self.selected_monitor + 1 < self.monitors.len() {
                    self.selected_monitor += 1;
                }
            }
            Focus::Scaling => {
                if self.selected_scaling_mode + 1 < self.scaling_modes.len() {
                    self.selected_scaling_mode += 1;
                }
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

        self.apply_wallpaper_from_selection(backend);
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
        match backend.apply_wallpaper(&wallpaper, &monitor_name, mode) {
            Ok(()) => {
                self.push_log(format!(
                    "[INFO] Applied {} to {} using {}",
                    wallpaper.display(),
                    monitor_name,
                    mode
                ));
                self.sync_from_backend(backend);
            }
            Err(error) => {
                self.push_log(format!("[ERROR] Failed to apply wallpaper: {error}"));
            }
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

    pub fn selected_monitor_ref(&self) -> Option<&Monitor> {
        self.monitors.get(self.selected_monitor)
    }

    pub fn selected_monitor_label(&self) -> String {
        self.selected_monitor_ref()
            .map(|monitor| monitor.name.clone())
            .unwrap_or_else(|| "No monitors".to_string())
    }

    pub fn selected_wallpaper_label(&self) -> String {
        self.selected_monitor_ref()
            .and_then(|monitor| monitor.wallpaper.as_ref())
            .map(|wallpaper| wallpaper.display().to_string())
            .unwrap_or_else(|| "(none)".to_string())
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
        }
    }

    fn push_log(&mut self, entry: String) {
        if self.logs.len() >= LOG_CAPACITY {
            self.logs.remove(0);
        }

        self.logs.push(entry);
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
