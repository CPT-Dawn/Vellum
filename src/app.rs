use crossterm::event::{KeyCode, KeyEvent};

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

/// Global application state shared by the event loop and renderer.
#[derive(Debug, Clone)]
pub struct App {
    /// Whether the application should terminate on the next loop iteration.
    pub should_quit: bool,
    /// The pane currently selected by keyboard navigation.
    pub focus: FocusPane,
    /// Monotonic tick counter used for future animations and periodic tasks.
    pub ticks: u64,
}

impl Default for App {
    /// Builds the default initial state for application startup.
    fn default() -> Self {
        Self {
            should_quit: false,
            focus: FocusPane::Browser,
            ticks: 0,
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
            _ => {}
        }
    }

    /// Advances periodic state once per tick interval.
    pub fn on_tick(&mut self) {
        self.ticks = self.ticks.saturating_add(1);
    }
}
