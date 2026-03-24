mod backend;
mod data;
mod ui;

use std::{
    collections::{HashMap, VecDeque},
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
use data::{
    BrowserEntry, ImagePreview, InputMode, MonitorEntry, Notification, NotificationLevel, Panel,
    Rotation, ScaleMode, TransitionState,
};
use ratatui::{Terminal, backend::CrosstermBackend, widgets::ListState};
use tokio::{
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    time,
};
use vellum_core::{VellumServer, VellumServerConfig};

const TICK_RATE_MS: u64 = 33;
const MONITOR_REFRESH_INTERVAL: Duration = Duration::from_secs(8);
const DAEMON_PROBE_INTERVAL: Duration = Duration::from_secs(2);
const NOTIFICATION_TTL: Duration = Duration::from_secs(5);

pub async fn run_entrypoint() -> io::Result<()> {
    if let Some(namespace) = daemon_launch_namespace_from_args() {
        let config = VellumServerConfig {
            namespace,
            quiet: true,
            ..VellumServerConfig::default()
        };
        let server = VellumServer::new(config);
        return server.run().map_err(io::Error::other);
    }

    run_app().await
}

fn daemon_launch_namespace_from_args() -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg != "--daemon-subprocess" {
            continue;
        }

        let mut namespace = String::new();
        while let Some(flag) = args.next() {
            if flag == "--namespace" {
                namespace = args.next().unwrap_or_default();
                break;
            }
        }

        return Some(namespace);
    }

    None
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
    pub(crate) panel: Panel,
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
    pub(crate) monitor_state: ListState,

    pub(crate) selected_preview: Option<ImagePreview>,
    image_dim_cache: HashMap<PathBuf, Option<(u32, u32)>>,

    pub(crate) playlist: Vec<PathBuf>,
    pub(crate) playlist_running: bool,
    pub(crate) playlist_interval: Duration,
    playlist_index: usize,
    last_playlist_tick: Instant,

    pub(crate) scale_mode: ScaleMode,
    pub(crate) rotation: Rotation,
    pub(crate) transition: TransitionState,

    pub(crate) notifications: VecDeque<Notification>,
    pub(crate) status: String,

    pub(crate) daemon_namespace: String,
    backend_child: Option<tokio::process::Child>,
    pub(crate) daemon_online: bool,
    last_daemon_probe: Instant,
    last_monitor_refresh: Option<Instant>,
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

        Self {
            should_quit: false,
            panel: Panel::Library,
            input_mode: InputMode::Normal,
            help_open: false,
            browser_dir: data::preferred_initial_browser_dir(),
            search_query: String::new(),
            browser_entries: Vec::new(),
            browser_filtered: Vec::new(),
            browser_selected: 0,
            browser_state,
            monitors: Vec::new(),
            monitor_selected: 0,
            monitor_state,
            selected_preview: None,
            image_dim_cache: HashMap::new(),
            playlist: Vec::new(),
            playlist_running: false,
            playlist_interval: Duration::from_secs(20),
            playlist_index: 0,
            last_playlist_tick: Instant::now(),
            scale_mode: ScaleMode::Fit,
            rotation: Rotation::Deg0,
            transition: TransitionState::default(),
            notifications: VecDeque::new(),
            status: String::from("Welcome to Vellum"),
            daemon_namespace: String::from("vellum-tui"),
            backend_child: None,
            daemon_online: false,
            last_daemon_probe: Instant::now() - DAEMON_PROBE_INTERVAL,
            last_monitor_refresh: None,
            refresh_monitors: true,
            ipc_in_flight: false,
            event_tx,
        }
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
            .is_some_and(|n| n.created_at.elapsed() > NOTIFICATION_TTL)
        {
            let _ = self.notifications.pop_front();
        }
    }

    fn selected_browser_entry(&self) -> Option<&BrowserEntry> {
        let idx = *self.browser_filtered.get(self.browser_selected)?;
        self.browser_entries.get(idx)
    }

    fn selected_monitor(&self) -> Option<&MonitorEntry> {
        self.monitors.get(self.monitor_selected)
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
            self.monitor_state.select(None);
            return;
        }

        self.monitor_selected = self
            .monitor_selected
            .min(self.monitors.len().saturating_sub(1));
        self.monitor_state.select(Some(self.monitor_selected));
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

    fn move_monitor(&mut self, delta: isize) {
        if self.monitors.is_empty() {
            return;
        }
        let max = self.monitors.len().saturating_sub(1) as isize;
        self.monitor_selected = (self.monitor_selected as isize + delta).clamp(0, max) as usize;
        self.sync_monitor_state();
    }

    fn apply_filter(&mut self) {
        self.browser_filtered = data::fuzzy_filter(&self.browser_entries, &self.search_query);
        self.sync_browser_state();
        self.refresh_preview();
    }

    fn refresh_preview(&mut self) {
        let Some(entry) = self.selected_browser_entry() else {
            self.selected_preview = None;
            return;
        };
        if entry.is_dir {
            self.selected_preview = None;
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
            path: image_path,
            width,
            height,
        });
    }

    fn toggle_playlist_for_selected(&mut self) {
        let Some(entry) = self.selected_browser_entry().cloned() else {
            self.notify(NotificationLevel::Warn, "No selected image");
            return;
        };
        if entry.is_dir {
            self.notify(NotificationLevel::Warn, "Playlist only accepts images");
            return;
        }

        if let Some(pos) = self.playlist.iter().position(|p| p == &entry.path) {
            self.playlist.remove(pos);
            self.notify(NotificationLevel::Info, "Removed from playlist");
            return;
        }

        self.playlist.push(entry.path);
        self.notify(NotificationLevel::Success, "Added to playlist");
    }

    fn selected_monitor_name(&self) -> Option<String> {
        self.selected_monitor().map(|m| m.name.clone())
    }

    fn selected_image_path(&self) -> Option<PathBuf> {
        self.selected_browser_entry()
            .filter(|entry| !entry.is_dir)
            .map(|entry| entry.path.clone())
    }

    fn open_selected_path(&mut self) -> Result<Option<PathBuf>> {
        let Some(entry) = self.selected_browser_entry().cloned() else {
            return Ok(None);
        };

        if entry.is_dir {
            self.browser_dir = entry.path;
            self.search_query.clear();
            self.reload_browser_dir()?;
            if entry.is_parent {
                self.notify(NotificationLevel::Info, "Moved to parent directory");
            } else {
                self.notify(NotificationLevel::Info, "Opened directory");
            }
            return Ok(None);
        }

        Ok(Some(entry.path))
    }

    fn reload_browser_dir(&mut self) -> Result<()> {
        self.browser_entries = data::load_browser_entries(&self.browser_dir)?;
        self.apply_filter();
        Ok(())
    }
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
    if let Err(err) = app.reload_browser_dir() {
        app.notify(
            NotificationLevel::Error,
            format!("Browser initialization failed: {err:#}"),
        );
    }

    ensure_backend_running(&mut app);

    let mut tick = time::interval(Duration::from_millis(TICK_RATE_MS));
    while !app.should_quit {
        tokio::select! {
            _ = tick.tick() => {
                handle_input(&mut app).await?;
                refresh_monitors_if_due(&mut app).await;
                probe_daemon_if_due(&mut app);
                poll_backend_child(&mut app).await;
                run_playlist_tick(&mut app);
                app.prune_notifications();
                terminal.draw(|frame| ui::draw(frame, &app))?;
            }
            Some(event) = event_rx.recv() => {
                handle_app_event(&mut app, event);
            }
        }
    }

    // Detach from daemon process. Wallpapers should stay visible after TUI exit.
    app.backend_child = None;

    Ok(())
}

fn ensure_backend_running(app: &mut App) {
    if backend::daemon_alive(&app.daemon_namespace) {
        app.daemon_online = true;
        app.notify(NotificationLevel::Success, "Connected to running daemon");
        return;
    }

    match backend::launch_daemon_subprocess(&app.daemon_namespace) {
        Ok(child) => {
            app.backend_child = Some(child);
            app.daemon_online = true;
            app.notify(NotificationLevel::Success, "Daemon started");
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

async fn refresh_monitors_if_due(app: &mut App) {
    let timed_due = app
        .last_monitor_refresh
        .is_none_or(|at| at.elapsed() >= MONITOR_REFRESH_INTERVAL);
    if !app.refresh_monitors && !timed_due {
        return;
    }

    match data::discover_monitors().await {
        Ok(monitors) => {
            let count = monitors.len();
            app.monitors = monitors;
            app.sync_monitor_state();
            app.last_monitor_refresh = Some(Instant::now());
            app.refresh_monitors = false;
            app.notify(
                NotificationLevel::Success,
                format!("Detected {count} monitor(s)"),
            );
        }
        Err(err) => {
            app.refresh_monitors = false;
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

    let idx = app.playlist_index % app.playlist.len();
    app.playlist_index = (app.playlist_index + 1) % app.playlist.len();
    app.last_playlist_tick = Instant::now();

    let path = app.playlist[idx].clone();
    request_apply(app, path);
}

fn request_apply(app: &mut App, path: PathBuf) {
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
            path.file_name().and_then(|s| s.to_str()).unwrap_or("image")
        ),
    );

    let namespace = app.daemon_namespace.clone();
    let preferred_output = app.selected_monitor_name();
    let transition = app.transition.clone();
    let scale_mode = app.scale_mode;
    let rotation = app.rotation;
    let tx = app.event_tx.clone();

    tokio::spawn(async move {
        let result = backend::perform_apply_request(
            path,
            transition,
            namespace,
            preferred_output,
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
            if matches!(key.code, KeyCode::Char('q')) {
                app.should_quit = true;
                app.notify(NotificationLevel::Info, "Exiting TUI");
                continue;
            }

            if app.help_open {
                app.help_open = false;
                continue;
            }

            if app.input_mode == InputMode::Search {
                handle_search_input(app, key.code, key.modifiers);
                continue;
            }

            if let KeyCode::Char(ch) = key.code
                && ('1'..='9').contains(&ch)
            {
                let idx = (ch as u8 - b'1') as usize;
                if idx < app.monitors.len() {
                    app.monitor_selected = idx;
                    app.sync_monitor_state();
                    app.notify(
                        NotificationLevel::Info,
                        format!("Active monitor: {}", app.monitors[idx].name),
                    );
                }
                continue;
            }

            match key.code {
                KeyCode::Char('?') => app.help_open = true,
                KeyCode::Char('b') => ensure_backend_running(app),
                KeyCode::Char('r') => app.refresh_monitors = true,
                KeyCode::Tab => app.panel = app.panel.next(),
                KeyCode::BackTab => app.panel = app.panel.prev(),
                _ => handle_panel_input(app, key.code),
            }
        }
    }

    Ok(())
}

fn handle_search_input(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    match code {
        KeyCode::Esc | KeyCode::Enter => {
            app.input_mode = InputMode::Normal;
        }
        KeyCode::Backspace => {
            app.search_query.pop();
            app.apply_filter();
        }
        KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
            app.search_query.clear();
            app.apply_filter();
        }
        KeyCode::Char(ch) => {
            if !ch.is_control() {
                app.search_query.push(ch);
                app.apply_filter();
            }
        }
        _ => {}
    }
}

fn handle_panel_input(app: &mut App, code: KeyCode) {
    match app.panel {
        Panel::Library => handle_library_panel_input(app, code),
        Panel::Monitor => handle_monitor_panel_input(app, code),
        Panel::Playback => handle_playback_panel_input(app, code),
    }
}

fn handle_library_panel_input(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('/') => app.input_mode = InputMode::Search,
        KeyCode::Up | KeyCode::Char('k') => app.move_browser(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_browser(1),
        KeyCode::Char('g') => {
            app.browser_selected = 0;
            app.sync_browser_state();
            app.refresh_preview();
        }
        KeyCode::Char('G') => {
            app.browser_selected = app.browser_filtered.len().saturating_sub(1);
            app.sync_browser_state();
            app.refresh_preview();
        }
        KeyCode::Char('p') => app.toggle_playlist_for_selected(),
        KeyCode::Char(' ') => {
            app.playlist_running = !app.playlist_running;
            app.last_playlist_tick = Instant::now();
            app.notify(
                NotificationLevel::Info,
                if app.playlist_running {
                    "Playlist running"
                } else {
                    "Playlist paused"
                },
            );
        }
        KeyCode::Enter => match app.open_selected_path() {
            Ok(Some(path)) => request_apply(app, path),
            Ok(None) => {}
            Err(err) => app.notify(NotificationLevel::Error, format!("Open failed: {err:#}")),
        },
        _ => {}
    }
}

fn handle_monitor_panel_input(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Up | KeyCode::Char('k') => app.move_monitor(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_monitor(1),
        KeyCode::Char('f') => {
            app.scale_mode = app.scale_mode.next();
            app.notify(
                NotificationLevel::Info,
                format!("Scale mode: {}", app.scale_mode.as_str()),
            );
        }
        KeyCode::Char('o') => {
            app.rotation = app.rotation.next();
            app.notify(
                NotificationLevel::Info,
                format!("Rotation: {}deg", app.rotation.degrees()),
            );
        }
        KeyCode::Char('a') | KeyCode::Enter => {
            if let Some(path) = app.selected_image_path() {
                request_apply(app, path);
            } else {
                app.notify(NotificationLevel::Warn, "No selected image to apply");
            }
        }
        _ => {}
    }
}

fn handle_playback_panel_input(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Up | KeyCode::Char('k') => {
            app.transition.selected_field = app.transition.selected_field.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.transition.selected_field = (app.transition.selected_field + 1).min(3);
        }
        KeyCode::Left | KeyCode::Char('h') => app.transition.change_selected(-1),
        KeyCode::Right | KeyCode::Char('l') => app.transition.change_selected(1),
        KeyCode::Char('+') | KeyCode::Char('=') => {
            let secs = app.playlist_interval.as_secs();
            app.playlist_interval = Duration::from_secs((secs + 5).min(600));
        }
        KeyCode::Char('-') => {
            let secs = app.playlist_interval.as_secs();
            app.playlist_interval = Duration::from_secs(secs.saturating_sub(5).max(5));
        }
        KeyCode::Char(' ') => {
            app.playlist_running = !app.playlist_running;
            app.last_playlist_tick = Instant::now();
        }
        KeyCode::Enter => {
            if let Some(path) = app.selected_image_path() {
                request_apply(app, path);
            } else if let Some(path) = app.playlist.first() {
                request_apply(app, path.clone());
            } else {
                app.notify(NotificationLevel::Warn, "No image available for apply");
            }
        }
        _ => {}
    }
}

pub(crate) fn monitor_slot_labels(monitors: &[MonitorEntry]) -> String {
    if monitors.is_empty() {
        return String::from("No monitors");
    }

    monitors
        .iter()
        .take(9)
        .enumerate()
        .map(|(i, m)| format!("{}:{}", i + 1, m.name))
        .collect::<Vec<_>>()
        .join("  ")
}

pub(crate) fn selected_preview_simulation(app: &App) -> Option<data::AspectSimulation> {
    let preview = app.selected_preview.as_ref()?;
    let monitor = app.selected_monitor()?;
    Some(data::simulate_aspect(
        (preview.width, preview.height),
        (monitor.width, monitor.height),
        app.scale_mode,
        app.rotation,
    ))
}

pub(crate) fn preview_ascii(app: &App, width: usize, height: usize) -> Vec<String> {
    let Some(monitor) = app.selected_monitor() else {
        return vec![String::from("Select a monitor")];
    };

    let preview = app.selected_preview.as_ref();
    let mw = monitor.width.max(1) as f32;
    let mh = monitor.height.max(1) as f32;

    let available_w = width.max(8) as f32;
    let available_h = height.max(4) as f32;
    let frame_scale = (available_w / mw).min(available_h / mh);

    let frame_w = ((mw * frame_scale).round() as usize).clamp(8, width.max(8));
    let frame_h = ((mh * frame_scale).round() as usize).clamp(4, height.max(4));

    let mut grid = vec![vec![' '; frame_w]; frame_h];
    for x in 0..frame_w {
        grid[0][x] = '█';
        grid[frame_h - 1][x] = '█';
    }
    for row in &mut grid {
        row[0] = '█';
        row[frame_w - 1] = '█';
    }

    if let Some(preview) = preview {
        let sim = data::simulate_aspect(
            (preview.width, preview.height),
            (monitor.width, monitor.height),
            app.scale_mode,
            app.rotation,
        );

        let fill_w = ((sim.target_width as f32 / monitor.width as f32) * (frame_w as f32 - 2.0))
            .round()
            .clamp(1.0, frame_w as f32 - 2.0) as usize;
        let fill_h = ((sim.target_height as f32 / monitor.height as f32) * (frame_h as f32 - 2.0))
            .round()
            .clamp(1.0, frame_h as f32 - 2.0) as usize;

        let start_x = (frame_w.saturating_sub(fill_w + 2)) / 2 + 1;
        let start_y = (frame_h.saturating_sub(fill_h + 2)) / 2 + 1;

        for y in start_y..(start_y + fill_h).min(frame_h - 1) {
            for x in start_x..(start_x + fill_w).min(frame_w - 1) {
                grid[y][x] = if matches!(app.scale_mode, ScaleMode::Fill) {
                    '▓'
                } else {
                    '▒'
                };
            }
        }
    }

    grid.into_iter()
        .map(|row| row.into_iter().collect::<String>())
        .collect()
}

pub(crate) fn daemon_status_text(app: &App) -> &'static str {
    if app.daemon_online {
        "online"
    } else {
        "offline"
    }
}

pub(crate) fn daemon_status_color_online(app: &App) -> bool {
    app.daemon_online
}

pub(crate) fn panel_key_hints(app: &App) -> &'static str {
    if app.input_mode == InputMode::Search {
        return "Search: type | Backspace | Ctrl+u clear | Enter/Esc done";
    }

    match app.panel {
        Panel::Library => {
            "Tab switch panel | j/k move | Enter open/apply | p add/remove playlist | / search | Space play"
        }
        Panel::Monitor => {
            "Tab switch panel | j/k monitor | 1..9 quick monitor | f scale | o rotate | a/Enter apply"
        }
        Panel::Playback => {
            "Tab switch panel | j/k field | h/l change | +/- playlist interval | Space play | Enter apply"
        }
    }
}

pub(crate) fn global_key_hints() -> &'static str {
    "q quit | ? help | r refresh monitors | b start daemon"
}
