mod app;
mod ui;

use std::{io, time::Duration};

use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::app::App;

/// Application-wide result type.
type AppResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Program entrypoint.
#[tokio::main]
async fn main() -> AppResult<()> {
    let mut terminal = init_terminal()?;
    let result = run_app(&mut terminal, App::default()).await;
    restore_terminal(&mut terminal)?;
    result
}

/// Enables raw terminal mode and configures the alternate screen.
fn init_terminal() -> AppResult<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Restores terminal state so the shell works correctly after exit.
fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> AppResult<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Runs the main event/render loop until the user requests exit.
async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mut app: App,
) -> AppResult<()> {
    const TICK_RATE: Duration = Duration::from_millis(120);

    loop {
        terminal.draw(|frame| ui::render(frame, &app))?;

        if event::poll(TICK_RATE)? {
            if let Event::Key(key) = event::read()? {
                app.on_key(key);
            }
        } else {
            app.on_tick();
        }

        if app.should_quit {
            break;
        }

        tokio::task::yield_now().await;
    }

    Ok(())
}
