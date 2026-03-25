mod app;
mod events;
mod ui;

use std::io;
use std::time::Duration;

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use ratatui::DefaultTerminal;

use crate::app::App;
use crate::events::{spawn_event_thread, AppEvent};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut terminal = ratatui::init();
    execute!(io::stdout(), EnableMouseCapture)?;

    let mut app = App::load_or_default();
    let receiver = spawn_event_thread(Duration::from_millis(120));

    let run_result = run(&mut terminal, &mut app, receiver);

    execute!(io::stdout(), DisableMouseCapture)?;
    ratatui::restore();

    if let Err(error) = app.save_config() {
        eprintln!("failed to save config: {error}");
    }

    run_result.map_err(Into::into)
}

fn run(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    receiver: std::sync::mpsc::Receiver<AppEvent>,
) -> Result<(), io::Error> {
    terminal.draw(|frame| ui::draw(frame, app))?;

    for event in receiver {
        let should_quit = match event {
            AppEvent::Input(input) => app.handle_event(input),
            AppEvent::Tick => false,
        };

        terminal.draw(|frame| ui::draw(frame, app))?;

        if should_quit {
            break;
        }
    }

    Ok(())
}
