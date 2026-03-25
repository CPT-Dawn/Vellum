use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::{App, DaemonStatus, FileEntry, FileKind, Monitor};

const BG: Color = Color::Rgb(11, 15, 20);
const PANEL: Color = Color::Rgb(17, 23, 32);
const PANEL_ALT: Color = Color::Rgb(22, 30, 42);
const BORDER: Color = Color::Rgb(52, 66, 84);
const ACCENT: Color = Color::Rgb(98, 196, 230);
const ACCENT_SOFT: Color = Color::Rgb(68, 120, 160);
const GOOD: Color = Color::Rgb(84, 190, 132);
const WARN: Color = Color::Rgb(227, 174, 90);
const BAD: Color = Color::Rgb(224, 92, 110);
const TEXT: Color = Color::Rgb(220, 228, 237);
const MUTED: Color = Color::Rgb(145, 156, 170);

pub fn draw(frame: &mut Frame, app: &App) {
    let root = frame.area();
    frame.render_widget(Block::default().style(Style::default().bg(BG)), root);

    let vertical = Layout::vertical([
        Constraint::Percentage(10),
        Constraint::Percentage(70),
        Constraint::Percentage(20),
    ])
    .spacing(1);
    let [top, middle, bottom] = vertical.areas(root);

    draw_top_bar(frame, top, app);

    let middle_layout = Layout::horizontal([
        Constraint::Percentage(30),
        Constraint::Percentage(50),
        Constraint::Percentage(20),
    ])
    .spacing(1);
    let [browser_area, canvas_area, controls_area] = middle_layout.areas(middle);

    draw_browser(frame, browser_area, app);
    draw_monitor_canvas(frame, canvas_area, app);
    draw_controls(frame, controls_area, app);

    let bottom_layout =
        Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)]).spacing(1);
    let [logs_area, legend_area] = bottom_layout.areas(bottom);

    draw_logs(frame, logs_area, app);
    draw_legend(frame, legend_area, app);
}

fn draw_top_bar(frame: &mut Frame, area: Rect, app: &App) {
    if app.search_active {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL))
            .title(Line::from(vec![
                Span::styled(
                    " Search ",
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::styled("/", Style::default().fg(WARN)),
            ]));

        let input = Paragraph::new(Text::from(format!("/{}", app.search_buffer)))
            .style(Style::default().fg(TEXT))
            .block(block);

        frame.render_widget(input, area);
        return;
    }

    let status = daemon_status_label(app.daemon_status);
    let monitor = app.selected_monitor_label();
    let wallpaper = app.selected_wallpaper_label();
    let title = Line::from(vec![
        Span::styled(
            " Waywall ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled("Wallpaper Manager Frontend", Style::default().fg(TEXT)),
    ]);
    let subtitle = Line::from(vec![
        Span::styled(" Daemon ", Style::default().fg(MUTED)),
        Span::styled(status, status_style(app.daemon_status)),
        Span::raw("  "),
        Span::styled("Monitor ", Style::default().fg(MUTED)),
        Span::styled(monitor, Style::default().fg(ACCENT_SOFT)),
        Span::raw("  "),
        Span::styled("Wallpaper ", Style::default().fg(MUTED)),
        Span::styled(wallpaper, Style::default().fg(TEXT)),
    ]);

    let paragraph = Paragraph::new(Text::from(vec![title, subtitle]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER))
                .style(Style::default().bg(PANEL)),
        )
        .style(Style::default().fg(TEXT));

    frame.render_widget(paragraph, area);
}

fn draw_browser(frame: &mut Frame, area: Rect, app: &App) {
    let items = app
        .visible_browser_items()
        .map(|(_, entry)| browser_item(entry))
        .collect::<Vec<_>>();

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Files ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{} items", items.len()), Style::default().fg(MUTED)),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().bg(PANEL));

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .fg(TEXT)
                .bg(PANEL_ALT)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    let mut state = ListState::default();
    state.select(if app.browser_filtered_indices.is_empty() {
        None
    } else {
        Some(app.browser_selected)
    });

    frame.render_stateful_widget(list, area, &mut state);
}

fn browser_item(entry: &FileEntry) -> ListItem<'static> {
    let favorite = entry.favorite;
    let icon = match entry.kind {
        FileKind::Directory => "[DIR]",
        FileKind::File => "[FILE]",
    };
    let support_badge = if entry.kind == FileKind::File && entry.supported {
        "IMG"
    } else if entry.kind == FileKind::File {
        "UNSUP"
    } else {
        "DIR"
    };

    let mut spans = vec![Span::styled(
        format!("{} ", icon),
        Style::default().fg(if entry.kind == FileKind::Directory {
            ACCENT
        } else {
            TEXT
        }),
    )];

    if favorite {
        spans.push(Span::styled(
            "★ ",
            Style::default().fg(WARN).add_modifier(Modifier::BOLD),
        ));
    }

    spans.push(Span::styled(
        entry.name.clone(),
        Style::default().fg(if favorite { WARN } else { TEXT }),
    ));

    spans.push(Span::styled(
        format!("  [{}]", support_badge),
        Style::default().fg(MUTED),
    ));

    ListItem::new(Line::from(spans))
}

fn draw_monitor_canvas(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Monitor Canvas ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "{} / {}",
                    app.selected_monitor_label(),
                    app.current_scaling_mode()
                ),
                Style::default().fg(MUTED),
            ),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().bg(PANEL));
    frame.render_widget(block, area);

    if let Some(monitor) = app.selected_monitor_ref() {
        let preview = fitted_monitor_rect(area, monitor);
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ACCENT))
                .style(Style::default().bg(PANEL_ALT))
                .title(Line::from(vec![
                    Span::styled(
                        " Preview ",
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{}: {}x{}", monitor.name, monitor.width, monitor.height),
                        Style::default().fg(MUTED),
                    ),
                ])),
            preview,
        );

        let inner = inner_rect(preview);
        let wallpaper = app
            .selected_browser_entry()
            .map(|entry| entry.name.clone())
            .unwrap_or_else(|| "No wallpaper selected".to_string());
        let content = vec![
            Line::from(vec![
                Span::styled("Image ", Style::default().fg(MUTED)),
                Span::styled(
                    format!("[{}]", app.current_scaling_mode()),
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" to {}x{}", monitor.width, monitor.height),
                    Style::default().fg(TEXT),
                ),
            ]),
            Line::from(vec![
                Span::styled("Monitor ", Style::default().fg(MUTED)),
                Span::styled(monitor.name.clone(), Style::default().fg(TEXT)),
                Span::raw(" • "),
                Span::styled(
                    format!("aspect {:.2}:1", monitor.aspect_ratio()),
                    Style::default().fg(ACCENT_SOFT),
                ),
            ]),
            Line::from(vec![
                Span::styled("Source ", Style::default().fg(MUTED)),
                Span::styled(wallpaper, Style::default().fg(TEXT)),
            ]),
            Line::from(vec![
                Span::styled(
                    "Preview only ",
                    Style::default().fg(WARN).add_modifier(Modifier::BOLD),
                ),
                Span::styled("Sixel/Kitty overlay reserved", Style::default().fg(MUTED)),
            ]),
        ];

        frame.render_widget(
            Paragraph::new(Text::from(content))
                .style(Style::default().fg(TEXT))
                .left_aligned(),
            inner,
        );
    }
}

fn fitted_monitor_rect(area: Rect, monitor: &Monitor) -> Rect {
    let usable_width = area.width.saturating_sub(4).max(8);
    let usable_height = area.height.saturating_sub(4).max(4);

    let target_ratio = monitor.aspect_ratio();
    let area_ratio = usable_width as f64 / usable_height.max(1) as f64;

    let (width, height) = if area_ratio > target_ratio {
        let height = usable_height;
        let width = ((height as f64 * target_ratio).round() as u16).clamp(8, usable_width);
        (width, height)
    } else {
        let width = usable_width;
        let height = ((width as f64 / target_ratio).round() as u16).clamp(4, usable_height);
        (width, height)
    };

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

fn inner_rect(area: Rect) -> Rect {
    Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    )
}

fn draw_controls(frame: &mut Frame, area: Rect, app: &App) {
    let chunks =
        Layout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)]).split(area);
    draw_monitors(frame, chunks[0], app);
    draw_scaling_modes(frame, chunks[1], app);
}

fn draw_monitors(frame: &mut Frame, area: Rect, app: &App) {
    let items = app
        .monitors
        .iter()
        .map(|monitor| {
            let label = format!(
                "{} {}x{} {}",
                monitor.name,
                monitor.width,
                monitor.height,
                monitor
                    .wallpaper
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "(none)".to_string())
            );

            ListItem::new(Line::from(vec![Span::styled(
                label,
                Style::default().fg(TEXT),
            )]))
        })
        .collect::<Vec<_>>();

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Monitors ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("selected {}", app.selected_monitor + 1),
                Style::default().fg(MUTED),
            ),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().bg(PANEL));

    let list = List::new(items)
        .block(block)
        .highlight_symbol("▸ ")
        .highlight_style(
            Style::default()
                .fg(TEXT)
                .bg(PANEL_ALT)
                .add_modifier(Modifier::BOLD),
        );

    let mut state = ListState::default();
    state.select(Some(app.selected_monitor));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_scaling_modes(frame: &mut Frame, area: Rect, app: &App) {
    let items = app
        .scaling_modes
        .iter()
        .map(|mode| {
            ListItem::new(Line::from(vec![Span::styled(
                mode.to_string(),
                Style::default().fg(TEXT),
            )]))
        })
        .collect::<Vec<_>>();

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Scaling ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled("current mode", Style::default().fg(MUTED)),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().bg(PANEL));

    let list = List::new(items)
        .block(block)
        .highlight_symbol("▸ ")
        .highlight_style(
            Style::default()
                .fg(TEXT)
                .bg(PANEL_ALT)
                .add_modifier(Modifier::BOLD),
        );

    let mut state = ListState::default();
    state.select(Some(app.selected_scaling_mode));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_logs(frame: &mut Frame, area: Rect, app: &App) {
    let log_items = app
        .logs
        .iter()
        .rev()
        .take(area.height.saturating_sub(2) as usize)
        .map(|log| {
            let style = if log.contains("[ERROR]") {
                Style::default().fg(BAD)
            } else if log.contains("[WARN]") {
                Style::default().fg(WARN)
            } else {
                Style::default().fg(TEXT)
            };

            ListItem::new(Line::from(vec![Span::styled(log.clone(), style)]))
        })
        .collect::<Vec<_>>();

    let list = List::new(log_items).block(
        Block::default()
            .title(Line::from(vec![
                Span::styled(
                    " Logs ",
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::styled("ring buffer", Style::default().fg(MUTED)),
            ]))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(PANEL)),
    );

    frame.render_widget(list, area);
}

fn draw_legend(frame: &mut Frame, area: Rect, app: &App) {
    let lines = vec![
        Line::from(vec![
            Span::styled(
                "[/]",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Search", Style::default().fg(TEXT)),
            Span::raw("  "),
            Span::styled(
                "[f]",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Favorite", Style::default().fg(TEXT)),
        ]),
        Line::from(vec![
            Span::styled(
                "[o]",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Favorites only", Style::default().fg(TEXT)),
            Span::raw("  "),
            Span::styled(
                "[v]",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Toggle Formats", Style::default().fg(TEXT)),
        ]),
        Line::from(vec![
            Span::styled(
                "[s]",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Start Daemon", Style::default().fg(TEXT)),
            Span::raw("  "),
            Span::styled(
                "[Enter]",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Apply Wallpaper", Style::default().fg(TEXT)),
        ]),
        Line::from(vec![
            Span::styled(
                "[Tab]",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Focus Pane", Style::default().fg(TEXT)),
            Span::raw("  "),
            Span::styled("[q]", Style::default().fg(BAD).add_modifier(Modifier::BOLD)),
            Span::styled(" Quit", Style::default().fg(TEXT)),
        ]),
        Line::from(vec![
            Span::styled(
                "[↑/↓ h/j/k/l]",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Navigate lists", Style::default().fg(TEXT)),
        ]),
    ];

    let paragraph = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .title(Line::from(vec![
                    Span::styled(
                        " Keybindings ",
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("mode: {:?}", app.focus), Style::default().fg(MUTED)),
                ]))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER))
                .style(Style::default().bg(PANEL)),
        )
        .style(Style::default().fg(TEXT));

    frame.render_widget(paragraph, area);
}

fn daemon_status_label(status: DaemonStatus) -> &'static str {
    match status {
        DaemonStatus::Running => "Running",
        DaemonStatus::Stopped => "Stopped",
        DaemonStatus::Crashed => "Crashed",
    }
}

fn status_style(status: DaemonStatus) -> Style {
    match status {
        DaemonStatus::Running => Style::default().fg(GOOD).add_modifier(Modifier::BOLD),
        DaemonStatus::Stopped => Style::default().fg(WARN).add_modifier(Modifier::BOLD),
        DaemonStatus::Crashed => Style::default().fg(BAD).add_modifier(Modifier::BOLD),
    }
}
