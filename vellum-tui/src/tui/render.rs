use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap},
};
use ratatui_image::{Resize, StatefulImage};

use super::{
    App, LeftPaneTab, app_key_hints, daemon_status_color, daemon_status_text, global_key_hints,
};
use crate::tui::model::{FocusRegion, NotificationLevel};

const C_BG: Color = Color::Rgb(10, 14, 22);
const C_PANEL: Color = Color::Rgb(20, 26, 38);
const C_PANEL_ALT: Color = Color::Rgb(15, 20, 30);
const C_TEXT: Color = Color::Rgb(225, 231, 239);
const C_MUTED: Color = Color::Rgb(133, 149, 173);
const C_ACCENT: Color = Color::Rgb(74, 189, 255);
const C_ACCENT_2: Color = Color::Rgb(255, 179, 102);
const C_GREEN: Color = Color::Rgb(120, 210, 151);

pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
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

    if app.activity_open {
        draw_activity_overlay(frame, app);
    }

    if app.help_open {
        draw_help_overlay(frame, app);
    }
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(72), Constraint::Percentage(28)])
        .split(area);

    let monitor_tabs = Tabs::new(monitor_tab_labels(app))
        .select(
            app.monitor_selected
                .min(app.monitors.len().saturating_sub(1)),
        )
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
                    Span::styled(" monitors ", Style::default().fg(C_MUTED)),
                ]))
                .borders(Borders::ALL)
                .border_set(border::ROUNDED)
                .border_style(Style::default().fg(C_MUTED))
                .style(Style::default().bg(C_PANEL)),
        );
    frame.render_widget(monitor_tabs, cols[0]);

    let status = Paragraph::new(Line::from(vec![
        Span::styled("Daemon ", Style::default().fg(C_MUTED)),
        Span::styled(
            daemon_status_text(app),
            Style::default()
                .fg(daemon_status_color(app))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  |  ", Style::default().fg(C_MUTED)),
        Span::styled(
            app.selected_output_names().join(", "),
            Style::default().fg(C_TEXT),
        ),
    ]))
    .block(
        Block::default()
            .title(Line::from(vec![
                Span::styled(" Status ", Style::default().fg(C_MUTED)),
                Span::styled(" daemon online/offline ", Style::default().fg(C_ACCENT_2)),
            ]))
            .borders(Borders::ALL)
            .border_set(border::ROUNDED)
            .border_style(Style::default().fg(C_MUTED))
            .style(Style::default().bg(C_PANEL)),
    )
    .style(Style::default().bg(C_PANEL_ALT).fg(C_TEXT))
    .wrap(Wrap { trim: true });

    frame.render_widget(status, cols[1]);
}

fn draw_body(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(area);

    draw_left_pane(frame, cols[0], app);
    draw_stage_pane(frame, cols[1], app);
}

fn draw_left_pane(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Library / Queue ",
                Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                match app.left_tab {
                    LeftPaneTab::LibraryExplorer => "library",
                    LeftPaneTab::ActiveQueue => "queue",
                },
                Style::default().fg(C_MUTED),
            ),
        ]))
        .style(Style::default().bg(C_PANEL))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(panel_border(
            matches!(app.focus, FocusRegion::Library | FocusRegion::Playlist),
            C_ACCENT,
        ));

    frame.render_widget(block.clone(), area);
    let inner_area = inner(block.inner(area));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8)])
        .split(inner_area);

    let tabs = Tabs::new(vec![
        Line::from(Span::styled(" Library Explorer ", Style::default())),
        Line::from(Span::styled(" Active Queue ", Style::default())),
    ])
    .select(match app.left_tab {
        LeftPaneTab::LibraryExplorer => 0,
        LeftPaneTab::ActiveQueue => 1,
    })
    .style(Style::default().fg(C_MUTED).bg(C_PANEL_ALT))
    .highlight_style(
        Style::default()
            .fg(C_BG)
            .bg(C_ACCENT)
            .add_modifier(Modifier::BOLD),
    )
    .divider(" ")
    .block(Block::default().style(Style::default().bg(C_PANEL_ALT)));
    frame.render_widget(tabs, chunks[0]);

    match app.left_tab {
        LeftPaneTab::LibraryExplorer => draw_library_list(frame, chunks[1], app),
        LeftPaneTab::ActiveQueue => draw_queue_list(frame, chunks[1], app),
    }
}

fn draw_stage_pane(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
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
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(inner_area);

    if let Some(state) = app.preview_state.as_mut() {
        frame.render_stateful_widget(
            StatefulImage::default().resize(Resize::Fit(None)),
            rows[0],
            state,
        );
    } else {
        frame.render_widget(
            Paragraph::new(preview_empty_lines(app))
                .style(Style::default().bg(C_PANEL_ALT).fg(C_TEXT))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Rgb(61, 75, 97)))
                        .title(Line::from(vec![
                            Span::styled(" Preview ", Style::default().fg(C_TEXT)),
                            Span::styled(app.scale_mode.label(), Style::default().fg(C_ACCENT_2)),
                        ])),
                )
                .wrap(Wrap { trim: true }),
            rows[0],
        );
    }

    frame.render_widget(
        Paragraph::new(stage_details(app))
            .style(Style::default().bg(C_PANEL_ALT).fg(C_TEXT))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(61, 75, 97)))
                    .title(Line::from(vec![
                        Span::styled(" Settings ", Style::default().fg(C_TEXT)),
                        Span::styled(" fit / transition ", Style::default().fg(C_MUTED)),
                    ])),
            )
            .wrap(Wrap { trim: true }),
        rows[1],
    );
}

fn draw_library_list(frame: &mut Frame<'_>, area: Rect, app: &App) {
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
        .style(Style::default().bg(C_PANEL_ALT))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(panel_border(
            matches!(app.focus, FocusRegion::Library),
            C_ACCENT,
        ));

    frame.render_widget(block.clone(), area);
    let inner_area = inner(block.inner(area));
    let visible_rows = inner_area.height.saturating_sub(2) as usize;
    let (items, selected_index) = visible_browser_items(app, visible_rows);

    let list = List::new(items)
        .highlight_symbol("▸ ")
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(41, 55, 78))
                .fg(C_TEXT)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(C_PANEL_ALT));

    let mut state = app.browser_state;
    state.select(selected_index);
    frame.render_stateful_widget(list, inner_area, &mut state);
}

fn draw_queue_list(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Active Queue ",
                Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} items", app.playlist.len()),
                Style::default().fg(C_MUTED),
            ),
        ]))
        .style(Style::default().bg(C_PANEL_ALT))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(panel_border(
            matches!(app.focus, FocusRegion::Playlist),
            C_ACCENT,
        ));

    frame.render_widget(block.clone(), area);
    let inner_area = inner(block.inner(area));
    let visible_rows = inner_area.height.saturating_sub(2) as usize;
    let (items, selected_index) = visible_playlist_items(app, visible_rows);

    let list = List::new(items)
        .highlight_symbol("▸ ")
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(41, 55, 78))
                .fg(C_TEXT)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(C_PANEL_ALT));

    let mut state = app.playlist_state;
    state.select(selected_index);
    frame.render_stateful_widget(list, inner_area, &mut state);
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(app_key_hints(app), Style::default().fg(C_ACCENT)),
            Span::styled("  |  ", Style::default().fg(C_MUTED)),
            Span::styled(global_key_hints(), Style::default().fg(C_TEXT)),
        ]))
        .block(
            Block::default()
                .style(Style::default().bg(C_PANEL))
                .borders(Borders::ALL)
                .border_set(border::ROUNDED)
                .border_style(Style::default().fg(C_MUTED)),
        )
        .style(Style::default().bg(C_PANEL_ALT).fg(C_TEXT))
        .wrap(Wrap { trim: true }),
        area,
    );
}

fn draw_help_overlay(frame: &mut Frame<'_>, app: &App) {
    let popup = centered_rect(76, 74, frame.area());
    frame.render_widget(Clear, popup);

    let text = format!(
        "VELLUM HELP\n\nFocus: {}\n\nGlobal\n- q quit\n- ? toggle help\n- L open or close activity log\n- 1-4 switch monitor tab\n- Tab / Shift+Tab switch panel focus\n- Ctrl+r refresh monitors\n- b launch daemon\n\nLibrary\n- j/k or arrows move\n- Enter open folder or apply image\n- p add/remove playlist item\n- / search\n- g / G top or bottom\n\nPreview\n- f cycle fit mode\n- r rotate image\n- Enter apply to selected outputs\n\nPlaylist\n- j/k move\n- Enter apply queued image\n- Space toggle auto-cycle\n- d delete item\n- x clear playlist\n- u / n move item\n\nTransitions\n- j/k select field\n- h/l or arrows edit field\n- +/- adjust playlist interval\n- Enter apply current selection\n\nPress Esc to close this overlay",
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

fn draw_activity_overlay(frame: &mut Frame<'_>, app: &App) {
    let popup = centered_rect(70, 68, frame.area());
    frame.render_widget(Clear, popup);

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            "Activity feed",
            Style::default().fg(C_ACCENT_2).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  recent status messages", Style::default().fg(C_MUTED)),
    ]));
    lines.push(Line::from(Span::styled("", Style::default())));

    if app.notifications.is_empty() {
        lines.push(Line::from(Span::styled(
            "No activity yet",
            Style::default().fg(C_MUTED),
        )));
    } else {
        for note in app.notifications.iter().rev().take(12) {
            let color = match note.level {
                NotificationLevel::Info => C_TEXT,
                NotificationLevel::Success => C_GREEN,
                NotificationLevel::Warn => C_ACCENT_2,
                NotificationLevel::Error => Color::Rgb(242, 112, 122),
            };

            lines.push(Line::from(vec![
                Span::styled(
                    match note.level {
                        NotificationLevel::Info => "info",
                        NotificationLevel::Success => "ok",
                        NotificationLevel::Warn => "warn",
                        NotificationLevel::Error => "error",
                    },
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled("  ", Style::default().fg(C_MUTED)),
                Span::styled(note.text.as_str(), Style::default().fg(C_TEXT)),
            ]));
        }
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Activity ")
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

fn monitor_tab_labels(app: &App) -> Vec<Line<'static>> {
    if app.monitors.is_empty() {
        return vec![Line::from(Span::styled(
            " 1. waiting for monitors ",
            Style::default().fg(C_MUTED),
        ))];
    }

    let mut labels = app
        .monitors
        .iter()
        .take(4)
        .enumerate()
        .map(|(slot, monitor)| {
            let selected = slot == app.monitor_selected;
            let target_mark = if app.selected_targets.contains(&monitor.name) {
                "*"
            } else {
                ""
            };
            Line::from(vec![
                Span::styled(
                    format!("{}{}. ", if selected { "▸ " } else { "" }, slot + 1),
                    Style::default().fg(if selected { C_ACCENT_2 } else { C_MUTED }),
                ),
                Span::styled(monitor.name.clone(), Style::default().fg(C_TEXT)),
                Span::styled(target_mark, Style::default().fg(C_GREEN)),
            ])
        })
        .collect::<Vec<_>>();

    if app.monitors.len() > 4 {
        labels.push(Line::from(Span::styled(
            format!(" +{} more", app.monitors.len() - 4),
            Style::default().fg(C_MUTED),
        )));
    }

    labels
}

fn preview_empty_lines(app: &App) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(entry) = app.selected_browser_entry() {
        lines.push(Line::from(vec![
            Span::styled("File ", Style::default().fg(C_MUTED)),
            Span::styled(entry.name.clone(), Style::default().fg(C_TEXT)),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "No image selected",
            Style::default().fg(C_MUTED),
        )));
    }

    lines.push(Line::from(vec![
        Span::styled("Preview ", Style::default().fg(C_MUTED)),
        Span::styled("waiting for a valid image", Style::default().fg(C_TEXT)),
    ]));

    lines
}

fn stage_details(app: &App) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if let Some(preview) = app.selected_preview.as_ref() {
        lines.push(Line::from(vec![
            Span::styled("Image ", Style::default().fg(C_MUTED)),
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
        ]));
    }

    if let Some(monitor) = app.selected_monitor() {
        lines.push(Line::from(vec![
            Span::styled("Monitor ", Style::default().fg(C_MUTED)),
            Span::styled(
                format!("{}x{}", monitor.width, monitor.height),
                Style::default().fg(C_TEXT),
            ),
            Span::styled("  |  ", Style::default().fg(C_MUTED)),
            Span::styled(monitor.name.clone(), Style::default().fg(C_ACCENT_2)),
        ]));
    }

    lines.push(Line::from(vec![
        Span::styled("Fit ", Style::default().fg(C_MUTED)),
        Span::styled(app.scale_mode.label(), Style::default().fg(C_TEXT)),
        Span::styled("  |  Rotation ", Style::default().fg(C_MUTED)),
        Span::styled(
            format!("{} ({}deg)", app.rotation.label(), app.rotation.degrees()),
            Style::default().fg(C_ACCENT_2),
        ),
    ]));

    for field_idx in 0..4 {
        lines.push(Line::from(vec![
            Span::styled(
                format!(
                    "{} ",
                    crate::tui::model::TransitionState::field_name(field_idx)
                ),
                Style::default().fg(C_MUTED),
            ),
            Span::styled(
                app.transition.field_value(field_idx),
                Style::default().fg(C_TEXT),
            ),
        ]));
    }

    lines.push(Line::from(vec![
        Span::styled("Summary ", Style::default().fg(C_MUTED)),
        Span::styled(app.transition.summary(), Style::default().fg(C_ACCENT_2)),
    ]));

    lines
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

fn visible_browser_items(
    app: &App,
    visible_rows: usize,
) -> (Vec<ListItem<'static>>, Option<usize>) {
    if app.browser_filtered.is_empty() {
        return (
            vec![ListItem::new(Span::styled(
                "No images found",
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
        .filter_map(|index| app.browser_entries.get(*index))
        .map(|entry| {
            ListItem::new(Line::from(vec![
                Span::styled(entry.name.clone(), Style::default().fg(C_TEXT)),
                Span::styled(
                    if entry.is_dir() { "  dir" } else { "  img" },
                    Style::default().fg(C_MUTED),
                ),
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
                "Queue is empty",
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
            ListItem::new(Line::from(vec![
                Span::styled(
                    entry
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("image")
                        .to_string(),
                    Style::default().fg(C_TEXT),
                ),
                Span::styled("  on ", Style::default().fg(C_MUTED)),
                Span::styled(
                    if entry.target_monitor.is_empty() {
                        String::from("current")
                    } else {
                        entry.target_monitor.clone()
                    },
                    Style::default().fg(C_ACCENT_2),
                ),
            ]))
        })
        .collect::<Vec<_>>();

    (items, Some(app.playlist_selected.saturating_sub(start)))
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
