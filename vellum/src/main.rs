mod app;
mod backend;
mod events;
mod imgproc;
mod playlist_worker;
mod preview;
mod ui;

use std::env;
use std::io;
use std::time::Duration;

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use ratatui::DefaultTerminal;

use crate::app::App;
use crate::backend::Backend;
use crate::events::{AppEvent, spawn_event_thread};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    if matches!(args.next().as_deref(), Some("--playlist-worker")) {
        let namespace = args.next().unwrap_or_default();
        return playlist_worker::run(namespace);
    }

    let _ = playlist_worker::mark_tui_active();

    let mut terminal = ratatui::init();
    execute!(io::stdout(), EnableMouseCapture)?;

    let mut backend = Backend::new("");
    let mut app = App::load_or_default();
    app.sync_from_backend(&mut backend);
    let receiver = spawn_event_thread(Duration::from_millis(120));

    let run_result = run(&mut terminal, &mut app, &mut backend, receiver);

    execute!(io::stdout(), DisableMouseCapture)?;
    ratatui::restore();

    playlist_worker::clear_tui_active_marker();

    if app.has_running_playlists() {
        let _ = playlist_worker::spawn_background_worker("");
    }

    run_result.map_err(Into::into)
}

fn run(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    backend: &mut Backend,
    receiver: std::sync::mpsc::Receiver<AppEvent>,
) -> Result<(), io::Error> {
    terminal.draw(|frame| ui::draw(frame, app))?;

    for event in receiver {
        let should_quit = match event {
            AppEvent::Input(input) => app.handle_event(input, backend),
            AppEvent::Tick => {
                app.handle_tick(backend);
                false
            }
        };

        terminal.draw(|frame| ui::draw(frame, app))?;

        if should_quit {
            break;
        }
    }

    Ok(())
}
