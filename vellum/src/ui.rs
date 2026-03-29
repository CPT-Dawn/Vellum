use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, Borders, HighlightSpacing, List, ListItem, ListState, Padding, Paragraph,
};

use crate::app::{App, DaemonStatus, FileEntry, FileKind, Focus, Monitor, ScalingMode};

const fn hex(value: u32) -> Color {
    Color::Rgb(
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    )
}

const PANEL_BORDER: Color = hex(0x3b4261);
const PANEL_BORDER_ACTIVE: Color = hex(0x7dcfff);
const PANEL_GLOW: Color = hex(0x89ddff);
const ACCENT_PRIMARY: Color = hex(0xbb9af7);
const ACCENT_SECONDARY: Color = hex(0x7dcfff);
const GOOD: Color = hex(0x9ece6a);
const WARN: Color = hex(0xe0af68);
const BAD: Color = hex(0xf7768e);
const TEXT_PRIMARY: Color = hex(0xc0caf5);
const TEXT_SECONDARY: Color = hex(0xa9b1d6);
const TEXT_MUTED: Color = hex(0x565f89);
const TEXT_DIM: Color = hex(0x7a88b8);
const HIGHLIGHT_BG: Color = hex(0x2a2f45);
const HIGHLIGHT_BG_SOFT: Color = hex(0x24283b);
const CURSOR_TEXT: Color = hex(0xf5f7ff);
const CELL_ASPECT_COMPENSATION: f64 = 2.0;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let root = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(Color::Reset)),
        root,
    );

    let vertical = Layout::vertical([
        Constraint::Length(5),
        Constraint::Min(12),
        Constraint::Length(4),
    ])
    .spacing(1);
    let [top, middle, bottom] = vertical.areas(root);

    draw_header(frame, top, app);

    let middle_layout = Layout::horizontal([
        Constraint::Percentage(36),
        Constraint::Percentage(24),
        Constraint::Percentage(40),
    ])
    .spacing(1);
    let [browser_area, settings_area, preview_logs_area] = middle_layout.areas(middle);

    draw_browser(frame, browser_area, app, app.focus == Focus::Files);
    draw_settings_panel(frame, settings_area, app, app.focus == Focus::Scaling);
    draw_preview_and_logs(frame, preview_logs_area, app);

    draw_keybinds(frame, bottom, app);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
        .spacing(1)
        .split(area);

    draw_monitor_header(frame, chunks[0], app);
    draw_daemon_header(frame, chunks[1], app);
}

fn draw_monitor_header(frame: &mut Frame, area: Rect, app: &App) {
    let title = Line::from(vec![
        Span::styled(
            " 󰍹 Outputs ",
            Style::default()
                .fg(ACCENT_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("session targets", Style::default().fg(TEXT_MUTED)),
    ]);

    let lines = if app.monitors.is_empty() {
        vec![
            Line::from(vec![Span::styled(
                " 󰖪 No monitors detected",
                Style::default().fg(WARN).add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![
                Span::styled(" 󰆊 Wallpaper ", Style::default().fg(TEXT_MUTED)),
                Span::styled("(none)", Style::default().fg(TEXT_DIM)),
            ]),
        ]
    } else {
        vec![
            Line::from(vec![
                Span::styled(" 󰨈 ", Style::default().fg(TEXT_MUTED)),
                Span::styled(
                    app.selected_monitor_label(),
                    Style::default()
                        .fg(ACCENT_SECONDARY)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    app.selected_monitor_metrics_label(),
                    Style::default().fg(TEXT_SECONDARY),
                ),
            ]),
            Line::from(vec![
                Span::styled(" 󰆊 ", Style::default().fg(TEXT_MUTED)),
                Span::styled(
                    app.selected_wallpaper_label(),
                    Style::default().fg(TEXT_DIM),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("{} output(s)", app.monitors.len()),
                    Style::default().fg(TEXT_MUTED),
                ),
            ]),
        ]
    };

    let paragraph = Paragraph::new(Text::from(lines))
        .block(header_panel_block(title, false))
        .style(Style::default().fg(TEXT_PRIMARY));

    frame.render_widget(paragraph, area);
}

fn draw_daemon_header(frame: &mut Frame, area: Rect, app: &App) {
    let title = Line::from(vec![
        Span::styled(
            " 󰒋 Daemon ",
            Style::default()
                .fg(ACCENT_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("runtime status", Style::default().fg(TEXT_MUTED)),
    ]);

    let paragraph = Paragraph::new(Text::from(vec![
        Line::from(vec![
            Span::styled(
                format!(
                    "{} {}",
                    daemon_status_glyph(app.daemon_status),
                    daemon_status_label(app.daemon_status)
                ),
                status_style(app.daemon_status),
            ),
            Span::raw("  "),
            Span::styled("󰘚 ", Style::default().fg(TEXT_MUTED)),
            Span::styled(
                app.daemon_resource_label(),
                Style::default().fg(ACCENT_SECONDARY),
            ),
        ]),
        Line::from(vec![
            Span::styled(" 󰓦 ", Style::default().fg(TEXT_MUTED)),
            Span::styled(
                app.current_scaling_mode().to_string(),
                Style::default().fg(TEXT_SECONDARY),
            ),
            Span::raw("  "),
            Span::styled("󰌌 ", Style::default().fg(TEXT_MUTED)),
            Span::styled(
                focus_label(app.focus),
                Style::default()
                    .fg(ACCENT_SECONDARY)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ]))
    .block(header_panel_block(title, false))
    .style(Style::default().fg(TEXT_PRIMARY));

    frame.render_widget(paragraph, area);
}

fn draw_browser(frame: &mut Frame, area: Rect, app: &App, active: bool) {
    let items = app
        .visible_browser_items()
        .map(|(_, entry)| {
            let is_applied = app
                .selected_monitor_ref()
                .and_then(|monitor| monitor.wallpaper.as_ref())
                .map(|wallpaper| wallpaper == &entry.path)
                .unwrap_or(false);
            browser_item(entry, is_applied)
        })
        .collect::<Vec<_>>();

    let title = Line::from(vec![
        Span::styled(
            " 󰉋 Workspace ",
            Style::default()
                .fg(ACCENT_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{} items", items.len()),
            Style::default().fg(TEXT_MUTED),
        ),
    ]);

    let block = panel_block(title, active);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [meta_area, list_area] =
        Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(inner);

    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(vec![
                Span::styled(" 󰉖 ", Style::default().fg(TEXT_MUTED)),
                Span::styled(
                    app.current_path.display().to_string(),
                    Style::default().fg(TEXT_SECONDARY),
                ),
            ]),
            browser_filter_line(app),
        ]))
        .style(Style::default().fg(TEXT_SECONDARY)),
        meta_area,
    );

    let list = List::new(items)
        .highlight_style(list_highlight_style(active))
        .highlight_symbol("▸ ")
        .highlight_spacing(HighlightSpacing::Always);

    let mut state = ListState::default();
    state.select(if app.browser_filtered_indices.is_empty() {
        None
    } else {
        Some(app.browser_selected)
    });

    frame.render_stateful_widget(list, list_area, &mut state);
}

fn draw_settings_panel(frame: &mut Frame, area: Rect, app: &App, active: bool) {
    draw_scaling_modes(frame, area, app, active);
}

fn browser_filter_line(app: &App) -> Line<'static> {
    let mut spans = vec![Span::styled(" 󰈲 ", Style::default().fg(TEXT_MUTED))];

    if app.search_active || !app.search_buffer.is_empty() {
        let query = if app.search_buffer.is_empty() {
            "search...".to_string()
        } else {
            app.search_buffer.clone()
        };
        spans.push(Span::styled(
            format!("󰍉 {}", query),
            Style::default()
                .fg(ACCENT_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::styled("󰍉 no query", Style::default().fg(TEXT_MUTED)));
    }

    spans.push(Span::raw("  "));
    spans.push(toggle_badge(" favorites", app.favorites_only, WARN));
    spans.push(Span::raw(" "));
    spans.push(toggle_badge(
        "󰈉 unsupported",
        app.hide_unsupported,
        ACCENT_SECONDARY,
    ));

    Line::from(spans)
}

fn toggle_badge(label: &'static str, enabled: bool, tone: Color) -> Span<'static> {
    if enabled {
        Span::styled(
            format!(" {} ", label),
            Style::default()
                .fg(CURSOR_TEXT)
                .bg(tone)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(format!(" {} ", label), Style::default().fg(TEXT_MUTED))
    }
}

fn list_highlight_style(active: bool) -> Style {
    if active {
        Style::default()
            .fg(CURSOR_TEXT)
            .bg(HIGHLIGHT_BG)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(TEXT_SECONDARY).bg(HIGHLIGHT_BG_SOFT)
    }
}

fn browser_item(entry: &FileEntry, is_applied: bool) -> ListItem<'static> {
    let icon = match entry.kind {
        FileKind::Directory => "󰉋",
        FileKind::File if entry.supported => "󰈔",
        FileKind::File => "󰛉",
    };

    let mut spans = vec![Span::styled(
        format!("{} ", icon),
        Style::default().fg(if entry.kind == FileKind::Directory {
            ACCENT_SECONDARY
        } else {
            TEXT_SECONDARY
        }),
    )];

    if entry.favorite {
        spans.push(Span::styled(
            " ",
            Style::default().fg(WARN).add_modifier(Modifier::BOLD),
        ));
    }

    let label_style = if is_applied {
        Style::default()
            .fg(ACCENT_SECONDARY)
            .add_modifier(Modifier::BOLD)
    } else if entry.favorite {
        Style::default().fg(WARN)
    } else if entry.kind == FileKind::File && !entry.supported {
        Style::default().fg(BAD)
    } else if entry.kind == FileKind::Directory {
        Style::default().fg(TEXT_SECONDARY)
    } else {
        Style::default().fg(TEXT_DIM)
    };

    spans.push(Span::styled(entry.name.clone(), label_style));

    if is_applied {
        spans.push(Span::styled(
            "  󰄬 applied",
            Style::default().fg(GOOD).add_modifier(Modifier::BOLD),
        ));
    } else if entry.kind == FileKind::File && !entry.supported {
        spans.push(Span::styled(
            "  󰅚 unsupported",
            Style::default().fg(TEXT_MUTED),
        ));
    }

    ListItem::new(Line::from(spans))
}

fn draw_preview_and_logs(frame: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::vertical([Constraint::Percentage(62), Constraint::Percentage(38)])
        .spacing(1)
        .split(area);

    draw_preview_with_summary(frame, chunks[0], app, false);
    draw_logs(frame, chunks[1], app);
}

fn draw_preview_with_summary(frame: &mut Frame, area: Rect, app: &mut App, active: bool) {
    let block = panel_block(
        Line::from(vec![
            Span::styled(
                " 󰋩 Live Preview ",
                Style::default()
                    .fg(ACCENT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("monitor framing", Style::default().fg(TEXT_MUTED)),
        ]),
        active,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let preview_summary =
        Layout::vertical([Constraint::Percentage(68), Constraint::Percentage(32)]).split(inner);
    let art_area = preview_summary[0];
    let summary_area = preview_summary[1];

    if let Some(preview) = app
        .selected_monitor_ref()
        .map(|monitor| fitted_monitor_rect(art_area, monitor))
    {
        let stand = monitor_stand_rect(art_area, preview);
        let base = monitor_base_rect(art_area, preview);

        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_set(border::ROUNDED)
                .border_style(Style::default().fg(PANEL_GLOW))
                .style(Style::default().fg(ACCENT_SECONDARY)),
            preview,
        );

        if let Some(stand) = stand {
            render_bar(frame, stand, "▄", PANEL_BORDER_ACTIVE);
        }

        if let Some(base) = base {
            render_bar(frame, base, "▀", PANEL_BORDER);
        }

        let preview_inner = preview_inner_rect(preview);
        if preview_inner.width > 0 && preview_inner.height > 0 {
            app.update_preview_request(preview_inner.width, preview_inner.height);
            if let Some(image) = app.preview_image() {
                draw_halfblock_preview(frame, preview_inner, image);
            } else {
                let placeholder = Paragraph::new(Text::from(vec![Line::from(vec![
                    Span::styled("󰋩 ", Style::default().fg(TEXT_MUTED)),
                    Span::styled(
                        app.preview_status().to_string(),
                        Style::default().fg(TEXT_DIM),
                    ),
                ])]))
                .style(Style::default().fg(TEXT_DIM));
                frame.render_widget(placeholder, preview_inner);
            }
        }
    }

    let wallpaper = app
        .selected_browser_entry()
        .map(|entry| entry.name.clone())
        .unwrap_or_else(|| "No wallpaper selected".to_string());

    let flow = if app.daemon_status == DaemonStatus::Running {
        "daemon online / apply ready"
    } else {
        "daemon unavailable"
    };

    let summary = Paragraph::new(Text::from(vec![
        Line::from(vec![
            Span::styled(" 󰍹 Resolution ", Style::default().fg(TEXT_MUTED)),
            Span::styled(
                app.selected_monitor_metrics_label(),
                Style::default().fg(TEXT_SECONDARY),
            ),
        ]),
        Line::from(vec![
            Span::styled(" 󰆊 Candidate ", Style::default().fg(TEXT_MUTED)),
            Span::styled(wallpaper, Style::default().fg(ACCENT_PRIMARY)),
        ]),
        Line::from(vec![
            Span::styled(" 󰑓 Pipeline ", Style::default().fg(TEXT_MUTED)),
            Span::styled(flow, Style::default().fg(ACCENT_SECONDARY)),
            Span::raw("  "),
            Span::styled("󰓦 ", Style::default().fg(TEXT_MUTED)),
            Span::styled(
                app.current_scaling_mode().to_string(),
                Style::default().fg(TEXT_SECONDARY),
            ),
        ]),
    ]))
    .style(Style::default().fg(TEXT_PRIMARY));

    frame.render_widget(summary, summary_area);
}

fn render_bar(frame: &mut Frame, area: Rect, glyph: &str, color: Color) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let line = glyph.repeat(area.width as usize);
    let rows = (0..area.height)
        .map(|_| Line::from(vec![Span::styled(line.clone(), Style::default().fg(color))]))
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(Text::from(rows)), area);
}

fn preview_inner_rect(preview: Rect) -> Rect {
    Rect::new(
        preview.x.saturating_add(1),
        preview.y.saturating_add(1),
        preview.width.saturating_sub(2),
        preview.height.saturating_sub(2),
    )
}

fn draw_halfblock_preview(frame: &mut Frame, area: Rect, image: &crate::preview::PreviewImage) {
    let cols = area.width.min(image.width);
    let rows = area.height.min(image.height_px / 2);

    if cols == 0 || rows == 0 {
        return;
    }

    let image_width = image.width as usize;
    let mut lines = Vec::with_capacity(rows as usize);

    for y in 0..rows as usize {
        let mut spans = Vec::with_capacity(cols as usize);
        let top_row = y * 2;
        let bottom_row = top_row + 1;

        for x in 0..cols as usize {
            let top_idx = (top_row * image_width + x) * 3;
            let bottom_idx = (bottom_row * image_width + x) * 3;

            let fg = Color::Rgb(
                image.pixels_rgb[top_idx],
                image.pixels_rgb[top_idx + 1],
                image.pixels_rgb[top_idx + 2],
            );
            let bg = Color::Rgb(
                image.pixels_rgb[bottom_idx],
                image.pixels_rgb[bottom_idx + 1],
                image.pixels_rgb[bottom_idx + 2],
            );

            spans.push(Span::styled("▀", Style::default().fg(fg).bg(bg)));
        }

        lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn fitted_monitor_rect(area: Rect, monitor: &Monitor) -> Rect {
    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );

    let usable_width = inner.width.saturating_sub(2).max(8);
    let usable_height = inner.height.saturating_sub(4).max(4);

    let target_ratio = monitor.aspect_ratio() * CELL_ASPECT_COMPENSATION;
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

    let x = inner.x + (inner.width.saturating_sub(width)) / 2;
    let y = inner.y + (inner.height.saturating_sub(height + 2)) / 2;
    Rect::new(x, y, width, height)
}

fn monitor_stand_rect(area: Rect, screen: Rect) -> Option<Rect> {
    let stand_width = (screen.width / 5).clamp(4, screen.width.saturating_sub(2).max(4));
    let stand_x = screen.x + (screen.width.saturating_sub(stand_width)) / 2;
    let stand_y = screen.y.saturating_add(screen.height);

    if stand_y >= area.bottom() {
        return None;
    }

    Some(Rect::new(stand_x, stand_y, stand_width, 1))
}

fn monitor_base_rect(area: Rect, screen: Rect) -> Option<Rect> {
    let stand_y = screen.y.saturating_add(screen.height);
    let base_y = stand_y.saturating_add(1);
    let base_width = (screen.width / 3).clamp(6, screen.width.max(6));
    let base_x = screen.x + (screen.width.saturating_sub(base_width)) / 2;

    if base_y >= area.bottom() {
        return None;
    }

    Some(Rect::new(base_x, base_y, base_width, 1))
}

fn draw_scaling_modes(frame: &mut Frame, area: Rect, app: &App, active: bool) {
    let items = app
        .scaling_modes
        .iter()
        .map(|mode| {
            let is_applied = *mode == app.applied_scaling_mode();
            let style = if is_applied {
                Style::default().fg(GOOD).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(TEXT_DIM)
            };

            let mut spans = vec![Span::styled(
                format!("{} ", scaling_mode_icon(*mode)),
                Style::default().fg(ACCENT_SECONDARY),
            )];
            spans.push(Span::styled(mode.to_string(), style));

            if is_applied {
                spans.push(Span::styled(
                    "  • applied",
                    Style::default().fg(GOOD).add_modifier(Modifier::BOLD),
                ));
            }

            ListItem::new(Line::from(spans))
        })
        .collect::<Vec<_>>();

    let block = panel_block(
        Line::from(vec![
            Span::styled(
                " 󰆞 Scaling Modes ",
                Style::default()
                    .fg(ACCENT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("active strategy", Style::default().fg(TEXT_MUTED)),
        ]),
        active,
    );

    let list = List::new(items)
        .block(block)
        .highlight_symbol("▸ ")
        .highlight_style(list_highlight_style(active))
        .highlight_spacing(HighlightSpacing::Always);

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
            let (icon, style) = if log.contains("[ERROR]") {
                ("󰅙", Style::default().fg(BAD))
            } else if log.contains("[WARN]") {
                ("󰀦", Style::default().fg(WARN))
            } else if log.contains("[SUCCESS]") {
                ("󰄬", Style::default().fg(GOOD))
            } else if log.contains("[ACTION]") {
                ("󰑮", Style::default().fg(ACCENT_SECONDARY))
            } else {
                ("󰌶", Style::default().fg(TEXT_SECONDARY))
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!("{} ", icon), style),
                Span::styled(log.clone(), style),
            ]))
        })
        .collect::<Vec<_>>();

    let list = List::new(log_items).block(panel_block(
        Line::from(vec![
            Span::styled(
                " 󰈚 Activity ",
                Style::default()
                    .fg(ACCENT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("event stream", Style::default().fg(TEXT_MUTED)),
        ]),
        false,
    ));

    frame.render_widget(list, area);
}

fn draw_keybinds(frame: &mut Frame, area: Rect, app: &App) {
    let focus_chip = Span::styled(
        format!(" {} ", focus_label(app.focus)),
        Style::default()
            .fg(CURSOR_TEXT)
            .bg(ACCENT_SECONDARY)
            .add_modifier(Modifier::BOLD),
    );

    let mut lines = vec![Line::from(vec![
        Span::styled(" 󰌌 Active Panel ", Style::default().fg(TEXT_MUTED)),
        focus_chip,
        Span::raw("  "),
        Span::styled(
            format!("󰍉 {}", search_hint_label(app)),
            Style::default().fg(TEXT_DIM),
        ),
    ])];

    lines.push(Line::from(match app.focus {
        Focus::Files => vec![
            key_span("↑/↓ hjkl"),
            label_span(" Navigate files"),
            Span::raw("  "),
            key_span("Tab"),
            label_span(" Next panel"),
            Span::raw("  "),
            key_span("Enter"),
            label_span(" Apply"),
            Span::raw("  "),
            key_span("/"),
            label_span(" Search"),
            Span::raw("  "),
            key_span("f"),
            label_span(" Favorite"),
            Span::raw("  "),
            key_span("1..n"),
            label_span(" Monitors"),
            Span::raw("  "),
            key_span("q"),
            label_span(" Quit"),
        ],
        Focus::Scaling => vec![
            key_span("↑/↓ hjkl"),
            label_span(" Navigate settings"),
            Span::raw("  "),
            key_span("Tab"),
            label_span(" Next panel"),
            Span::raw("  "),
            key_span("[ / ]"),
            label_span(" Scaling"),
            Span::raw("  "),
            key_span("o / v"),
            label_span(" Filters"),
            Span::raw("  "),
            key_span("1..n"),
            label_span(" Monitors"),
            Span::raw("  "),
            key_span("q"),
            label_span(" Quit"),
        ],
    }));

    let paragraph = Paragraph::new(Text::from(lines))
        .block(
            panel_block(
                Line::from(vec![
                    Span::styled(
                        " 󰌌 Interaction ",
                        Style::default()
                            .fg(ACCENT_PRIMARY)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("live controls", Style::default().fg(TEXT_MUTED)),
                ]),
                false,
            )
            .padding(Padding::horizontal(1)),
        )
        .style(Style::default().fg(TEXT_PRIMARY));

    frame.render_widget(paragraph, area);
}

fn search_hint_label(app: &App) -> String {
    if app.search_active {
        if app.search_buffer.is_empty() {
            "type to search".to_string()
        } else {
            app.search_buffer.clone()
        }
    } else {
        "inactive".to_string()
    }
}

fn daemon_status_label(status: DaemonStatus) -> &'static str {
    match status {
        DaemonStatus::Running => "Running",
        DaemonStatus::Stopped => "Stopped",
        DaemonStatus::Crashed => "Crashed",
    }
}

fn daemon_status_glyph(status: DaemonStatus) -> &'static str {
    match status {
        DaemonStatus::Running => "󰐊",
        DaemonStatus::Stopped => "󰓛",
        DaemonStatus::Crashed => "󰅚",
    }
}

fn status_style(status: DaemonStatus) -> Style {
    match status {
        DaemonStatus::Running => Style::default().fg(GOOD).add_modifier(Modifier::BOLD),
        DaemonStatus::Stopped => Style::default().fg(WARN).add_modifier(Modifier::BOLD),
        DaemonStatus::Crashed => Style::default().fg(BAD).add_modifier(Modifier::BOLD),
    }
}

fn focus_label(focus: Focus) -> &'static str {
    match focus {
        Focus::Files => "󰉋 Files",
        Focus::Scaling => "󰆞 Scaling",
    }
}

fn scaling_mode_icon(mode: ScalingMode) -> &'static str {
    match mode {
        ScalingMode::Fill => "󰊠",
        ScalingMode::Fit => "󰉋",
        ScalingMode::Crop => "󰆐",
        ScalingMode::Center => "󰩃",
        ScalingMode::Tile => "󰔉",
    }
}

fn panel_block(title: impl Into<Line<'static>>, active: bool) -> Block<'static> {
    panel_block_with_padding(title, active, Padding::symmetric(2, 1))
}

fn header_panel_block(title: impl Into<Line<'static>>, active: bool) -> Block<'static> {
    panel_block_with_padding(title, active, Padding::symmetric(1, 0))
}

fn panel_block_with_padding(
    title: impl Into<Line<'static>>,
    active: bool,
    padding: Padding,
) -> Block<'static> {
    Block::default()
        .title(title)
        .title_style(
            Style::default()
                .fg(if active { PANEL_GLOW } else { TEXT_MUTED })
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(Style::default().fg(if active {
            PANEL_BORDER_ACTIVE
        } else {
            PANEL_BORDER
        }))
        .style(Style::default().bg(Color::Reset))
        .padding(padding)
}

fn key_span(key: &'static str) -> Span<'static> {
    Span::styled(
        format!("[{}]", key),
        Style::default()
            .fg(ACCENT_SECONDARY)
            .add_modifier(Modifier::BOLD),
    )
}

fn label_span(label: &'static str) -> Span<'static> {
    Span::styled(label, Style::default().fg(TEXT_SECONDARY))
}

#[cfg(test)]
fn monitor_aspect_label(width: u16, height: u16) -> String {
    const COMMON_RATIOS: &[(u32, u32, &str)] = &[
        (32, 9, "32:9"),
        (21, 9, "21:9"),
        (16, 10, "16:10"),
        (16, 9, "16:9"),
        (3, 2, "3:2"),
        (4, 3, "4:3"),
        (5, 4, "5:4"),
        (9, 16, "9:16"),
        (10, 16, "10:16"),
        (2, 3, "2:3"),
        (3, 4, "3:4"),
    ];

    let width = width as u32;
    let height = height as u32;

    for (ratio_width, ratio_height, label) in COMMON_RATIOS {
        if width.saturating_mul(*ratio_height) == height.saturating_mul(*ratio_width) {
            return (*label).to_string();
        }
    }

    let divisor = gcd(width, height);
    format!("{}:{}", width / divisor, height / divisor)
}

#[cfg(test)]
fn gcd(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }

    left.max(1)
}

#[cfg(test)]
mod tests {
    use super::monitor_aspect_label;

    #[test]
    fn recognizes_common_monitor_ratios() {
        assert_eq!(monitor_aspect_label(2560, 1600), "16:10");
        assert_eq!(monitor_aspect_label(1920, 1080), "16:9");
        assert_eq!(monitor_aspect_label(1080, 1920), "9:16");
        assert_eq!(monitor_aspect_label(3440, 1440), "43:18");
        assert_eq!(monitor_aspect_label(1280, 1024), "5:4");
    }
}
