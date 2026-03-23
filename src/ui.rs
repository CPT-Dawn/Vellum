use ratatui::{
    layout::{Constraint, Direction, Layout},
    prelude::{Color, Frame, Line, Modifier, Style},
    widgets::{block::BorderType, Block, Borders, Paragraph},
};

use crate::app::{App, FocusPane};

/// Renders the full terminal frame for the current application state.
pub fn render(frame: &mut Frame<'_>, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(38),
            Constraint::Percentage(32),
            Constraint::Percentage(30),
        ])
        .split(frame.area());

    render_pane(
        frame,
        chunks[0],
        "Wallpapers",
        app,
        FocusPane::Browser,
        "fuzzy finder ready in phase 4",
    );
    render_pane(
        frame,
        chunks[1],
        "Monitors",
        app,
        FocusPane::Monitor,
        "monitor graph ready in phase 2",
    );
    render_pane(
        frame,
        chunks[2],
        "Transitions",
        app,
        FocusPane::Transition,
        "controls ready in phase 4",
    );
}

/// Renders a single pane with dynamic styling based on current focus.
fn render_pane(
    frame: &mut Frame<'_>,
    area: ratatui::layout::Rect,
    title: &'static str,
    app: &App,
    pane: FocusPane,
    subtitle: &'static str,
) {
    let is_active = app.focus == pane;
    let border_color = if is_active {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .title(Line::from(title).style(Style::default().add_modifier(Modifier::BOLD)))
        .title_bottom(Line::from(subtitle).style(Style::default().fg(Color::Gray)))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color));

    let body = Paragraph::new(
        Line::from(format!("ticks: {}", app.ticks)).style(Style::default().fg(Color::White)),
    )
    .block(block)
    .style(Style::default().bg(Color::Black));

    frame.render_widget(body, area);
}
