//! Application state and input handling for `awww-tui`.

use std::{path::PathBuf, time::Duration};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{
    backend::{awww::TransitionKind, monitors::MonitorInfo},
    persistence::{StoredPlaylist, StoredProfile, StoredState},
    wallpapers::{fuzzy_filter_indices, WallpaperItem},
};

/// Active high-level pane in the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPane {
    /// Left file browser pane.
    Browser,
    /// Middle monitor preview pane.
    Monitor,
    /// Right transition settings pane.
    Transition,
}

impl FocusPane {
    /// Moves focus to the previous pane in a cyclic order.
    #[must_use]
    pub fn move_left(self) -> Self {
        match self {
            Self::Browser => Self::Transition,
            Self::Monitor => Self::Browser,
            Self::Transition => Self::Monitor,
        }
    }

    /// Moves focus to the next pane in a cyclic order.
    #[must_use]
    pub fn move_right(self) -> Self {
        match self {
            Self::Browser => Self::Monitor,
            Self::Monitor => Self::Transition,
            Self::Transition => Self::Browser,
        }
    }
}

/// Available transition setting rows in the transition pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionField {
    /// Transition kind row.
    Kind,
    /// Transition step row.
    Step,
    /// Transition FPS row.
    Fps,
}

impl TransitionField {
    /// Moves to the previous transition row.
    #[must_use]
    pub fn prev(self) -> Self {
        match self {
            Self::Kind => Self::Fps,
            Self::Step => Self::Kind,
            Self::Fps => Self::Step,
        }
    }

    /// Moves to the next transition row.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Kind => Self::Step,
            Self::Step => Self::Fps,
            Self::Fps => Self::Kind,
        }
    }
}

/// Aspect simulation mode used by monitor preview pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AspectMode {
    /// Contain image fully inside monitor bounds.
    Fit,
    /// Fill monitor and crop overflow.
    Fill,
    /// Hard crop preserving monitor aspect.
    Crop,
}

impl AspectMode {
    /// Cycles to the next aspect mode.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Fit => Self::Fill,
            Self::Fill => Self::Crop,
            Self::Crop => Self::Fit,
        }
    }

    /// Returns lowercase label for rendering and persistence.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Fit => "fit",
            Self::Fill => "fill",
            Self::Crop => "crop",
        }
    }

    /// Parses mode label from persisted state.
    #[must_use]
    pub fn from_label(label: &str) -> Self {
        match label {
            "fill" => Self::Fill,
            "crop" => Self::Crop,
            _ => Self::Fit,
        }
    }
}

/// Runtime control message for the background playlist worker.
#[derive(Debug, Clone, Copy)]
pub struct PlaylistControl {
    /// Whether playlist cycling is enabled.
    pub enabled: bool,
    /// Tick interval used for auto-cycling.
    pub interval: Duration,
}

/// Computed aspect-ratio simulation snapshot.
#[derive(Debug, Clone, Copy)]
pub struct AspectSimulation {
    /// Source image width.
    pub image_width: u32,
    /// Source image height.
    pub image_height: u32,
    /// Monitor width.
    pub monitor_width: u32,
    /// Monitor height.
    pub monitor_height: u32,
    /// Simulated draw width on monitor canvas.
    pub draw_width: u32,
    /// Simulated draw height on monitor canvas.
    pub draw_height: u32,
    /// Horizontal crop amount in source pixels.
    pub crop_x: u32,
    /// Vertical crop amount in source pixels.
    pub crop_y: u32,
}

/// Side-effect request emitted by key handling for runtime execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAction {
    /// Apply selected wallpaper as a temporary live preview.
    TryOnSelected,
    /// Confirm currently selected wallpaper as committed choice.
    ConfirmSelected,
    /// Revert temporary preview to the last confirmed wallpaper.
    CancelPreview,
    /// Persist current selection as quick profile.
    SaveQuickProfile,
    /// Load quick profile and apply it.
    LoadQuickProfile,
    /// Toggle background playlist worker.
    TogglePlaylist,
    /// Apply next wallpaper from active playlist.
    AutoCycleNext,
}

/// Global application state shared by input, runtime, and renderer.
#[derive(Debug, Clone)]
pub struct App {
    /// Whether the application should terminate on the next loop iteration.
    pub should_quit: bool,
    /// The pane currently selected by keyboard navigation.
    pub focus: FocusPane,
    /// Monotonic tick counter used for subtle theme animation.
    pub ticks: u64,
    /// Full wallpaper list discovered from filesystem.
    pub wallpapers: Vec<WallpaperItem>,
    /// Filtered indices into `wallpapers`, ranked by fuzzy matching.
    pub filtered_wallpaper_indices: Vec<usize>,
    /// Selected row index in the filtered list.
    pub selected_wallpaper_row: usize,
    /// Discovered monitor list from backend query.
    pub monitors: Vec<MonitorInfo>,
    /// Selected monitor row index.
    pub selected_monitor: usize,
    /// Transition row currently selected in the transition pane.
    pub transition_field: TransitionField,
    /// Transition kind value.
    pub transition_kind: TransitionKind,
    /// Transition step value.
    pub transition_step: u16,
    /// Transition FPS value.
    pub transition_fps: u16,
    /// Browser fuzzy search query.
    pub search_query: String,
    /// Whether the browser is in search-input mode.
    pub search_mode: bool,
    /// Status line rendered in footer.
    pub status: String,
    /// Last confirmed wallpaper path for cancel/revert behavior.
    pub confirmed_wallpaper: Option<PathBuf>,
    /// Whether a temporary preview is currently active.
    pub preview_active: bool,
    /// Active aspect simulation mode.
    pub aspect_mode: AspectMode,
    /// Persisted state data for profiles and playlists.
    pub stored_state: StoredState,
    /// Active playlist index in stored state.
    pub active_playlist: Option<usize>,
    /// Current playlist cursor.
    pub playlist_cursor: usize,
    /// Whether auto-cycling is enabled.
    pub playlist_enabled: bool,
}

impl App {
    /// Builds initial app state from discovered wallpapers, monitors, and persisted data.
    #[must_use]
    pub fn new(
        wallpapers: Vec<WallpaperItem>,
        monitors: Vec<MonitorInfo>,
        stored_state: StoredState,
    ) -> Self {
        let mut app = Self {
            should_quit: false,
            focus: FocusPane::Browser,
            ticks: 0,
            wallpapers,
            filtered_wallpaper_indices: Vec::new(),
            selected_wallpaper_row: 0,
            monitors,
            selected_monitor: 0,
            transition_field: TransitionField::Kind,
            transition_kind: TransitionKind::Fade,
            transition_step: 16,
            transition_fps: 60,
            search_query: String::new(),
            search_mode: false,
            status: String::from("ready"),
            confirmed_wallpaper: None,
            preview_active: false,
            aspect_mode: AspectMode::Fit,
            stored_state,
            active_playlist: None,
            playlist_cursor: 0,
            playlist_enabled: false,
        };

        app.rebuild_filter();
        app.initialize_playlist_state();
        app
    }

    /// Handles a terminal key event and returns emitted runtime actions.
    pub fn on_key(&mut self, key: KeyEvent) -> Vec<AppAction> {
        let mut actions = Vec::new();

        if self.handle_search_input(key, &mut actions) {
            return actions;
        }

        match key.code {
            KeyCode::Char('q') => {
                if self.preview_active {
                    actions.push(AppAction::CancelPreview);
                }
                self.should_quit = true;
            }
            KeyCode::Esc => {
                if self.preview_active {
                    actions.push(AppAction::CancelPreview);
                }
                self.should_quit = true;
            }
            KeyCode::Char('h') | KeyCode::Left => self.focus = self.focus.move_left(),
            KeyCode::Char('l') | KeyCode::Right => self.focus = self.focus.move_right(),
            KeyCode::Char('j') | KeyCode::Down => self.navigate_next(&mut actions),
            KeyCode::Char('k') | KeyCode::Up => self.navigate_previous(&mut actions),
            KeyCode::Char('H') => self.adjust_left(&mut actions),
            KeyCode::Char('L') => self.adjust_right(&mut actions),
            KeyCode::Char('/') if self.focus == FocusPane::Browser => {
                self.search_mode = true;
                self.set_status("search mode: type to fuzzy filter, Enter to exit");
            }
            KeyCode::Char('m') => {
                self.aspect_mode = self.aspect_mode.next();
                self.set_status(format!("aspect mode: {}", self.aspect_mode.label()));
            }
            KeyCode::Char('p') => actions.push(AppAction::SaveQuickProfile),
            KeyCode::Char('o') => actions.push(AppAction::LoadQuickProfile),
            KeyCode::Char('g') => actions.push(AppAction::TogglePlaylist),
            KeyCode::Char(']') => actions.push(AppAction::AutoCycleNext),
            KeyCode::Enter => actions.push(AppAction::ConfirmSelected),
            KeyCode::Char('c') => actions.push(AppAction::CancelPreview),
            _ => {}
        }

        actions
    }

    /// Advances periodic state once per tick interval.
    pub fn on_tick(&mut self) {
        self.ticks = self.ticks.saturating_add(1);
    }

    /// Stores a user-visible status line.
    pub fn set_status(&mut self, status: impl Into<String>) {
        self.status = status.into();
    }

    /// Marks preview state as active after a successful live apply.
    pub fn mark_preview_active(&mut self) {
        self.preview_active = true;
    }

    /// Marks selected wallpaper as confirmed after a successful apply.
    pub fn mark_confirmed(&mut self) {
        self.confirmed_wallpaper = self.selected_wallpaper_path();
        self.preview_active = false;
    }

    /// Clears preview active state after cancel or revert completes.
    pub fn clear_preview(&mut self) {
        self.preview_active = false;
    }

    /// Returns currently selected wallpaper item, if any.
    #[must_use]
    pub fn selected_wallpaper(&self) -> Option<&WallpaperItem> {
        let index = *self
            .filtered_wallpaper_indices
            .get(self.selected_wallpaper_row)?;
        self.wallpapers.get(index)
    }

    /// Returns selected wallpaper path cloned for runtime command execution.
    #[must_use]
    pub fn selected_wallpaper_path(&self) -> Option<PathBuf> {
        self.selected_wallpaper().map(|item| item.path.clone())
    }

    /// Returns selected monitor name for per-output wallpaper application.
    #[must_use]
    pub fn selected_monitor_name(&self) -> Option<&str> {
        self.monitors
            .get(self.selected_monitor)
            .map(|monitor| monitor.name.as_str())
    }

    /// Returns selected monitor geometry.
    #[must_use]
    pub fn selected_monitor_geometry(&self) -> Option<(u32, u32)> {
        self.monitors
            .get(self.selected_monitor)
            .map(|monitor| (monitor.width, monitor.height))
    }

    /// Computes aspect simulation from selected wallpaper and monitor.
    #[must_use]
    pub fn aspect_simulation(&self) -> Option<AspectSimulation> {
        let (image_width, image_height) = self.selected_wallpaper()?.dimensions?;
        let (monitor_width, monitor_height) = self.selected_monitor_geometry()?;

        if image_width == 0 || image_height == 0 || monitor_width == 0 || monitor_height == 0 {
            return None;
        }

        let src_ratio = image_width as f64 / image_height as f64;
        let dst_ratio = monitor_width as f64 / monitor_height as f64;

        let (draw_width, draw_height) = match self.aspect_mode {
            AspectMode::Fit => {
                if src_ratio > dst_ratio {
                    (
                        monitor_width,
                        ((monitor_width as f64 / src_ratio).round() as u32).max(1),
                    )
                } else {
                    (
                        ((monitor_height as f64 * src_ratio).round() as u32).max(1),
                        monitor_height,
                    )
                }
            }
            AspectMode::Fill | AspectMode::Crop => {
                if src_ratio > dst_ratio {
                    (
                        ((monitor_height as f64 * src_ratio).round() as u32).max(1),
                        monitor_height,
                    )
                } else {
                    (
                        monitor_width,
                        ((monitor_width as f64 / src_ratio).round() as u32).max(1),
                    )
                }
            }
        };

        let crop_x = draw_width.saturating_sub(monitor_width) / 2;
        let crop_y = draw_height.saturating_sub(monitor_height) / 2;

        Some(AspectSimulation {
            image_width,
            image_height,
            monitor_width,
            monitor_height,
            draw_width,
            draw_height,
            crop_x,
            crop_y,
        })
    }

    /// Returns a snapshot control message for playlist worker configuration.
    #[must_use]
    pub fn playlist_control(&self) -> PlaylistControl {
        PlaylistControl {
            enabled: self.playlist_enabled,
            interval: Duration::from_secs(self.playlist_interval_secs()),
        }
    }

    /// Returns true when there is an active playlist with entries.
    #[must_use]
    pub fn has_active_playlist_entries(&self) -> bool {
        self.active_playlist
            .and_then(|idx| self.stored_state.playlists.get(idx))
            .map(|pl| !pl.entries.is_empty())
            .unwrap_or(false)
    }

    /// Toggles playlist enabled state when there is a valid active playlist.
    pub fn toggle_playlist(&mut self) {
        if !self.has_active_playlist_entries() {
            self.playlist_enabled = false;
            self.set_status("no playlist entries available");
            return;
        }

        self.playlist_enabled = !self.playlist_enabled;
        if self.playlist_enabled {
            self.set_status("playlist auto-cycle enabled");
        } else {
            self.set_status("playlist auto-cycle disabled");
        }
    }

    /// Returns next playlist wallpaper path and advances internal cursor.
    #[must_use]
    pub fn next_playlist_path(&mut self) -> Option<PathBuf> {
        let playlist = self
            .active_playlist
            .and_then(|idx| self.stored_state.playlists.get(idx))?;

        if playlist.entries.is_empty() {
            return None;
        }

        let index = self.playlist_cursor % playlist.entries.len();
        self.playlist_cursor = self.playlist_cursor.wrapping_add(1);
        Some(PathBuf::from(&playlist.entries[index]))
    }

    /// Saves the currently selected state into a profile named `quick-save`.
    pub fn save_quick_profile(&mut self) {
        let Some(path) = self.selected_wallpaper_path() else {
            self.set_status("cannot save profile: no wallpaper selected");
            return;
        };

        let profile = StoredProfile {
            name: String::from("quick-save"),
            wallpaper_path: path.to_string_lossy().into_owned(),
            monitor_name: self.selected_monitor_name().map(str::to_owned),
            transition_kind: self.transition_kind,
            transition_step: self.transition_step,
            transition_fps: self.transition_fps,
            simulator_mode: self.aspect_mode.label().to_owned(),
        };

        upsert_profile(&mut self.stored_state.profiles, profile);
        self.set_status("saved quick profile");

        self.ensure_default_playlist();
    }

    /// Loads profile named `quick-save` and applies it to runtime selection.
    pub fn load_quick_profile(&mut self) {
        let Some(profile) = self
            .stored_state
            .profiles
            .iter()
            .find(|profile| profile.name == "quick-save")
            .cloned()
        else {
            self.set_status("no quick profile found");
            return;
        };

        self.transition_kind = profile.transition_kind;
        self.transition_step = profile.transition_step.max(1);
        self.transition_fps = profile.transition_fps.max(1);
        self.aspect_mode = AspectMode::from_label(&profile.simulator_mode);

        if let Some(target_monitor) = profile.monitor_name {
            if let Some(index) = self
                .monitors
                .iter()
                .position(|monitor| monitor.name == target_monitor)
            {
                self.selected_monitor = index;
            }
        }

        self.select_wallpaper_by_path(PathBuf::from(profile.wallpaper_path));
        self.set_status("loaded quick profile");
    }

    /// Selects one wallpaper by absolute path if it exists in the current index.
    pub fn select_wallpaper_by_path(&mut self, path: PathBuf) {
        if let Some(index) = self.wallpapers.iter().position(|item| item.path == path) {
            if let Some(row) = self
                .filtered_wallpaper_indices
                .iter()
                .position(|entry| *entry == index)
            {
                self.selected_wallpaper_row = row;
            } else {
                self.search_query.clear();
                self.rebuild_filter();
                if let Some(row) = self
                    .filtered_wallpaper_indices
                    .iter()
                    .position(|entry| *entry == index)
                {
                    self.selected_wallpaper_row = row;
                }
            }
        }
    }

    /// Handles search input mode and returns true when key was consumed.
    fn handle_search_input(&mut self, key: KeyEvent, actions: &mut Vec<AppAction>) -> bool {
        if self.focus != FocusPane::Browser || !self.search_mode {
            return false;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.search_mode = false;
                self.set_status("search mode closed");
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.rebuild_filter();
                if self.selected_wallpaper().is_some() {
                    actions.push(AppAction::TryOnSelected);
                }
            }
            KeyCode::Char(ch)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.search_query.push(ch);
                self.rebuild_filter();
                if self.selected_wallpaper().is_some() {
                    actions.push(AppAction::TryOnSelected);
                }
            }
            _ => {}
        }

        true
    }

    /// Handles forward navigation for the currently focused pane.
    fn navigate_next(&mut self, actions: &mut Vec<AppAction>) {
        match self.focus {
            FocusPane::Browser => {
                self.selected_wallpaper_row = next_index(
                    self.selected_wallpaper_row,
                    self.filtered_wallpaper_indices.len(),
                );
                if self.selected_wallpaper().is_some() {
                    actions.push(AppAction::TryOnSelected);
                }
            }
            FocusPane::Monitor => {
                self.selected_monitor = next_index(self.selected_monitor, self.monitors.len());
                if self.selected_wallpaper().is_some() {
                    actions.push(AppAction::TryOnSelected);
                }
            }
            FocusPane::Transition => {
                self.transition_field = self.transition_field.next();
            }
        }
    }

    /// Handles reverse navigation for the currently focused pane.
    fn navigate_previous(&mut self, actions: &mut Vec<AppAction>) {
        match self.focus {
            FocusPane::Browser => {
                self.selected_wallpaper_row = prev_index(
                    self.selected_wallpaper_row,
                    self.filtered_wallpaper_indices.len(),
                );
                if self.selected_wallpaper().is_some() {
                    actions.push(AppAction::TryOnSelected);
                }
            }
            FocusPane::Monitor => {
                self.selected_monitor = prev_index(self.selected_monitor, self.monitors.len());
                if self.selected_wallpaper().is_some() {
                    actions.push(AppAction::TryOnSelected);
                }
            }
            FocusPane::Transition => {
                self.transition_field = self.transition_field.prev();
            }
        }
    }

    /// Decreases transition parameters when transition pane is focused.
    fn adjust_left(&mut self, actions: &mut Vec<AppAction>) {
        if self.focus != FocusPane::Transition {
            return;
        }

        match self.transition_field {
            TransitionField::Kind => {
                self.transition_kind = prev_transition_kind(self.transition_kind);
            }
            TransitionField::Step => {
                self.transition_step = self.transition_step.saturating_sub(1).max(1);
            }
            TransitionField::Fps => {
                self.transition_fps = self.transition_fps.saturating_sub(1).max(1);
            }
        }

        if self.preview_active {
            actions.push(AppAction::TryOnSelected);
        }
    }

    /// Increases transition parameters when transition pane is focused.
    fn adjust_right(&mut self, actions: &mut Vec<AppAction>) {
        if self.focus != FocusPane::Transition {
            return;
        }

        match self.transition_field {
            TransitionField::Kind => {
                self.transition_kind = next_transition_kind(self.transition_kind);
            }
            TransitionField::Step => {
                self.transition_step = self.transition_step.saturating_add(1).min(64);
            }
            TransitionField::Fps => {
                self.transition_fps = self.transition_fps.saturating_add(1).min(240);
            }
        }

        if self.preview_active {
            actions.push(AppAction::TryOnSelected);
        }
    }

    /// Recomputes fuzzy filtering and clamps selected row.
    fn rebuild_filter(&mut self) {
        self.filtered_wallpaper_indices =
            fuzzy_filter_indices(&self.wallpapers, &self.search_query);
        self.selected_wallpaper_row = clamp_index(
            self.selected_wallpaper_row,
            self.filtered_wallpaper_indices.len(),
        );
    }

    /// Initializes playlist defaults from loaded persisted state.
    fn initialize_playlist_state(&mut self) {
        self.ensure_default_playlist();

        if self.stored_state.playlists.is_empty() {
            self.active_playlist = None;
            self.playlist_enabled = false;
            return;
        }

        self.active_playlist = Some(0);
        self.playlist_enabled = false;
        self.playlist_cursor = 0;
    }

    /// Ensures there is at least one playlist by creating a generated fallback.
    fn ensure_default_playlist(&mut self) {
        if !self.stored_state.playlists.is_empty() {
            return;
        }

        let entries = self
            .wallpapers
            .iter()
            .take(32)
            .map(|item| item.path.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        if entries.is_empty() {
            return;
        }

        self.stored_state.playlists.push(StoredPlaylist {
            name: String::from("default"),
            entries,
            interval_secs: 30,
        });
    }

    /// Returns playlist interval in seconds with sane lower bound.
    fn playlist_interval_secs(&self) -> u64 {
        self.active_playlist
            .and_then(|index| self.stored_state.playlists.get(index))
            .map(|playlist| playlist.interval_secs.max(1))
            .unwrap_or(30)
    }
}

/// Upserts a profile by name.
fn upsert_profile(profiles: &mut Vec<StoredProfile>, profile: StoredProfile) {
    if let Some(index) = profiles
        .iter()
        .position(|existing| existing.name == profile.name)
    {
        profiles[index] = profile;
    } else {
        profiles.push(profile);
    }
}

/// Computes the next cyclic index in a non-empty collection.
#[must_use]
fn next_index(current: usize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    (current + 1) % len
}

/// Computes the previous cyclic index in a non-empty collection.
#[must_use]
fn prev_index(current: usize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    if current == 0 {
        len - 1
    } else {
        current - 1
    }
}

/// Clamps an index to a collection length, returning 0 when empty.
#[must_use]
fn clamp_index(current: usize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    current.min(len - 1)
}

/// Cycles forward through transition kinds.
#[must_use]
fn next_transition_kind(kind: TransitionKind) -> TransitionKind {
    match kind {
        TransitionKind::Fade => TransitionKind::Wipe,
        TransitionKind::Wipe => TransitionKind::Grow,
        TransitionKind::Grow => TransitionKind::Fade,
    }
}

/// Cycles backward through transition kinds.
#[must_use]
fn prev_transition_kind(kind: TransitionKind) -> TransitionKind {
    match kind {
        TransitionKind::Fade => TransitionKind::Grow,
        TransitionKind::Wipe => TransitionKind::Fade,
        TransitionKind::Grow => TransitionKind::Wipe,
    }
}
