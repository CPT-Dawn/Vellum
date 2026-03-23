//! Ratatui rendering layer for the awww-tui application.

use ratatui::{
    layout::{Constraint, Direction, Layout},
    prelude::{Alignment, Color, Frame, Line, Modifier, Style},
    symbols::border,
    text::Span,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use crate::{
    app::{App, FocusPane, TransitionField},
    backend::awww::TransitionKind,
};

/// Renders the full terminal frame for the current application state.
pub fn render(frame: &mut Frame<'_>, app: &App) {
    let theme = Theme::from_tick(app.ticks);

    let frame_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(frame.area());

    render_header(frame, frame_chunks[0], app, theme);

    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(38),
            Constraint::Percentage(32),
            Constraint::Percentage(30),
        ])
        .split(frame_chunks[1]);

    render_browser_pane(frame, body_chunks[0], app, theme);
    render_monitor_pane(frame, body_chunks[1], app, theme);
    render_transition_pane(frame, body_chunks[2], app, theme);
    render_footer(frame, frame_chunks[2], app, theme);
}

/// Renders top status information and key bindings.
fn render_header(frame: &mut Frame<'_>, area: ratatui::layout::Rect, app: &App, theme: Theme) {
    let selected = app
        .selected_wallpaper()
        .map(|item| item.name.as_str())
        .unwrap_or("<none>");

    let status = format!(
        "phase 4 live integration | focus: {:?} | selected: {}",
        app.focus, selected
    );

    let keys = "h/l pane  j/k row  / search  Enter confirm  c cancel  q quit";

    let text = Line::from(vec![
        Span::styled(status, Style::default().fg(theme.header_fg)),
        Span::raw("  |  "),
        Span::styled(keys, Style::default().fg(theme.muted)),
    ]);

    let header = Paragraph::new(text)
        .alignment(Alignment::Center)
        .style(Style::default().bg(theme.background));
    frame.render_widget(header, area);
}

/// Renders a footer status line.
fn render_footer(frame: &mut Frame<'_>, area: ratatui::layout::Rect, app: &App, theme: Theme) {
    let search = if app.search_mode {
        format!("search: {}_", app.search_query)
    } else if app.search_query.is_empty() {
        String::from("search: <off>")
    } else {
        format!("search: {}", app.search_query)
    };

    let footer = Paragraph::new(Line::from(vec![
        Span::styled(search, Style::default().fg(theme.accent)),
        Span::raw("  |  "),
        Span::styled(app.status.as_str(), Style::default().fg(theme.text)),
    ]))
    .alignment(Alignment::Left)
    .style(Style::default().bg(theme.background));

    frame.render_widget(footer, area);
}

/// Renders the wallpaper browser pane with fuzzy-filtered filesystem data.
fn render_browser_pane(
    frame: &mut Frame<'_>,
    area: ratatui::layout::Rect,
    app: &App,
    theme: Theme,
) {
    let is_active = app.focus == FocusPane::Browser;
    let border_color = if is_active { theme.focus } else { theme.idle };

    let subtitle = if app.search_mode {
        "search mode active"
    } else {
        "filesystem + fuzzy finder"
    };

    let block = pane_block("Wallpapers", subtitle, border_color, theme);

    let items = app
        .filtered_wallpaper_indices
        .iter()
        .map(|index| {
            let entry = &app.wallpapers[*index];
            ListItem::new(Line::from(entry.name.clone()))
        })
        .collect::<Vec<_>>();

    let selected = if app.filtered_wallpaper_indices.is_empty() {
        None
    } else {
        Some(app.selected_wallpaper_row)
    };

    let mut state = ListState::default().with_selected(selected);
    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .fg(theme.background)
                .bg(theme.focus)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("-> ");

    frame.render_stateful_widget(list, area, &mut state);
}

/// Renders monitor metadata discovered from compositor IPC.
fn render_monitor_pane(
    frame: &mut Frame<'_>,
    area: ratatui::layout::Rect,
    app: &App,
    theme: Theme,
) {
    let is_active = app.focus == FocusPane::Monitor;
    let border_color = if is_active { theme.focus } else { theme.idle };

    let mut lines = Vec::with_capacity(app.monitors.len() + 5);

    if app.monitors.is_empty() {
        lines.push(Line::from("no monitors discovered"));
        lines.push(Line::from("(hyprctl/wlr-randr unavailable)"));
    } else {
        let selected = &app.monitors[app.selected_monitor];
        lines.push(Line::from(format!(
            "selected: {} ({}x{})",
            selected.name, selected.width, selected.height
        )));
        lines.push(Line::from("layout:"));

        for (idx, monitor) in app.monitors.iter().enumerate() {
            let prefix = if idx == app.selected_monitor {
                "*"
            } else {
                " "
            };
            lines.push(Line::from(format!(
                "{prefix} {:<10} {:>4}x{:<4} @ ({:>5},{:>5})",
                monitor.name, monitor.width, monitor.height, monitor.x, monitor.y
            )));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(monitor_bar(
            selected.width,
            selected.height,
            24,
            theme.accent,
        )));
    }

    let body = Paragraph::new(lines)
        .block(pane_block(
            "Monitors",
            "j/k to choose output target",
            border_color,
            theme,
        ))
        .style(Style::default().fg(theme.text));

    frame.render_widget(body, area);
}

/// Renders transition settings pane with editable values.
fn render_transition_pane(
    frame: &mut Frame<'_>,
    area: ratatui::layout::Rect,
    app: &App,
    theme: Theme,
) {
    let is_active = app.focus == FocusPane::Transition;
    let border_color = if is_active { theme.focus } else { theme.idle };

    let rows = vec![
        transition_row(
            "type",
            transition_kind_label(app.transition_kind),
            app.transition_field == TransitionField::Kind,
            theme,
        ),
        transition_row(
            "step",
            &app.transition_step.to_string(),
            app.transition_field == TransitionField::Step,
            theme,
        ),
        transition_row(
            "fps",
            &app.transition_fps.to_string(),
            app.transition_field == TransitionField::Fps,
            theme,
        ),
        Line::from(""),
        Line::from("Shift+H / Shift+L edits selected field"),
        Line::from("changes reapply live while preview is active"),
    ];

    let body = Paragraph::new(rows)
        .block(pane_block(
            "Transitions",
            "phase 4 live controls",
            border_color,
            theme,
        ))
        .style(Style::default().fg(theme.text));

    frame.render_widget(body, area);
}

/// Creates a pane block with consistent style and title placement.
fn pane_block(
    title: &'static str,
    subtitle: &'static str,
    border_color: Color,
    theme: Theme,
) -> Block<'static> {
    Block::default()
        .title_top(Line::from(title).style(Style::default().add_modifier(Modifier::BOLD)))
        .title_bottom(Line::from(subtitle).style(Style::default().fg(theme.muted)))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme.background))
}

/// Renders one transition row with selected-state emphasis.
fn transition_row(label: &str, value: &str, selected: bool, theme: Theme) -> Line<'static> {
    if selected {
        Line::from(vec![
            Span::styled("-> ", Style::default().fg(theme.focus)),
            Span::styled(
                format!("{label:<6}"),
                Style::default()
                    .fg(theme.focus)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(value.to_owned(), Style::default().fg(theme.accent)),
        ])
    } else {
        Line::from(format!("   {label:<6}{value}"))
    }
}

/// Builds a simple monitor ratio bar using ASCII fill characters.
fn monitor_bar(width: u32, height: u32, max_units: usize, accent: Color) -> Line<'static> {
    let dominant = width.max(height).max(1);
    let horizontal_units = ((width as usize) * max_units) / (dominant as usize);
    let vertical_units = ((height as usize) * max_units) / (dominant as usize);
    let horizontal = "#".repeat(horizontal_units.max(1));
    let vertical = "#".repeat(vertical_units.max(1));

    Line::from(vec![
        Span::styled(
            format!("w[{horizontal:<max_units$}] ", max_units = max_units),
            Style::default().fg(accent),
        ),
        Span::styled(
            format!("h[{vertical:<max_units$}]", max_units = max_units),
            Style::default().fg(accent),
        ),
    ])
}

/// Returns display text for transition kind values.
fn transition_kind_label(kind: TransitionKind) -> &'static str {
    match kind {
        TransitionKind::Fade => "fade",
        TransitionKind::Wipe => "wipe",
        TransitionKind::Grow => "grow",
    }
}

/// Shared color palette for current frame rendering.
#[derive(Debug, Clone, Copy)]
struct Theme {
    /// Global background color.
    background: Color,
    /// Primary pane text color.
    text: Color,
    /// Header emphasized foreground.
    header_fg: Color,
    /// Active pane border color.
    focus: Color,
    /// Secondary accent color.
    accent: Color,
    /// Inactive border and tertiary text color.
    muted: Color,
    /// Idle pane border color.
    idle: Color,
}

impl Theme {
    /// Computes a subtle animated theme from the tick counter.
    #[must_use]
    fn from_tick(tick: u64) -> Self {
        if (tick / 20).is_multiple_of(2) {
            Self {
                background: Color::Rgb(10, 18, 24),
                text: Color::Rgb(232, 236, 240),
                header_fg: Color::Rgb(126, 224, 255),
                focus: Color::Rgb(84, 182, 255),
                accent: Color::Rgb(96, 255, 180),
                muted: Color::Rgb(124, 137, 150),
                idle: Color::Rgb(64, 84, 96),
            }
        } else {
            Self {
                background: Color::Rgb(18, 17, 24),
                text: Color::Rgb(236, 236, 244),
                header_fg: Color::Rgb(255, 216, 102),
                focus: Color::Rgb(252, 152, 103),
                accent: Color::Rgb(166, 210, 120),
                muted: Color::Rgb(142, 132, 153),
                idle: Color::Rgb(90, 74, 105),
            }
        }
    }
}
