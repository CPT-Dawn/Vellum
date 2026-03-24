use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Row, Table, Tabs, Wrap},
};

use super::{
    App, daemon_status_color_online, daemon_status_text, global_key_hints, monitor_slot_labels,
    panel_key_hints, selected_preview_simulation,
};
use crate::tui::data::{InputMode, NotificationLevel, Panel, make_monitor_layout_ascii};

const C_BG: Color = Color::Rgb(14, 18, 26);
const C_PANEL: Color = Color::Rgb(22, 28, 41);
const C_PANEL_ALT: Color = Color::Rgb(18, 23, 35);
const C_TEXT: Color = Color::Rgb(214, 222, 235);
const C_MUTED: Color = Color::Rgb(122, 142, 168);
const C_ACCENT: Color = Color::Rgb(49, 181, 255);
const C_ACCENT_ALT: Color = Color::Rgb(255, 173, 92);
const C_OK: Color = Color::Rgb(118, 200, 147);
const C_WARN: Color = Color::Rgb(240, 186, 93);
const C_ERR: Color = Color::Rgb(237, 114, 120);

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    frame.render_widget(
        Block::default().style(Style::default().bg(C_BG)),
        frame.area(),
    );

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(frame.area());

    draw_header(frame, rows[0], app);
    draw_body(frame, rows[1], app);
    draw_footer(frame, rows[2], app);

    if app.help_open {
        draw_help_overlay(frame, app);
    }

    draw_toast(frame, app);
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(64), Constraint::Percentage(36)])
        .split(area);

    let tabs = Tabs::new(vec![" Library ", " Monitor ", " Playback "])
        .select(match app.panel {
            Panel::Library => 0,
            Panel::Monitor => 1,
            Panel::Playback => 2,
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
                        Style::default()
                            .fg(C_ACCENT_ALT)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(" redesigned TUI", Style::default().fg(C_MUTED)),
                ]))
                .style(Style::default().bg(C_PANEL))
                .borders(Borders::ALL)
                .border_set(border::ROUNDED)
                .border_style(Style::default().fg(C_MUTED)),
        );

    frame.render_widget(tabs, cols[0]);

    let daemon_online = daemon_status_color_online(app);
    let right = Paragraph::new(Line::from(vec![
        Span::styled("Daemon ", Style::default().fg(C_MUTED)),
        Span::styled(
            daemon_status_text(app),
            Style::default()
                .fg(if daemon_online { C_OK } else { C_WARN })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | Monitors ", Style::default().fg(C_MUTED)),
        Span::styled(
            app.monitors.len().to_string(),
            Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | Mode ", Style::default().fg(C_MUTED)),
        Span::styled(
            if app.input_mode == InputMode::Search {
                "SEARCH"
            } else {
                "NORMAL"
            },
            Style::default().fg(if app.input_mode == InputMode::Search {
                C_WARN
            } else {
                C_OK
            }),
        ),
    ]))
    .block(
        Block::default()
            .style(Style::default().bg(C_PANEL))
            .borders(Borders::ALL)
            .border_set(border::ROUNDED)
            .border_style(Style::default().fg(C_MUTED)),
    );

    frame.render_widget(right, cols[1]);
}

fn draw_body(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(38),
            Constraint::Percentage(28),
        ])
        .split(area);

    draw_library_panel(frame, cols[0], app);
    draw_monitor_panel(frame, cols[1], app);
    draw_playback_panel(frame, cols[2], app);
}

fn panel_border(active: bool) -> Style {
    if active {
        Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Rgb(67, 84, 110))
    }
}

fn inner(area: Rect) -> Rect {
    area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    })
}

fn draw_library_panel(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Library ",
                Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} files", app.browser_filtered.len()),
                Style::default().fg(C_MUTED),
            ),
        ]))
        .style(Style::default().bg(C_PANEL))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(panel_border(matches!(app.panel, Panel::Library)));

    frame.render_widget(block.clone(), area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(5),
        ])
        .split(inner(block.inner(area)));

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Dir: ", Style::default().fg(C_MUTED)),
            Span::styled(
                app.browser_dir.display().to_string(),
                Style::default().fg(C_TEXT),
            ),
        ])),
        chunks[0],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Search: ", Style::default().fg(C_MUTED)),
            Span::styled(
                if app.search_query.is_empty() {
                    "(none)"
                } else {
                    &app.search_query
                },
                Style::default().fg(C_ACCENT),
            ),
        ]))
        .style(Style::default().bg(C_PANEL_ALT)),
        chunks[1],
    );

    let items = if app.browser_filtered.is_empty() {
        vec![ListItem::new(Span::styled(
            "No matching files",
            Style::default().fg(C_MUTED),
        ))]
    } else {
        app.browser_filtered
            .iter()
            .filter_map(|idx| app.browser_entries.get(*idx))
            .map(|entry| {
                let icon = if entry.is_parent {
                    ".."
                } else if entry.is_dir {
                    "DIR"
                } else {
                    "IMG"
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{icon:>3} "), Style::default().fg(C_MUTED)),
                    Span::styled(entry.name.as_str(), Style::default().fg(C_TEXT)),
                ]))
            })
            .collect()
    };

    let list = List::new(items)
        .highlight_symbol("▸ ")
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(42, 56, 83))
                .fg(C_TEXT)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(C_PANEL));

    let mut state = app.browser_state;
    frame.render_stateful_widget(list, chunks[2], &mut state);

    let playlist_lines = if app.playlist.is_empty() {
        vec![Line::from(Span::styled(
            "Playlist empty (p to add)",
            Style::default().fg(C_MUTED),
        ))]
    } else {
        app.playlist
            .iter()
            .take(3)
            .map(|p| {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("image");
                Line::from(vec![
                    Span::styled("• ", Style::default().fg(C_ACCENT_ALT)),
                    Span::styled(name, Style::default().fg(C_TEXT)),
                ])
            })
            .collect::<Vec<_>>()
    };

    frame.render_widget(
        Paragraph::new(playlist_lines)
            .block(
                Block::default()
                    .title(Line::from(Span::styled(
                        format!(
                            " Playlist {} | every {}s ",
                            if app.playlist_running {
                                "running"
                            } else {
                                "paused"
                            },
                            app.playlist_interval.as_secs()
                        ),
                        Style::default().fg(C_MUTED),
                    )))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(67, 84, 110))),
            )
            .style(Style::default().bg(C_PANEL_ALT)),
        chunks[3],
    );
}

fn draw_monitor_panel(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Monitor Layout ",
                Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                monitor_slot_labels(&app.monitors),
                Style::default().fg(C_MUTED),
            ),
        ]))
        .style(Style::default().bg(C_PANEL))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(panel_border(matches!(app.panel, Panel::Monitor)));

    frame.render_widget(block.clone(), area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(9),
            Constraint::Length(10),
            Constraint::Min(6),
        ])
        .split(inner(block.inner(area)));

    let layout_ascii = make_monitor_layout_ascii(&app.monitors, app.monitor_selected, 42, 7);
    frame.render_widget(
        Paragraph::new(layout_ascii.join("\n")).style(Style::default().fg(C_TEXT).bg(C_PANEL_ALT)),
        chunks[0],
    );

    let preview_ascii = super::preview_ascii(app, 42, 8);
    frame.render_widget(
        Paragraph::new(preview_ascii.join("\n"))
            .block(
                Block::default()
                    .title(" Wallpaper Preview ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(67, 84, 110))),
            )
            .style(Style::default().bg(C_PANEL_ALT)),
        chunks[1],
    );

    let monitor_details = if let Some(mon) = app.monitors.get(app.monitor_selected) {
        let mut text = format!(
            "Active: {}{}\nResolution: {}x{}\nPosition: ({}, {})\nScale: {}\nRotation: {}deg",
            mon.name,
            if mon.focused { " (focused)" } else { "" },
            mon.width,
            mon.height,
            mon.x,
            mon.y,
            app.scale_mode.as_str(),
            app.rotation.degrees()
        );

        if let (Some(preview), Some(sim)) = (
            app.selected_preview.as_ref(),
            selected_preview_simulation(app),
        ) {
            text.push_str(&format!(
                "\n\nImage: {}\nSource: {}x{}\nRender: {}x{}\nBars: {}x{}\nCrop: {}x{}",
                preview.path.display(),
                preview.width,
                preview.height,
                sim.target_width,
                sim.target_height,
                sim.bars_x,
                sim.bars_y,
                sim.crop_x,
                sim.crop_y
            ));
        } else {
            text.push_str("\n\nSelect an image in Library");
        }

        text
    } else {
        String::from("No monitor selected")
    };

    frame.render_widget(
        Paragraph::new(monitor_details)
            .style(Style::default().fg(C_TEXT).bg(C_PANEL_ALT))
            .wrap(Wrap { trim: false }),
        chunks[2],
    );
}

fn draw_playback_panel(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Playback ",
                Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled("transition + cycle", Style::default().fg(C_MUTED)),
        ]))
        .style(Style::default().bg(C_PANEL))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(panel_border(matches!(app.panel, Panel::Playback)));

    frame.render_widget(block.clone(), area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(8)])
        .split(inner(block.inner(area)));

    let rows = [
        (
            "duration",
            format!("{} ms", app.transition.duration_ms),
            0usize,
        ),
        ("fps", app.transition.fps.to_string(), 1usize),
        (
            "easing",
            crate::tui::data::EASING_PRESETS[app.transition.easing_idx].to_string(),
            2usize,
        ),
        (
            "effect",
            crate::tui::data::TRANSITION_EFFECTS[app.transition.effect_idx].to_string(),
            3usize,
        ),
    ];

    let table_rows = rows.into_iter().map(|(label, value, idx)| {
        let selected = matches!(app.panel, Panel::Playback) && app.transition.selected_field == idx;
        Row::new(vec![
            if selected {
                String::from("▸")
            } else {
                String::from(" ")
            },
            label.to_string(),
            value,
        ])
        .style(if selected {
            Style::default().bg(Color::Rgb(42, 56, 83)).fg(C_TEXT)
        } else {
            Style::default().fg(C_TEXT)
        })
    });

    let table = Table::new(
        table_rows,
        [
            Constraint::Length(2),
            Constraint::Length(9),
            Constraint::Min(10),
        ],
    )
    .header(
        Row::new(vec!["", "Field", "Value"])
            .style(Style::default().fg(C_MUTED).add_modifier(Modifier::BOLD)),
    )
    .column_spacing(1)
    .style(Style::default().bg(C_PANEL_ALT));

    frame.render_widget(table, chunks[0]);

    let log_lines = if app.notifications.is_empty() {
        vec![Line::from(Span::styled(
            "No notifications",
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
                    NotificationLevel::Success => C_OK,
                    NotificationLevel::Warn => C_WARN,
                    NotificationLevel::Error => C_ERR,
                };
                Line::from(Span::styled(note.text.as_str(), Style::default().fg(color)))
            })
            .collect::<Vec<_>>()
    };

    frame.render_widget(
        Paragraph::new(log_lines)
            .block(
                Block::default()
                    .title(Line::from(Span::styled(
                        format!(
                            " Logs | Playlist {} | every {}s ",
                            if app.playlist_running {
                                "running"
                            } else {
                                "paused"
                            },
                            app.playlist_interval.as_secs()
                        ),
                        Style::default().fg(C_MUTED),
                    )))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(67, 84, 110))),
            )
            .style(Style::default().bg(C_PANEL_ALT)),
        chunks[1],
    );
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let line = Line::from(vec![
        Span::styled(panel_key_hints(app), Style::default().fg(C_ACCENT)),
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
    let popup = centered_rect(72, 70, frame.area());
    frame.render_widget(Clear, popup);

    let text = format!(
        "VELLUM HELP\n\nActive Panel: {}\n\nGlobal\n- q quit\n- Tab / Shift+Tab switch panel\n- r refresh monitors\n- b launch daemon\n- 1..9 quick monitor select\n\nLibrary\n- j/k move\n- Enter open/apply\n- p add/remove playlist\n- / search\n- Space toggle playlist\n\nMonitor\n- j/k select monitor\n- f scale mode\n- o rotation\n- a or Enter apply\n\nPlayback\n- j/k select transition field\n- h/l edit field\n- +/- playlist interval\n- Enter apply\n\nPress ? or Esc to close",
        app.panel.title()
    );

    frame.render_widget(
        Paragraph::new(text)
            .block(
                Block::default()
                    .title(" Quick Help ")
                    .style(Style::default().bg(C_PANEL))
                    .borders(Borders::ALL)
                    .border_set(border::ROUNDED)
                    .border_style(Style::default().fg(C_ACCENT_ALT)),
            )
            .style(Style::default().fg(C_TEXT)),
        popup,
    );
}

fn draw_toast(frame: &mut Frame<'_>, app: &App) {
    let Some(note) = app.notifications.back() else {
        return;
    };
    if note.created_at.elapsed().as_secs_f32() > 4.0 {
        return;
    }

    let area = toast_rect(frame.area(), 42, 3);
    frame.render_widget(Clear, area);

    let color = match note.level {
        NotificationLevel::Info => C_ACCENT,
        NotificationLevel::Success => C_OK,
        NotificationLevel::Warn => C_WARN,
        NotificationLevel::Error => C_ERR,
    };

    frame.render_widget(
        Paragraph::new(note.text.as_str())
            .block(
                Block::default()
                    .title(" Notification ")
                    .borders(Borders::ALL)
                    .border_set(border::ROUNDED)
                    .border_style(Style::default().fg(color)),
            )
            .style(Style::default().bg(C_PANEL_ALT).fg(C_TEXT))
            .wrap(Wrap { trim: true }),
        area,
    );
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

fn toast_rect(area: Rect, width_percent: u16, height: u16) -> Rect {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(height), Constraint::Min(1)])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(100 - width_percent),
            Constraint::Percentage(width_percent),
        ])
        .split(rows[0])[1]
}
