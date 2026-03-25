mod backend;
mod model;
mod render;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LeftPaneTab {
    LibraryExplorer,
    ActiveQueue,
}

use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    ffi::CString,
    io,
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use model::{
    BrowserEntry, ImagePreview, InputMode, MonitorEntry, Notification, NotificationLevel,
    PlaylistEntry, Rotation, ScaleMode, TransitionState,
};
use ratatui::{Terminal, backend::CrosstermBackend, widgets::ListState};
use ratatui_image::{picker::Picker, protocol::StatefulProtocol};
use tokio::{
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    time,
};
use vellum_core::{VellumServer, VellumServerConfig};

const TICK_RATE_MS: u64 = 33;
const DAEMON_PROBE_INTERVAL: Duration = Duration::from_secs(2);
const NOTIFICATION_TTL: Duration = Duration::from_secs(5);

pub async fn run_entrypoint() -> io::Result<()> {
    if let Some(namespace) = daemon_mode_namespace_from_args() {
        set_process_name("vellum-daemon");
        ensure_wayland_session()?;
        return run_daemon(namespace);
    }

    set_process_name("vellum");
    ensure_wayland_session()?;
    run_app().await
}

fn run_daemon(namespace: String) -> io::Result<()> {
    let config = VellumServerConfig {
        namespace,
        quiet: true,
        ..VellumServerConfig::default()
    };
    let server = VellumServer::new(config);
    server.run().map_err(io::Error::other)
}

fn daemon_mode_namespace_from_args() -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg != "--daemon" && arg != "--daemon-subprocess" {
            continue;
        }

        let mut namespace = String::new();
        while let Some(flag) = args.next() {
            if flag == "--namespace" {
                namespace = args.next().unwrap_or_default();
                break;
            }
        }

        if namespace.is_empty() {
            namespace = String::from("vellum");
        }

        return Some(namespace);
    }

    None
}

fn ensure_wayland_session() -> io::Result<()> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("WAYLAND_SOCKET").is_some()
    {
        return Ok(());
    }

    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "vellum requires a Wayland session (WAYLAND_DISPLAY or WAYLAND_SOCKET must be set)",
    ))
}

fn set_process_name(name: &str) {
    let Ok(name) = CString::new(name) else {
        return;
    };

    #[cfg(target_os = "linux")]
    unsafe {
        let _ = libc::prctl(libc::PR_SET_NAME, name.as_ptr() as usize, 0, 0, 0);
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

enum AppEvent {
    ApplyFinished(Result<String>),
}

pub(crate) struct App {
    pub(crate) should_quit: bool,
    pub(crate) focus: model::FocusRegion,
    pub(crate) left_tab: LeftPaneTab,
    pub(crate) input_mode: InputMode,
    pub(crate) help_open: bool,

    pub(crate) browser_dir: PathBuf,
    pub(crate) search_query: String,
    pub(crate) browser_entries: Vec<BrowserEntry>,
    pub(crate) browser_filtered: Vec<usize>,
    pub(crate) browser_selected: usize,
    pub(crate) browser_state: ListState,

    pub(crate) monitors: Vec<MonitorEntry>,
    pub(crate) monitor_selected: usize,
    active_monitor_name: Option<String>,
    pub(crate) monitor_state: ListState,
    pub(crate) selected_targets: BTreeSet<String>,
    targets_initialized: bool,

    pub(crate) selected_preview: Option<ImagePreview>,
    image_dim_cache: HashMap<PathBuf, Option<(u32, u32)>>,
    preview_picker: Option<Picker>,
    preview_state: Option<StatefulProtocol>,

    pub(crate) playlist: Vec<PlaylistEntry>,
    pub(crate) playlist_running: bool,
    pub(crate) playlist_interval: Duration,
    pub(crate) playlist_selected: usize,
    pub(crate) playlist_state: ListState,
    last_playlist_tick: Instant,

    pub(crate) scale_mode: ScaleMode,
    pub(crate) rotation: Rotation,
    pub(crate) transition: TransitionState,

    pub(crate) notifications: VecDeque<Notification>,
    pub(crate) status: String,
    pub(crate) activity_open: bool,

    pub(crate) daemon_namespace: String,
    backend_child: Option<tokio::process::Child>,
    pub(crate) daemon_online: bool,
    last_daemon_probe: Instant,
    refresh_monitors: bool,
    ipc_in_flight: bool,

    event_tx: UnboundedSender<AppEvent>,
}

impl App {
    fn new(event_tx: UnboundedSender<AppEvent>) -> Self {
        let mut browser_state = ListState::default();
        browser_state.select(Some(0));

        let mut monitor_state = ListState::default();
        monitor_state.select(Some(0));

        let playlist_state = ListState::default();

        Self {
            should_quit: false,
            focus: model::FocusRegion::Library,
            left_tab: LeftPaneTab::LibraryExplorer,
            input_mode: InputMode::Normal,
            help_open: false,
            browser_dir: model::preferred_initial_browser_dir(),
            search_query: String::new(),
            browser_entries: Vec::new(),
            browser_filtered: Vec::new(),
            browser_selected: 0,
            browser_state,
            monitors: Vec::new(),
            monitor_selected: 0,
            active_monitor_name: None,
            monitor_state,
            selected_targets: BTreeSet::new(),
            targets_initialized: false,
            selected_preview: None,
            image_dim_cache: HashMap::new(),
            preview_picker: None,
            preview_state: None,
            playlist: Vec::new(),
            playlist_running: false,
            playlist_interval: Duration::from_secs(20),
            playlist_selected: 0,
            playlist_state,
            last_playlist_tick: Instant::now(),
            scale_mode: ScaleMode::Fit,
            rotation: Rotation::Deg0,
            transition: TransitionState::default(),
            notifications: VecDeque::new(),
            status: String::from("Welcome to Vellum"),
            activity_open: false,
            daemon_namespace: String::from("vellum"),
            backend_child: None,
            daemon_online: false,
            last_daemon_probe: Instant::now() - DAEMON_PROBE_INTERVAL,
            refresh_monitors: false,
            ipc_in_flight: false,
            event_tx,
        }
    }

    fn request_monitor_refresh(&mut self) {
        self.refresh_monitors = true;
    }

    fn init_preview_picker(&mut self) {
        self.preview_picker = Picker::from_query_stdio()
            .ok()
            .or_else(|| Some(Picker::halfblocks()));
    }

    fn toggle_activity_log(&mut self) {
        self.activity_open = !self.activity_open;
    }

    fn toggle_left_tab(&mut self) {
        self.left_tab = match self.left_tab {
            LeftPaneTab::LibraryExplorer => LeftPaneTab::ActiveQueue,
            LeftPaneTab::ActiveQueue => LeftPaneTab::LibraryExplorer,
        };

        self.focus = match self.left_tab {
            LeftPaneTab::LibraryExplorer => model::FocusRegion::Library,
            LeftPaneTab::ActiveQueue => model::FocusRegion::Playlist,
        };
    }

    fn notify(&mut self, level: NotificationLevel, text: impl Into<String>) {
        let text = text.into();
        self.status = text.clone();
        self.notifications.push_back(Notification {
            text,
            level,
            created_at: Instant::now(),
        });
        while self.notifications.len() > 12 {
            let _ = self.notifications.pop_front();
        }
    }

    fn prune_notifications(&mut self) {
        while self
            .notifications
            .front()
            .is_some_and(|note| note.created_at.elapsed() > NOTIFICATION_TTL)
        {
            let _ = self.notifications.pop_front();
        }
    }

    fn selected_browser_entry(&self) -> Option<&BrowserEntry> {
        let index = *self.browser_filtered.get(self.browser_selected)?;
        self.browser_entries.get(index)
    }

    fn selected_monitor(&self) -> Option<&MonitorEntry> {
        self.monitors.get(self.monitor_selected)
    }

    fn selected_playlist_entry(&self) -> Option<&PlaylistEntry> {
        self.playlist.get(self.playlist_selected)
    }

    fn sync_browser_state(&mut self) {
        if self.browser_filtered.is_empty() {
            self.browser_selected = 0;
            self.browser_state.select(None);
            return;
        }

        self.browser_selected = self
            .browser_selected
            .min(self.browser_filtered.len().saturating_sub(1));
        self.browser_state.select(Some(self.browser_selected));
    }

    fn sync_monitor_state(&mut self) {
        if self.monitors.is_empty() {
            self.monitor_selected = 0;
            self.active_monitor_name = None;
            self.monitor_state.select(None);
            return;
        }

        if let Some(name) = self.active_monitor_name.as_ref()
            && let Some(index) = self
                .monitors
                .iter()
                .position(|monitor| &monitor.name == name)
        {
            self.monitor_selected = index;
        } else if let Some(index) = self.monitors.iter().position(|monitor| monitor.focused) {
            self.monitor_selected = index;
            self.active_monitor_name = Some(self.monitors[index].name.clone());
        } else {
            self.monitor_selected = self
                .monitor_selected
                .min(self.monitors.len().saturating_sub(1));
            self.active_monitor_name = self
                .monitors
                .get(self.monitor_selected)
                .map(|monitor| monitor.name.clone());
        }

        self.monitor_state.select(Some(self.monitor_selected));
    }

    fn sync_playlist_state(&mut self) {
        if self.playlist.is_empty() {
            self.playlist_selected = 0;
            self.playlist_state.select(None);
            return;
        }

        self.playlist_selected = self
            .playlist_selected
            .min(self.playlist.len().saturating_sub(1));
        self.playlist_state.select(Some(self.playlist_selected));
    }

    fn move_browser(&mut self, delta: isize) {
        if self.browser_filtered.is_empty() {
            return;
        }

        let max = self.browser_filtered.len().saturating_sub(1) as isize;
        self.browser_selected = (self.browser_selected as isize + delta).clamp(0, max) as usize;
        self.sync_browser_state();
        self.refresh_preview();
    }

    fn move_playlist(&mut self, delta: isize) {
        if self.playlist.is_empty() {
            return;
        }

        let max = self.playlist.len().saturating_sub(1) as isize;
        self.playlist_selected = (self.playlist_selected as isize + delta).clamp(0, max) as usize;
        self.sync_playlist_state();
    }

    fn select_all_targets(&mut self) {
        self.selected_targets.clear();
        for monitor in &self.monitors {
            self.selected_targets.insert(monitor.name.clone());
        }
    }

    fn clear_targets(&mut self) {
        self.selected_targets.clear();
    }

    fn toggle_target_for_monitor(&mut self, index: usize) {
        let Some(name) = self.monitors.get(index).map(|monitor| monitor.name.clone()) else {
            return;
        };

        if !self.selected_targets.insert(name.clone()) {
            let _ = self.selected_targets.remove(&name);
        }

        self.monitor_selected = index;
        self.active_monitor_name = Some(name);
        self.sync_monitor_state();
    }

    fn selected_output_names(&self) -> Vec<String> {
        if !self.selected_targets.is_empty() {
            return self.selected_targets.iter().cloned().collect();
        }

        if let Some(name) = self.active_monitor_name.as_ref() {
            return vec![name.clone()];
        }

        self.selected_monitor()
            .map(|monitor| vec![monitor.name.clone()])
            .unwrap_or_default()
    }

    fn apply_filter(&mut self) {
        self.browser_filtered = model::fuzzy_filter(&self.browser_entries, &self.search_query);
        self.sync_browser_state();
        self.refresh_preview();
    }

    fn refresh_preview(&mut self) {
        let Some(entry) = self.selected_browser_entry() else {
            self.selected_preview = None;
            self.preview_state = None;
            return;
        };

        if entry.is_dir() {
            self.selected_preview = None;
            self.preview_state = None;
            return;
        }

        let image_path = entry.path.clone();
        let dims = if let Some(cached) = self.image_dim_cache.get(&image_path) {
            *cached
        } else {
            let probed = image::image_dimensions(&image_path).ok();
            self.image_dim_cache.insert(image_path.clone(), probed);
            probed
        };

        self.selected_preview = dims.map(|(width, height)| ImagePreview {
            path: image_path.clone(),
            width,
            height,
        });

        self.preview_state = None;

        let Some(picker) = self.preview_picker.as_ref() else {
            return;
        };

        let Ok(reader) = image::ImageReader::open(&image_path) else {
            return;
        };

        let Ok(image) = reader.decode() else {
            return;
        };

        self.preview_state = Some(picker.new_resize_protocol(image));
    }

    fn toggle_playlist_for_selected(&mut self) {
        let Some(entry) = self.selected_browser_entry().cloned() else {
            self.notify(NotificationLevel::Warn, "No selected image");
            return;
        };

        if entry.is_dir() {
            self.notify(NotificationLevel::Warn, "Playlist only accepts images");
            return;
        }

        if let Some(index) = self
            .playlist
            .iter()
            .position(|playlist_entry| playlist_entry.path == entry.path)
        {
            self.playlist.remove(index);
            self.sync_playlist_state();
            self.notify(NotificationLevel::Info, "Removed from playlist");
            return;
        }

        self.playlist.push(PlaylistEntry {
            path: entry.path,
            transition: self.transition.clone(),
        });
        self.playlist_selected = self.playlist.len().saturating_sub(1);
        self.sync_playlist_state();
        self.notify(NotificationLevel::Success, "Added to playlist");
    }

    fn move_playlist_item(&mut self, delta: isize) {
        if self.playlist.is_empty() {
            return;
        }

        let next = (self.playlist_selected as isize + delta)
            .clamp(0, self.playlist.len().saturating_sub(1) as isize) as usize;
        if next == self.playlist_selected {
            return;
        }

        self.playlist.swap(self.playlist_selected, next);
        self.playlist_selected = next;
        self.sync_playlist_state();
    }

    fn remove_selected_playlist_item(&mut self) {
        if self.playlist.is_empty() {
            return;
        }

        self.playlist.remove(self.playlist_selected);
        self.sync_playlist_state();
    }

    fn clear_playlist(&mut self) {
        self.playlist.clear();
        self.sync_playlist_state();
    }

    fn open_selected_path(&mut self) -> Result<Option<PathBuf>> {
        let Some(entry) = self.selected_browser_entry().cloned() else {
            return Ok(None);
        };

        if entry.is_dir() {
            self.browser_dir = entry.path;
            self.search_query.clear();
            self.reload_browser_dir()?;
            if matches!(entry.kind, model::BrowserEntryKind::Parent) {
                self.notify(NotificationLevel::Info, "Moved to parent directory");
            } else {
                self.notify(NotificationLevel::Info, "Opened directory");
            }
            return Ok(None);
        }

        Ok(Some(entry.path))
    }

    fn reload_browser_dir(&mut self) -> Result<()> {
        self.browser_entries = model::load_browser_entries(&self.browser_dir)?;
        self.apply_filter();
        Ok(())
    }

    fn preview_title(&self) -> String {
        if let Some(entry) = self.selected_browser_entry() {
            let name = entry.name.clone();
            if let Some(monitor) = self.selected_monitor() {
                return format!(" {} -> {} ", name, monitor.name);
            }
            return format!(" {} ", name);
        }

        String::from(" no image selected ")
    }

    fn current_apply_transition(&self) -> TransitionState {
        self.transition.clone()
    }

    fn adjust_playlist_interval(&mut self, delta_secs: i64) {
        let current = i64::try_from(self.playlist_interval.as_secs()).unwrap_or(20);
        let next = (current + delta_secs).clamp(5, 600) as u64;
        self.playlist_interval = Duration::from_secs(next);
    }

    fn cycle_scale_mode(&mut self) {
        self.scale_mode = self.scale_mode.next();
        let label = self.scale_mode.label();
        if self.scale_mode.is_staged() {
            self.notify(
                NotificationLevel::Info,
                format!("Placement: {label} (preview stage)"),
            );
        } else {
            self.notify(NotificationLevel::Info, format!("Placement: {label}"));
        }
    }

    fn cycle_rotation(&mut self) {
        self.rotation = self.rotation.next();
        self.notify(
            NotificationLevel::Info,
            format!("Rotation: {}", self.rotation.label()),
        );
    }

    fn cycle_focus(&mut self, forward: bool) {
        self.focus = if forward {
            self.focus.next()
        } else {
            self.focus.prev()
        };
    }

    fn toggle_playlist_running(&mut self) {
        self.playlist_running = !self.playlist_running;
        self.last_playlist_tick = Instant::now();
        self.notify(
            NotificationLevel::Info,
            if self.playlist_running {
                "Playlist running"
            } else {
                "Playlist paused"
            },
        );
    }

    fn apply_current_browser_selection(&mut self) {
        let Some(path) = self.selected_image_path() else {
            self.notify(NotificationLevel::Warn, "No image selected");
            return;
        };

        request_apply(self, path, self.current_apply_transition());
    }

    fn apply_selected_playlist_item(&mut self) {
        let Some(entry) = self.selected_playlist_entry().cloned() else {
            self.notify(NotificationLevel::Warn, "No queue item selected");
            return;
        };

        request_apply(self, entry.path, entry.transition);
    }

    fn selected_image_path(&self) -> Option<PathBuf> {
        self.selected_browser_entry()
            .filter(|entry| !entry.is_dir())
            .map(|entry| entry.path.clone())
    }

    fn handle_transition_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.transition.selected_field = self.transition.selected_field.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.transition.selected_field = (self.transition.selected_field + 1).min(3);
            }
            KeyCode::Left | KeyCode::Char('h') => self.transition.change_selected(-1),
            KeyCode::Right | KeyCode::Char('l') => self.transition.change_selected(1),
            KeyCode::Char('+') | KeyCode::Char('=') => self.adjust_playlist_interval(5),
            KeyCode::Char('-') => self.adjust_playlist_interval(-5),
            KeyCode::Enter => self.apply_current_browser_selection(),
            _ => {}
        }
    }

    fn handle_library_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('/') => self.input_mode = InputMode::Search,
            KeyCode::Up | KeyCode::Char('k') => self.move_browser(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_browser(1),
            KeyCode::Char('g') => {
                self.browser_selected = 0;
                self.sync_browser_state();
                self.refresh_preview();
            }
            KeyCode::Char('G') => {
                self.browser_selected = self.browser_filtered.len().saturating_sub(1);
                self.sync_browser_state();
                self.refresh_preview();
            }
            KeyCode::Char('p') => self.toggle_playlist_for_selected(),
            KeyCode::Enter => match self.open_selected_path() {
                Ok(Some(path)) => request_apply(self, path, self.current_apply_transition()),
                Ok(None) => {}
                Err(err) => self.notify(NotificationLevel::Error, format!("Open failed: {err:#}")),
            },
            _ => {}
        }
    }

    fn handle_preview_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('f') => self.cycle_scale_mode(),
            KeyCode::Char('r') => self.cycle_rotation(),
            KeyCode::Char('m') => {
                if let Some(name) = self.selected_monitor().map(|monitor| monitor.name.clone())
                    && let Some(index) = self
                        .monitors
                        .iter()
                        .position(|monitor| monitor.name == name)
                {
                    self.toggle_target_for_monitor(index);
                }
            }
            KeyCode::Enter | KeyCode::Char('a') => self.apply_current_browser_selection(),
            _ => {}
        }
    }

    fn handle_playlist_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.move_playlist(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_playlist(1),
            KeyCode::Char('u') => self.move_playlist_item(-1),
            KeyCode::Char('n') => self.move_playlist_item(1),
            KeyCode::Char('d') => self.remove_selected_playlist_item(),
            KeyCode::Char('x') => self.clear_playlist(),
            KeyCode::Char(' ') => self.toggle_playlist_running(),
            KeyCode::Enter => self.apply_selected_playlist_item(),
            _ => {}
        }
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if self.help_open {
            if matches!(code, KeyCode::Esc | KeyCode::Char('?')) {
                self.help_open = false;
            }
            return;
        }

        if self.input_mode == InputMode::Search {
            self.handle_search_key(code, modifiers);
            return;
        }

        if let KeyCode::Char(ch) = code
            && ('1'..='4').contains(&ch)
        {
            let index = (ch as u8 - b'1') as usize;
            self.toggle_target_for_monitor(index);
            return;
        }

        match code {
            KeyCode::Char('f') if modifiers.contains(KeyModifiers::SHIFT) => {
                self.scale_mode = self.scale_mode.prev();
                self.notify(
                    NotificationLevel::Info,
                    format!("Placement: {}", self.scale_mode.label()),
                );
            }
            KeyCode::Char('r') if modifiers.contains(KeyModifiers::SHIFT) => {
                self.rotation = self.rotation.prev();
                self.notify(
                    NotificationLevel::Info,
                    format!("Rotation: {}", self.rotation.label()),
                );
            }
            KeyCode::Char('q') => {
                self.should_quit = true;
                self.notify(NotificationLevel::Info, "Exiting TUI");
            }
            KeyCode::Char('?') => self.help_open = true,
            KeyCode::Char('L') => self.toggle_activity_log(),
            KeyCode::Tab => {
                if matches!(
                    self.focus,
                    model::FocusRegion::Library | model::FocusRegion::Playlist
                ) {
                    self.toggle_left_tab();
                } else {
                    self.cycle_focus(true);
                }
            }
            KeyCode::BackTab => self.cycle_focus(false),
            KeyCode::Esc => {}
            KeyCode::Char('b') => ensure_backend_running(self),
            KeyCode::Char('r') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.request_monitor_refresh()
            }
            KeyCode::Char('r') => self.cycle_rotation(),
            KeyCode::Char('f') => self.cycle_scale_mode(),
            KeyCode::Char('a') => self.apply_current_browser_selection(),
            KeyCode::Char('m') => {
                if let Some(name) = self.active_monitor_name.clone()
                    && let Some(index) = self
                        .monitors
                        .iter()
                        .position(|monitor| monitor.name == name)
                {
                    self.toggle_target_for_monitor(index);
                }
            }
            KeyCode::Char('A') => self.select_all_targets(),
            KeyCode::Char('x') => {
                if matches!(self.focus, model::FocusRegion::Playlist) {
                    self.clear_playlist();
                } else {
                    self.clear_targets();
                }
            }
            KeyCode::Char(' ') => self.toggle_playlist_running(),
            KeyCode::Enter => match self.focus {
                model::FocusRegion::Library => self.handle_library_key(KeyCode::Enter),
                model::FocusRegion::Preview => self.apply_current_browser_selection(),
                model::FocusRegion::Playlist => self.apply_selected_playlist_item(),
                model::FocusRegion::Transitions => self.apply_current_browser_selection(),
            },
            _ => match self.focus {
                model::FocusRegion::Library => self.handle_library_key(code),
                model::FocusRegion::Preview => self.handle_preview_key(code),
                model::FocusRegion::Playlist => self.handle_playlist_key(code),
                model::FocusRegion::Transitions => self.handle_transition_key(code),
            },
        }
    }

    fn handle_search_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match code {
            KeyCode::Esc | KeyCode::Enter => {
                self.input_mode = InputMode::Normal;
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.apply_filter();
            }
            KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.search_query.clear();
                self.apply_filter();
            }
            KeyCode::Char(ch) => {
                if !ch.is_control() {
                    self.search_query.push(ch);
                    self.apply_filter();
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn daemon_status_text(app: &App) -> &'static str {
    if app.daemon_online {
        "online"
    } else {
        "offline"
    }
}

pub(crate) fn daemon_status_color(app: &App) -> ratatui::style::Color {
    if app.daemon_online {
        ratatui::style::Color::Rgb(120, 210, 151)
    } else {
        ratatui::style::Color::Rgb(245, 193, 96)
    }
}

pub(crate) fn app_key_hints(app: &App) -> String {
    if app.input_mode == InputMode::Search {
        return String::from("Search: type | Backspace delete | Ctrl+u clear | Enter/Esc done");
    }

    match app.focus {
        model::FocusRegion::Library => String::from(
            "j/k browse | Enter open/apply | p queue | / search | g/G jump | Tab queue",
        ),
        model::FocusRegion::Preview => {
            String::from("f fit mode | r rotate | Enter apply | m toggle active target | Tab next")
        }
        model::FocusRegion::Playlist => {
            String::from("j/k select | u/n reorder | d delete | x clear | Space play | Tab library")
        }
        model::FocusRegion::Transitions => {
            String::from("j/k field | h/l change | +/- interval | Enter apply | Tab next")
        }
    }
}

pub(crate) fn global_key_hints() -> &'static str {
    "q quit | ? help | L activity log | Ctrl+r refresh monitors | b launch daemon"
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

    let mut app = App::new(event_tx);
    app.init_preview_picker();
    if let Err(err) = app.reload_browser_dir() {
        app.notify(
            NotificationLevel::Error,
            format!("Browser initialization failed: {err:#}"),
        );
    }

    ensure_backend_running(&mut app);
    app.request_monitor_refresh();

    let mut tick = time::interval(Duration::from_millis(TICK_RATE_MS));
    while !app.should_quit {
        tokio::select! {
            _ = tick.tick() => {
                handle_input(&mut app).await?;
                refresh_monitors_if_requested(&mut app).await;
                probe_daemon_if_due(&mut app);
                poll_backend_child(&mut app).await;
                run_playlist_tick(&mut app);
                app.prune_notifications();
                terminal.draw(|frame| render::draw(frame, &mut app))?;
            }
            Some(event) = event_rx.recv() => {
                handle_app_event(&mut app, event);
            }
        }
    }

    app.backend_child = None;

    Ok(())
}

fn ensure_backend_running(app: &mut App) {
    if backend::daemon_alive(&app.daemon_namespace) {
        app.daemon_online = true;
        app.notify(NotificationLevel::Success, "Connected to running daemon");
        app.request_monitor_refresh();
        return;
    }

    match backend::launch_daemon_subprocess(&app.daemon_namespace) {
        Ok(child) => {
            app.backend_child = Some(child);
            app.daemon_online = true;
            app.notify(NotificationLevel::Success, "Daemon started");
            app.request_monitor_refresh();
        }
        Err(err) => {
            app.daemon_online = false;
            app.notify(
                NotificationLevel::Error,
                format!("Failed to launch daemon: {err:#}"),
            );
        }
    }
}

fn probe_daemon_if_due(app: &mut App) {
    if app.last_daemon_probe.elapsed() < DAEMON_PROBE_INTERVAL {
        return;
    }

    app.last_daemon_probe = Instant::now();
    app.daemon_online = backend::daemon_alive(&app.daemon_namespace);
}

async fn poll_backend_child(app: &mut App) {
    let Some(child) = app.backend_child.as_mut() else {
        return;
    };

    match child.try_wait() {
        Ok(Some(status)) => {
            app.backend_child = None;
            if status.success() {
                app.notify(NotificationLevel::Warn, "Daemon process exited");
            } else {
                app.notify(
                    NotificationLevel::Error,
                    format!("Daemon exited with status {status}"),
                );
            }
            app.daemon_online = backend::daemon_alive(&app.daemon_namespace);
        }
        Ok(None) => {}
        Err(err) => {
            app.backend_child = None;
            app.notify(
                NotificationLevel::Error,
                format!("Daemon process error: {err}"),
            );
            app.daemon_online = false;
        }
    }
}

fn handle_app_event(app: &mut App, event: AppEvent) {
    match event {
        AppEvent::ApplyFinished(result) => {
            app.ipc_in_flight = false;
            match result {
                Ok(msg) => app.notify(NotificationLevel::Success, msg),
                Err(err) => app.notify(NotificationLevel::Error, format!("Apply failed: {err}")),
            }
        }
    }
}

async fn refresh_monitors_if_requested(app: &mut App) {
    if !app.refresh_monitors {
        return;
    }

    app.refresh_monitors = false;

    match model::discover_monitors().await {
        Ok(monitors) => {
            let count = monitors.len();
            app.monitors = monitors;
            if !app.selected_targets.is_empty() {
                let available_names = app
                    .monitors
                    .iter()
                    .map(|monitor| monitor.name.clone())
                    .collect::<HashSet<_>>();
                app.selected_targets
                    .retain(|name| available_names.contains(name));
            }
            app.sync_monitor_state();
            if !app.targets_initialized {
                app.select_all_targets();
                app.targets_initialized = true;
            }
            app.notify(
                NotificationLevel::Success,
                format!("Detected {count} monitor(s)"),
            );
        }
        Err(err) => {
            app.notify(
                NotificationLevel::Error,
                format!("Monitor discovery failed: {err:#}"),
            );
        }
    }
}

fn run_playlist_tick(app: &mut App) {
    if app.ipc_in_flight || !app.playlist_running || app.playlist.is_empty() {
        return;
    }

    if app.last_playlist_tick.elapsed() < app.playlist_interval {
        return;
    }

    let index = app
        .playlist_selected
        .min(app.playlist.len().saturating_sub(1));
    let entry = app.playlist[index].clone();
    app.playlist_selected = (index + 1) % app.playlist.len();
    app.sync_playlist_state();
    app.last_playlist_tick = Instant::now();
    request_apply(app, entry.path, entry.transition);
}

fn request_apply(app: &mut App, path: PathBuf, transition: TransitionState) {
    if app.ipc_in_flight {
        app.notify(NotificationLevel::Warn, "Apply already in flight");
        return;
    }

    if !app.daemon_online {
        app.notify(NotificationLevel::Warn, "Daemon offline; press b to launch");
        return;
    }

    app.ipc_in_flight = true;
    app.notify(
        NotificationLevel::Info,
        format!(
            "Applying {}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("image")
        ),
    );

    let namespace = app.daemon_namespace.clone();
    let selected_outputs = app.selected_output_names();
    let scale_mode = app.scale_mode;
    let rotation = app.rotation;
    let tx = app.event_tx.clone();

    tokio::spawn(async move {
        let result = backend::perform_apply_request(
            path,
            transition,
            namespace,
            selected_outputs,
            scale_mode,
            rotation,
        )
        .await;

        let _ = tx.send(AppEvent::ApplyFinished(result));
    });
}

async fn handle_input(app: &mut App) -> io::Result<()> {
    while event::poll(Duration::from_millis(0))? {
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.handle_key(key.code, key.modifiers);
        }
    }

    Ok(())
}
