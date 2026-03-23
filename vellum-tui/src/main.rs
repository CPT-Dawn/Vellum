use std::{
    io,
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};

/// UI refresh cadence in milliseconds.
const TICK_RATE_MS: u64 = 33;

/// Application-level state for the initial Vellum TUI shell.
#[derive(Debug, Clone)]
struct AppState {
    /// Whether the event loop should exit.
    should_quit: bool,
    /// Status message rendered in the footer.
    status: String,
    /// Number of frames rendered in this session.
    frame_count: u64,
    /// Timestamp of the last frame.
    last_frame: Instant,
}

impl Default for AppState {
    /// Builds the default UI state used at startup.
    fn default() -> Self {
        Self {
            should_quit: false,
            status: String::from("Ready - Phase 1 shell running"),
            frame_count: 0,
            last_frame: Instant::now(),
        }
    }
}

/// Terminal setup guard that restores screen state on drop.
struct TerminalGuard;

impl Drop for TerminalGuard {
    /// Restores terminal raw mode and alternate screen settings.
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, LeaveAlternateScreen);
    }
}

/// Launches the async runtime and starts the TUI loop.
#[tokio::main(flavor = "multi_thread")]
async fn main() -> io::Result<()> {
    run_app().await
}

/// Initializes terminal mode and executes the draw/input loop.
async fn run_app() -> io::Result<()> {
    enable_raw_mode()?;
    let _guard = TerminalGuard;

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut state = AppState::default();
    let mut tick = tokio::time::interval(Duration::from_millis(TICK_RATE_MS));

    while !state.should_quit {
        tick.tick().await;
        handle_input(&mut state)?;
        terminal.draw(|frame| draw_ui(frame, &state))?;
        state.frame_count = state.frame_count.saturating_add(1);
        state.last_frame = Instant::now();
    }

    Ok(())
}

/// Reads keyboard events and mutates app state.
fn handle_input(state: &mut AppState) -> io::Result<()> {
    while event::poll(Duration::from_millis(0))? {
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    state.status = String::from("Exiting Vellum");
                    state.should_quit = true;
                }
                KeyCode::Char('r') => {
                    state.status = String::from("Renderer sync placeholder (Phase 2)");
                }
                KeyCode::Char('h') => {
                    state.status = String::from("Help: q/Esc quit, r backend refresh");
                }
                _ => {}
            }
        }
    }

    Ok(())
}

/// Renders the initial multi-pane Ratatui layout.
fn draw_ui(frame: &mut Frame<'_>, state: &AppState) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(frame.area());

    draw_header(frame, root[0]);
    draw_body(frame, root[1]);
    draw_footer(frame, root[2], state);
}

/// Draws the top status header.
fn draw_header(frame: &mut Frame<'_>, area: Rect) {
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            "VELLUM",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  native wallpaper control surface"),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .title(" Session "),
    );
    frame.render_widget(title, area);
}

/// Draws the three core panes for the Phase 1 shell.
fn draw_body(frame: &mut Frame<'_>, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(area);

    let file_browser =
        Paragraph::new("File Browser\n- phase scaffold\n- no filesystem binding yet").block(
            Block::default()
                .title(" Browser ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        );

    let monitor_preview = Paragraph::new(
        "Monitor Preview\n- awaiting native backend wiring\n- renderer sync in Phase 2",
    )
    .block(
        Block::default()
            .title(" Monitor ")
            .borders(Borders::ALL)
            .border_type(BorderType::Thick),
    );

    let transition_panel = Paragraph::new("Transitions\n- duration\n- easing\n- fps cap").block(
        Block::default()
            .title(" Transitions ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded),
    );

    frame.render_widget(file_browser, columns[0]);
    frame.render_widget(monitor_preview, columns[1]);
    frame.render_widget(transition_panel, columns[2]);
}

/// Draws footer hints and runtime diagnostics.
fn draw_footer(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let diagnostics = format!(
        "{} | frames={} | q/Esc=quit h=help r=refresh",
        state.status, state.frame_count
    );
    let footer = Paragraph::new(diagnostics).block(
        Block::default()
            .title(" Controls ")
            .borders(Borders::ALL)
            .border_type(BorderType::Double),
    );
    frame.render_widget(footer, area);
}
