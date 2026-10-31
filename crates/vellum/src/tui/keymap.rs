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
            KeyCode::Enter => Some(Action::Apply),
            KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
            _ => None,
        },
    }
}

/// The footer's key hints in `mode`: (keys, what they do).
pub fn hints(mode: Mode) -> &'static [(&'static str, &'static str)] {
    match mode {
        Mode::Normal => &[("↑↓/jk", "move"), ("⏎", "apply"), ("q", "quit")],
    }
}
