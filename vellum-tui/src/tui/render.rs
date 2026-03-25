use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Row, Table, Tabs, Wrap},
};

use super::{
    App, app_key_hints, daemon_status_color, daemon_status_text, global_key_hints,
    selected_preview_simulation,
};
use crate::tui::model::{
    FocusRegion, NotificationLevel, ScaleMode, TransitionState, monitor_layout_ascii,
};

const C_BG: Color = Color::Rgb(10, 14, 22);
const C_PANEL: Color = Color::Rgb(20, 26, 38);
const C_PANEL_ALT: Color = Color::Rgb(15, 20, 30);
const C_TEXT: Color = Color::Rgb(225, 231, 239);
const C_MUTED: Color = Color::Rgb(133, 149, 173);
const C_ACCENT: Color = Color::Rgb(74, 189, 255);
const C_ACCENT_2: Color = Color::Rgb(255, 179, 102);
const C_GREEN: Color = Color::Rgb(120, 210, 151);
const C_YELLOW: Color = Color::Rgb(245, 193, 96);
const C_RED: Color = Color::Rgb(242, 112, 122);

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    frame.render_widget(
        Block::default().style(Style::default().bg(C_BG)),
        frame.area(),
    );

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(frame.area());

    draw_header(frame, rows[0], app);
    draw_body(frame, rows[1], app);
    draw_footer(frame, rows[2], app);

    if app.help_open {
        draw_help_overlay(frame, app);
    }
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(area);

    let focus_tabs = Tabs::new(
        [
            FocusRegion::Library,
            FocusRegion::Preview,
            FocusRegion::Monitors,
            FocusRegion::Playlist,
            FocusRegion::Transitions,
        ]
        .into_iter()
        .map(|focus| {
            Line::from(Span::styled(
                format!(" {} ", focus.label()),
                Style::default(),
            ))
        })
        .collect::<Vec<_>>(),
    )
    .select(match app.focus {
        FocusRegion::Library => 0,
        FocusRegion::Preview => 1,
        FocusRegion::Monitors => 2,
        FocusRegion::Playlist => 3,
        FocusRegion::Transitions => 4,
    })
    .style(Style::default().fg(C_MUTED).bg(C_PANEL))
    .highlight_style(
        Style::default()
            .fg(C_BG)
            .bg(C_ACCENT)
            .add_modifier(Modifier::BOLD),
    )
    .divider(" ")
    .block(
        Block::default()
            .title(Line::from(vec![
                Span::styled(
                    " VELLUM ",
                    Style::default().fg(C_ACCENT_2).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" wallpaper control plane ", Style::default().fg(C_MUTED)),
            ]))
            .borders(Borders::ALL)
            .border_set(border::ROUNDED)
            .border_style(Style::default().fg(C_MUTED))
            .style(Style::default().bg(C_PANEL)),
    );

    frame.render_widget(focus_tabs, cols[0]);

    let status = Paragraph::new(Line::from(vec![
        Span::styled("Daemon ", Style::default().fg(C_MUTED)),
        Span::styled(
            daemon_status_text(app),
            Style::default()
                .fg(daemon_status_color(app))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | Focus ", Style::default().fg(C_MUTED)),
        Span::styled(
            app.focus.label(),
            Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | Targets ", Style::default().fg(C_MUTED)),
        Span::styled(
            app.selected_targets.len().to_string(),
            Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | Queue ", Style::default().fg(C_MUTED)),
        Span::styled(
            app.playlist.len().to_string(),
            Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
        ),
    ]))
    .block(
        Block::default()
            .title(Line::from(vec![
                Span::styled(" Status ", Style::default().fg(C_MUTED)),
                Span::styled(
                    if app.input_mode == super::model::InputMode::Search {
                        "searching"
                    } else {
                        "ready"
                    },
                    Style::default().fg(C_ACCENT_2),
                ),
            ]))
            .borders(Borders::ALL)
            .border_set(border::ROUNDED)
            .border_style(Style::default().fg(C_MUTED))
            .style(Style::default().bg(C_PANEL)),
    );

    frame.render_widget(status, cols[1]);
}

fn draw_body(frame: &mut Frame<'_>, area: Rect, app: &App) {
    if area.width < 120 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(40),
                Constraint::Percentage(35),
                Constraint::Percentage(25),
            ])
            .split(area);

        draw_library_stack(frame, chunks[0], app);
        draw_preview_panel(frame, chunks[1], app);
        draw_right_stack(frame, chunks[2], app);
        return;
    }

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(39),
            Constraint::Percentage(28),
        ])
        .split(area);

    draw_library_stack(frame, cols[0], app);
    draw_preview_panel(frame, cols[1], app);
    draw_right_stack(frame, cols[2], app);
}

fn draw_library_stack(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(64), Constraint::Percentage(36)])
        .split(area);

    draw_browser_panel(frame, rows[0], app);
    draw_playlist_panel(frame, rows[1], app);
}

fn draw_right_stack(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);

    draw_monitors_panel(frame, rows[0], app);
    draw_transitions_panel(frame, rows[1], app);
}

fn panel_border(active: bool, color: Color) -> Style {
    if active {
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Rgb(61, 75, 97))
    }
}

fn inner(area: Rect) -> Rect {
    area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    })
}

fn draw_browser_panel(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Library ",
                Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} items", app.browser_filtered.len()),
                Style::default().fg(C_MUTED),
            ),
        ]))
        .style(Style::default().bg(C_PANEL))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(panel_border(
            matches!(app.focus, FocusRegion::Library),
            C_ACCENT,
        ));

    frame.render_widget(block.clone(), area);

    let inner_area = inner(block.inner(area));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Min(5),
        ])
        .split(inner_area);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Dir ", Style::default().fg(C_MUTED)),
            Span::styled(
                app.browser_dir.display().to_string(),
                Style::default().fg(C_TEXT),
            ),
        ]))
        .style(Style::default().bg(C_PANEL_ALT)),
        rows[0],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Search ", Style::default().fg(C_MUTED)),
            Span::styled(
                if app.search_query.is_empty() {
                    "(all files)"
                } else {
                    &app.search_query
                },
                Style::default().fg(C_ACCENT_2),
            ),
        ]))
        .style(Style::default().bg(C_PANEL_ALT)),
        rows[1],
    );

    let visible_rows = rows[2].height.saturating_sub(2) as usize;
    let (items, selected_index) = visible_browser_items(app, visible_rows);

    let list = List::new(items.clone())
        .highlight_symbol("▸ ")
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(41, 55, 78))
                .fg(C_TEXT)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(C_PANEL));

    let mut state = app.browser_state;
    state.select(selected_index);
    frame.render_stateful_widget(list, rows[2], &mut state);
}

fn draw_playlist_panel(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Queue ",
                Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} entries", app.playlist.len()),
                Style::default().fg(C_MUTED),
            ),
        ]))
        .style(Style::default().bg(C_PANEL))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(panel_border(
            matches!(app.focus, FocusRegion::Playlist),
            C_ACCENT,
        ));

    frame.render_widget(block.clone(), area);

    let inner_area = inner(block.inner(area));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(3)])
        .split(inner_area);

    let visible_rows = rows[0].height.saturating_sub(2) as usize;
    let (items, selected_index) = visible_playlist_items(app, visible_rows);

    let list = List::new(items.clone())
        .highlight_symbol("▸ ")
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(41, 55, 78))
                .fg(C_TEXT)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(C_PANEL));

    let mut state = app.playlist_state;
    state.select(selected_index);
    frame.render_stateful_widget(list, rows[0], &mut state);

    let mut note_lines = Vec::new();
    note_lines.push(Line::from(vec![
        Span::styled(
            if app.playlist_running {
                "Auto"
            } else {
                "Paused"
            },
            Style::default()
                .fg(if app.playlist_running {
                    C_GREEN
                } else {
                    C_YELLOW
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | every ", Style::default().fg(C_MUTED)),
        Span::styled(
            format!("{}s", app.playlist_interval.as_secs()),
            Style::default().fg(C_TEXT),
        ),
    ]));

    if let Some(entry) = app.selected_playlist_entry() {
        note_lines.push(Line::from(vec![
            Span::styled("Active: ", Style::default().fg(C_MUTED)),
            Span::styled(
                entry
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("image")
                    .to_string(),
                Style::default().fg(C_TEXT),
            ),
        ]));
    }

    frame.render_widget(
        Paragraph::new(note_lines)
            .style(Style::default().bg(C_PANEL_ALT))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(61, 75, 97))),
            )
            .wrap(Wrap { trim: true }),
        rows[1],
    );
}

fn draw_preview_panel(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Stage ",
                Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(app.preview_title(), Style::default().fg(C_MUTED)),
        ]))
        .style(Style::default().bg(C_PANEL))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(panel_border(
            matches!(app.focus, FocusRegion::Preview),
            C_ACCENT,
        ));

    frame.render_widget(block.clone(), area);

    let inner_area = inner(block.inner(area));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((inner_area.height / 2).max(8)),
            Constraint::Min(8),
        ])
        .split(inner_area);

    let preview_text = preview_ascii(app, rows[0].width as usize, rows[0].height as usize);
    frame.render_widget(
        Paragraph::new(preview_text.join("\n"))
            .style(Style::default().bg(C_PANEL_ALT).fg(C_TEXT))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(61, 75, 97)))
                    .title(Line::from(vec![
                        Span::styled(" Monitor Ratio ", Style::default().fg(C_TEXT)),
                        Span::styled(app.scale_mode.label(), Style::default().fg(C_ACCENT_2)),
                        Span::styled(
                            if app.scale_mode.is_staged() {
                                " staged"
                            } else {
                                ""
                            },
                            Style::default().fg(C_YELLOW),
                        ),
                    ])),
            )
            .wrap(Wrap { trim: false }),
        rows[0],
    );

    let details = preview_details(app);
    frame.render_widget(
        Paragraph::new(details)
            .style(Style::default().bg(C_PANEL_ALT).fg(C_TEXT))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(61, 75, 97)))
                    .title(Line::from(vec![
                        Span::styled(" Image / Fit ", Style::default().fg(C_TEXT)),
                        Span::styled(app.rotation.label(), Style::default().fg(C_ACCENT_2)),
                    ])),
            )
            .wrap(Wrap { trim: true }),
        rows[1],
    );
}

fn draw_monitors_panel(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Outputs ",
                Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} discovered", app.monitors.len()),
                Style::default().fg(C_MUTED),
            ),
        ]))
        .style(Style::default().bg(C_PANEL))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(panel_border(
            matches!(app.focus, FocusRegion::Monitors),
            C_ACCENT,
        ));

    frame.render_widget(block.clone(), area);

    let inner_area = inner(block.inner(area));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(6)])
        .split(inner_area);

    let selected_index = app
        .monitor_selected
        .min(app.monitors.len().saturating_sub(1));
    let monitor_map = monitor_layout_ascii(
        &app.monitors,
        selected_index,
        rows[0].width as usize,
        rows[0].height as usize,
    );
    frame.render_widget(
        Paragraph::new(monitor_map.join("\n"))
            .style(Style::default().bg(C_PANEL_ALT).fg(C_TEXT))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(61, 75, 97)))
                    .title(Line::from(vec![
                        Span::styled(" Layout ", Style::default().fg(C_TEXT)),
                        Span::styled(target_summary(app), Style::default().fg(C_MUTED)),
                    ])),
            ),
        rows[0],
    );

    let visible_rows = rows[1].height.saturating_sub(2) as usize;
    let (items, selected_state) = visible_monitor_items(app, visible_rows);
    let list = List::new(items)
        .highlight_symbol("▸ ")
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(41, 55, 78))
                .fg(C_TEXT)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(C_PANEL));

    let mut state = app.monitor_state;
    state.select(selected_state);
    frame.render_stateful_widget(list, rows[1], &mut state);
}

fn draw_transitions_panel(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Transition ",
                Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if app.playlist_running {
                    "live"
                } else {
                    "paused"
                },
                Style::default().fg(if app.playlist_running {
                    C_GREEN
                } else {
                    C_YELLOW
                }),
            ),
        ]))
        .style(Style::default().bg(C_PANEL))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(panel_border(
            matches!(app.focus, FocusRegion::Transitions),
            C_ACCENT,
        ));

    frame.render_widget(block.clone(), area);

    let inner_area = inner(block.inner(area));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(6)])
        .split(inner_area);

    let fields = [0usize, 1, 2, 3].into_iter().map(|field_idx| {
        let active = app.transition.selected_field == field_idx
            && matches!(app.focus, FocusRegion::Transitions);
        Row::new(vec![
            if active { "▸" } else { " " }.to_string(),
            TransitionState::field_name(field_idx).to_string(),
            app.transition.field_value(field_idx),
        ])
        .style(if active {
            Style::default().bg(Color::Rgb(41, 55, 78)).fg(C_TEXT)
        } else {
            Style::default().fg(C_TEXT)
        })
    });

    let table = Table::new(
        fields,
        [
            Constraint::Length(2),
            Constraint::Length(12),
            Constraint::Min(8),
        ],
    )
    .header(
        Row::new(vec!["", "Field", "Value"])
            .style(Style::default().fg(C_MUTED).add_modifier(Modifier::BOLD)),
    )
    .column_spacing(1)
    .style(Style::default().bg(C_PANEL_ALT));

    frame.render_widget(table, rows[0]);

    let log_lines = if app.notifications.is_empty() {
        vec![Line::from(Span::styled(
            "No notifications yet",
            Style::default().fg(C_MUTED),
        ))]
    } else {
        app.notifications
            .iter()
            .rev()
            .take(6)
            .map(|note| {
                let color = match note.level {
                    NotificationLevel::Info => C_TEXT,
                    NotificationLevel::Success => C_GREEN,
                    NotificationLevel::Warn => C_YELLOW,
                    NotificationLevel::Error => C_RED,
                };
                Line::from(Span::styled(note.text.as_str(), Style::default().fg(color)))
            })
            .collect::<Vec<_>>()
    };

    frame.render_widget(
        Paragraph::new(log_lines)
            .style(Style::default().bg(C_PANEL_ALT).fg(C_TEXT))
            .block(
                Block::default()
                    .title(Line::from(vec![
                        Span::styled(" Activity ", Style::default().fg(C_TEXT)),
                        Span::styled(app.status.as_str(), Style::default().fg(C_MUTED)),
                    ]))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(61, 75, 97))),
            )
            .wrap(Wrap { trim: true }),
        rows[1],
    );
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let line = Line::from(vec![
        Span::styled(app_key_hints(app), Style::default().fg(C_ACCENT)),
        Span::styled("  |  ", Style::default().fg(C_MUTED)),
        Span::styled(global_key_hints(), Style::default().fg(C_TEXT)),
    ]);

    frame.render_widget(
        Paragraph::new(line).block(
            Block::default()
                .style(Style::default().bg(C_PANEL))
                .borders(Borders::ALL)
                .border_set(border::ROUNDED)
                .border_style(Style::default().fg(C_MUTED)),
        ),
        area,
    );
}

fn draw_help_overlay(frame: &mut Frame<'_>, app: &App) {
    let popup = centered_rect(76, 74, frame.area());
    frame.render_widget(Clear, popup);

    let text = format!(
        "VELLUM HELP\n\nFocus: {}\n\nGlobal\n- q quit\n- ? toggle help\n- Tab / Shift+Tab switch focus\n- Ctrl+r refresh monitors\n- b launch daemon\n\nLibrary\n- j/k or arrows move\n- Enter open folder or apply image\n- p add/remove playlist item\n- / search\n- g / G top or bottom\n\nPreview\n- f cycle fit mode\n- r rotate image\n- Enter apply to selected outputs\n\nMonitors\n- j/k select preview monitor\n- 1..9 toggle target monitor\n- m toggle active monitor target\n- A select all targets\n- x clear targets\n- Enter apply to selected outputs\n\nPlaylist\n- j/k move\n- Enter apply queued image\n- Space toggle auto-cycle\n- d delete item\n- x clear playlist\n- u / n move item\n\nTransitions\n- j/k select field\n- h/l or arrows edit field\n- +/- adjust playlist interval\n- Enter apply current selection\n\nPress Esc to close this overlay",
        app.focus.label()
    );

    frame.render_widget(
        Paragraph::new(text)
            .block(
                Block::default()
                    .title(" Quick Help ")
                    .style(Style::default().bg(C_PANEL))
                    .borders(Borders::ALL)
                    .border_set(border::ROUNDED)
                    .border_style(Style::default().fg(C_ACCENT_2)),
            )
            .style(Style::default().fg(C_TEXT).bg(C_PANEL_ALT))
            .wrap(Wrap { trim: true }),
        popup,
    );
}

fn preview_ascii(app: &App, width: usize, height: usize) -> Vec<String> {
    let Some(monitor) = app.selected_monitor() else {
        return vec![String::from("No monitor selected")];
    };

    let header = if let Some(preview) = app.selected_preview.as_ref() {
        let mut line = format!(
            "{}  {}x{}  ->  {}x{}",
            preview
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("image"),
            preview.width,
            preview.height,
            monitor.width,
            monitor.height
        );
        line.push_str(&format!(
            "  |  {} / {}",
            app.scale_mode.label(),
            app.rotation.label()
        ));
        if app.scale_mode.is_staged() {
            line.push_str("  |  preview only");
        }
        line
    } else {
        format!(
            "{}  {}x{}  |  no image selected",
            monitor.name, monitor.width, monitor.height
        )
    };

    let sim = selected_preview_simulation(app);
    let max_w = width.clamp(12, 72);
    let max_h = height.clamp(8, 24);
    let frame_w = if monitor.width >= monitor.height {
        max_w
    } else {
        ((max_h as f32 * monitor.width as f32 / monitor.height.max(1) as f32).round() as usize)
            .clamp(12, max_w)
    };
    let frame_h = if monitor.height >= monitor.width {
        max_h
    } else {
        ((max_w as f32 * monitor.height as f32 / monitor.width.max(1) as f32).round() as usize)
            .clamp(8, max_h)
    };

    let mut grid = vec![vec![' '; frame_w]; frame_h];
    if let Some(top) = grid.first_mut() {
        for cell in top.iter_mut() {
            *cell = '█';
        }
    }
    if let Some(bottom) = grid.last_mut() {
        for cell in bottom.iter_mut() {
            *cell = '█';
        }
    }
    for row in &mut grid {
        if let Some(first) = row.first_mut() {
            *first = '█';
        }
        if let Some(last) = row.last_mut() {
            *last = '█';
        }
    }

    if let Some(sim) = sim {
        let fill_w = ((sim.target_width as f32 / monitor.width.max(1) as f32)
            * (frame_w as f32 - 2.0))
            .round()
            .clamp(1.0, frame_w as f32 - 2.0) as usize;
        let fill_h = ((sim.target_height as f32 / monitor.height.max(1) as f32)
            * (frame_h as f32 - 2.0))
            .round()
            .clamp(1.0, frame_h as f32 - 2.0) as usize;

        let start_x = (frame_w.saturating_sub(fill_w + 2)) / 2 + 1;
        let start_y = (frame_h.saturating_sub(fill_h + 2)) / 2 + 1;

        let fill_char = match app.scale_mode {
            ScaleMode::Stretch => '▒',
            ScaleMode::Fill => '▓',
            ScaleMode::Fit => '░',
            ScaleMode::Center => '◼',
        };

        let y_end = (start_y + fill_h).min(frame_h - 1);
        let x_end = (start_x + fill_w).min(frame_w - 1);
        for row in grid
            .iter_mut()
            .skip(start_y)
            .take(y_end.saturating_sub(start_y))
        {
            for cell in row
                .iter_mut()
                .skip(start_x)
                .take(x_end.saturating_sub(start_x))
            {
                *cell = fill_char;
            }
        }

        let center_y = frame_h / 2;
        let center_x = frame_w / 2;
        if center_y < frame_h && center_x < frame_w {
            grid[center_y][center_x] = '+';
        }
    }

    let mut lines = vec![header];
    lines.push(String::from(""));
    lines.extend(
        grid.into_iter()
            .map(|row| row.into_iter().collect::<String>()),
    );
    lines
}

fn preview_details(app: &App) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    if let Some(preview) = app.selected_preview.as_ref() {
        lines.push(Line::from(vec![
            Span::styled("File ", Style::default().fg(C_MUTED)),
            Span::styled(
                preview
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("image")
                    .to_string(),
                Style::default().fg(C_TEXT),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Source ", Style::default().fg(C_MUTED)),
            Span::styled(
                format!("{}x{}", preview.width, preview.height),
                Style::default().fg(C_TEXT),
            ),
            Span::styled("  |  ", Style::default().fg(C_MUTED)),
            Span::styled(
                format!("{}deg", app.rotation.degrees()),
                Style::default().fg(C_ACCENT_2),
            ),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "Select an image in Library to inspect its fit.",
            Style::default().fg(C_MUTED),
        )));
    }

    if let Some(sim) = selected_preview_simulation(app) {
        lines.push(Line::from(vec![
            Span::styled("Render ", Style::default().fg(C_MUTED)),
            Span::styled(
                format!("{}x{}", sim.target_width, sim.target_height),
                Style::default().fg(C_TEXT),
            ),
            Span::styled("  |  bars ", Style::default().fg(C_MUTED)),
            Span::styled(
                format!("{}x{}", sim.bars_x, sim.bars_y),
                Style::default().fg(C_GREEN),
            ),
            Span::styled("  |  crop ", Style::default().fg(C_MUTED)),
            Span::styled(
                format!("{}x{}", sim.crop_x, sim.crop_y),
                Style::default().fg(C_YELLOW),
            ),
        ]));
    }

    lines.push(Line::from(vec![
        Span::styled("Placement ", Style::default().fg(C_MUTED)),
        Span::styled(app.scale_mode.label(), Style::default().fg(C_ACCENT_2)),
        Span::styled(
            if app.scale_mode.is_staged() {
                " (staged)"
            } else {
                ""
            },
            Style::default().fg(C_YELLOW),
        ),
    ]));

    if let Some(entry) = app.selected_playlist_entry() {
        lines.push(Line::from(vec![
            Span::styled("Queue ", Style::default().fg(C_MUTED)),
            Span::styled(
                entry
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("image")
                    .to_string(),
                Style::default().fg(C_TEXT),
            ),
            Span::styled("  |  ", Style::default().fg(C_MUTED)),
            Span::styled(entry.transition.summary(), Style::default().fg(C_MUTED)),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "Queue is empty. Add the current selection with p.",
            Style::default().fg(C_MUTED),
        )));
    }

    lines
}

fn visible_browser_items(
    app: &App,
    visible_rows: usize,
) -> (Vec<ListItem<'static>>, Option<usize>) {
    if app.browser_filtered.is_empty() {
        return (
            vec![ListItem::new(Span::styled(
                "No matching files",
                Style::default().fg(C_MUTED),
            ))],
            Some(0),
        );
    }

    let total = app.browser_filtered.len();
    let window = visible_rows.max(1).min(total);
    let start = app
        .browser_selected
        .saturating_sub(window.saturating_sub(1) / 2)
        .min(total.saturating_sub(window));
    let end = (start + window).min(total);

    let items = app.browser_filtered[start..end]
        .iter()
        .filter_map(|idx| app.browser_entries.get(*idx))
        .map(|entry| {
            let icon = match entry.kind {
                super::model::BrowserEntryKind::Parent => "..",
                super::model::BrowserEntryKind::Directory => "DIR",
                super::model::BrowserEntryKind::Image => "IMG",
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{icon:>3} "), Style::default().fg(C_MUTED)),
                Span::styled(entry.name.clone(), Style::default().fg(C_TEXT)),
            ]))
        })
        .collect::<Vec<_>>();

    (items, Some(app.browser_selected.saturating_sub(start)))
}

fn visible_playlist_items(
    app: &App,
    visible_rows: usize,
) -> (Vec<ListItem<'static>>, Option<usize>) {
    if app.playlist.is_empty() {
        return (
            vec![ListItem::new(Span::styled(
                "Playlist is empty",
                Style::default().fg(C_MUTED),
            ))],
            Some(0),
        );
    }

    let total = app.playlist.len();
    let window = visible_rows.max(1).min(total);
    let start = app
        .playlist_selected
        .saturating_sub(window.saturating_sub(1) / 2)
        .min(total.saturating_sub(window));
    let end = (start + window).min(total);

    let items = app.playlist[start..end]
        .iter()
        .map(|entry| {
            let file_name = entry
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("image");
            ListItem::new(Line::from(vec![
                Span::styled("• ", Style::default().fg(C_ACCENT_2)),
                Span::styled(file_name.to_string(), Style::default().fg(C_TEXT)),
                Span::styled("  ", Style::default().fg(C_MUTED)),
                Span::styled(entry.transition.summary(), Style::default().fg(C_MUTED)),
            ]))
        })
        .collect::<Vec<_>>();

    (items, Some(app.playlist_selected.saturating_sub(start)))
}

fn visible_monitor_items(
    app: &App,
    visible_rows: usize,
) -> (Vec<ListItem<'static>>, Option<usize>) {
    if app.monitors.is_empty() {
        return (
            vec![ListItem::new(Span::styled(
                "No monitors detected",
                Style::default().fg(C_MUTED),
            ))],
            Some(0),
        );
    }

    let total = app.monitors.len();
    let window = visible_rows.max(1).min(total);
    let start = app
        .monitor_selected
        .saturating_sub(window.saturating_sub(1) / 2)
        .min(total.saturating_sub(window));
    let end = (start + window).min(total);

    let items = app.monitors[start..end]
        .iter()
        .map(|monitor| {
            let selected = app.selected_targets.contains(&monitor.name);
            let marker = if selected { "[x]" } else { "[ ]" };
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{marker} "),
                    Style::default().fg(if selected { C_GREEN } else { C_MUTED }),
                ),
                Span::styled(monitor.name.clone(), Style::default().fg(C_TEXT)),
                Span::styled(
                    format!("  {}x{}", monitor.width, monitor.height),
                    Style::default().fg(C_MUTED),
                ),
                Span::styled(
                    if monitor.focused { "  focused" } else { "" },
                    Style::default().fg(C_ACCENT_2),
                ),
            ]))
        })
        .collect::<Vec<_>>();

    (items, Some(app.monitor_selected.saturating_sub(start)))
}

fn target_summary(app: &App) -> String {
    if app.selected_targets.is_empty() {
        return String::from(" follow active ");
    }

    format!(" {} selected ", app.selected_targets.len())
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(v[1])[1]
}
