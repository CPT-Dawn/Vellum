use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::state::App;

pub(crate) fn handle_key_event(app: &mut App, key: KeyEvent) -> bool {
    if key.kind != KeyEventKind::Press {
        return false;
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return true,
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('l') => app.select_next(),
        KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('h') => app.select_previous(),
        KeyCode::Home => app.select_first(),
        KeyCode::End | KeyCode::Char('G') => app.select_last(),
        KeyCode::PageDown => app.select_page_down(10),
        KeyCode::PageUp => app.select_page_up(10),
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.select_page_down(10)
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.select_page_up(10)
        }
        KeyCode::Enter | KeyCode::Char(' ') => app.apply_selected_wallpaper(),
        KeyCode::Char('t') => app.cycle_monitor_target(),
        KeyCode::Char('s') => app.cycle_scale_mode(),
        KeyCode::Char('m') => app.fetch_monitors(),
        KeyCode::Char('a') => app.fetch_assignments(),
        KeyCode::Char('x') => app.clear_assignments(),
        KeyCode::Char('r') => app.reload_files(),
        KeyCode::Char('?') => app.toggle_help(),
        KeyCode::Char('g') => {
            if app.pending_g {
                app.select_first();
                app.pending_g = false;
            } else {
                app.pending_g = true;
                app.status = "pending motion: g (press g again for top)".to_string();
            }
        }
        _ => {}
    }

    if !matches!(key.code, KeyCode::Char('g')) {
        app.pending_g = false;
    }

    false
}
