//! Keys to Actions, per Mode, and the footer hints.
//!
//! #11 turns this into the Action registry (remappable from Config, also
//! driving the `?` overlay and the palette). Until then it is a fixed
//! table, but update already goes key -> [`Action`] -> effect, so new keys
//! only need an Action variant, a row here, and an arm in
//! `update::perform`.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::model::Mode;

/// Something the user can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Down,
    Up,
    /// The tile to the left, in the grid.
    Left,
    /// The tile to the right, in the grid.
    Right,
    /// Switch between the list and the thumbnail grid.
    ToggleGrid,
    /// Apply the selection to the targeted Monitor.
    Apply,
    Quit,
}

/// The Action `key` means in `mode`, if any.
pub fn action(mode: Mode, key: KeyEvent) -> Option<Action> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match mode {
        Mode::Normal => match key.code {
            KeyCode::Char('c') if ctrl => Some(Action::Quit),
            _ if ctrl => None,
            KeyCode::Char('j') | KeyCode::Down => Some(Action::Down),
            KeyCode::Char('k') | KeyCode::Up => Some(Action::Up),
            KeyCode::Char('h') | KeyCode::Left => Some(Action::Left),
            KeyCode::Char('l') | KeyCode::Right => Some(Action::Right),
            KeyCode::Char('g') => Some(Action::ToggleGrid),
            KeyCode::Enter => Some(Action::Apply),
            KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
            _ => None,
        },
    }
}

/// The footer's key hints in `mode`, in the grid or the list: (keys, what
/// they do).
pub fn hints(mode: Mode, grid: bool) -> &'static [(&'static str, &'static str)] {
    match (mode, grid) {
        (Mode::Normal, false) => &[
            ("↑↓/jk", "move"),
            ("g", "grid"),
            ("⏎", "apply"),
            ("q", "quit"),
        ],
        (Mode::Normal, true) => &[
            ("←↓↑→/hjkl", "move"),
            ("g", "list"),
            ("⏎", "apply"),
            ("q", "quit"),
        ],
    }
}
