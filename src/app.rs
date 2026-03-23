use crossterm::event::{KeyCode, KeyEvent};

use crate::backend::awww::TransitionKind;

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

/// Dummy monitor entry used for PHASE 3 layout rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DummyMonitor {
    /// Display connector name.
    pub name: &'static str,
    /// Top-left X coordinate.
    pub x: i32,
    /// Top-left Y coordinate.
    pub y: i32,
    /// Monitor width in pixels.
    pub width: u32,
    /// Monitor height in pixels.
    pub height: u32,
}

const DUMMY_WALLPAPERS: &[&str] = &[
    "aurora-icefield.jpg",
    "neon-rain-alley.png",
    "sunset-over-grid.webp",
    "kinetic-particles.jpeg",
    "quiet-forest-dawn.avif",
    "cityline-midnight.png",
    "paperfold-minimal.jpg",
];

const DUMMY_MONITORS: &[DummyMonitor] = &[
    DummyMonitor {
        name: "eDP-1",
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    },
    DummyMonitor {
        name: "DP-1",
        x: 1920,
        y: 0,
        width: 2560,
        height: 1440,
    },
    DummyMonitor {
        name: "HDMI-A-1",
        x: -1080,
        y: 100,
        width: 1080,
        height: 1920,
    },
];

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

/// Global application state shared by the event loop and renderer.
#[derive(Debug, Clone)]
pub struct App {
    /// Whether the application should terminate on the next loop iteration.
    pub should_quit: bool,
    /// The pane currently selected by keyboard navigation.
    pub focus: FocusPane,
    /// Monotonic tick counter used for future animations and periodic tasks.
    pub ticks: u64,
    /// Dummy wallpaper list shown by the PHASE 3 file browser pane.
    pub wallpapers: &'static [&'static str],
    /// Selected wallpaper row index.
    pub selected_wallpaper: usize,
    /// Dummy monitor layout entries for PHASE 3 monitor pane.
    pub monitors: &'static [DummyMonitor],
    /// Selected monitor row index.
    pub selected_monitor: usize,
    /// Transition row currently selected in the transition pane.
    pub transition_field: TransitionField,
    /// Transition kind preview value.
    pub transition_kind: TransitionKind,
    /// Transition step preview value.
    pub transition_step: u16,
    /// Transition FPS preview value.
    pub transition_fps: u16,
}

impl Default for App {
    /// Builds the default initial state for application startup.
    fn default() -> Self {
        Self {
            should_quit: false,
            focus: FocusPane::Browser,
            ticks: 0,
            wallpapers: DUMMY_WALLPAPERS,
            selected_wallpaper: 0,
            monitors: DUMMY_MONITORS,
            selected_monitor: 0,
            transition_field: TransitionField::Kind,
            transition_kind: TransitionKind::Fade,
            transition_step: 16,
            transition_fps: 60,
        }
    }
}

impl App {
    /// Handles a terminal key event and mutates state accordingly.
    pub fn on_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('h') | KeyCode::Left => self.focus = self.focus.move_left(),
            KeyCode::Char('l') | KeyCode::Right => self.focus = self.focus.move_right(),
            KeyCode::Char('j') | KeyCode::Down => self.navigate_next(),
            KeyCode::Char('k') | KeyCode::Up => self.navigate_previous(),
            KeyCode::Char('H') => self.adjust_left(),
            KeyCode::Char('L') => self.adjust_right(),
            _ => {}
        }
    }

    /// Advances periodic state once per tick interval.
    pub fn on_tick(&mut self) {
        self.ticks = self.ticks.saturating_add(1);
    }

    /// Returns the currently selected dummy wallpaper name.
    #[must_use]
    pub fn selected_wallpaper_name(&self) -> &'static str {
        self.wallpapers[self.selected_wallpaper]
    }

    /// Returns the currently selected dummy monitor.
    #[must_use]
    pub fn selected_monitor(&self) -> DummyMonitor {
        self.monitors[self.selected_monitor]
    }

    /// Handles row navigation for the pane currently in focus.
    fn navigate_next(&mut self) {
        match self.focus {
            FocusPane::Browser => {
                self.selected_wallpaper =
                    next_index(self.selected_wallpaper, self.wallpapers.len());
            }
            FocusPane::Monitor => {
                self.selected_monitor = next_index(self.selected_monitor, self.monitors.len());
            }
            FocusPane::Transition => {
                self.transition_field = self.transition_field.next();
            }
        }
    }

    /// Handles reverse row navigation for the pane currently in focus.
    fn navigate_previous(&mut self) {
        match self.focus {
            FocusPane::Browser => {
                self.selected_wallpaper =
                    prev_index(self.selected_wallpaper, self.wallpapers.len());
            }
            FocusPane::Monitor => {
                self.selected_monitor = prev_index(self.selected_monitor, self.monitors.len());
            }
            FocusPane::Transition => {
                self.transition_field = self.transition_field.prev();
            }
        }
    }

    /// Decreases transition parameters while transition pane is focused.
    fn adjust_left(&mut self) {
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
    }

    /// Increases transition parameters while transition pane is focused.
    fn adjust_right(&mut self) {
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
