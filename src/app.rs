//! Application state and input handling for `awww-tui`.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{
    backend::{awww::TransitionKind, monitors::MonitorInfo},
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

/// Side-effect request emitted by key handling for runtime execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAction {
    /// Apply selected wallpaper as a temporary live preview.
    TryOnSelected,
    /// Confirm currently selected wallpaper as committed choice.
    ConfirmSelected,
    /// Revert temporary preview to the last confirmed wallpaper.
    CancelPreview,
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
    /// Status line rendered in header.
    pub status: String,
    /// Last confirmed wallpaper path for cancel/revert behavior.
    pub confirmed_wallpaper: Option<PathBuf>,
    /// Whether a temporary preview is currently active.
    pub preview_active: bool,
}

impl App {
    /// Builds initial app state from discovered wallpapers and monitor metadata.
    #[must_use]
    pub fn new(wallpapers: Vec<WallpaperItem>, monitors: Vec<MonitorInfo>) -> Self {
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
        };

        app.rebuild_filter();
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
