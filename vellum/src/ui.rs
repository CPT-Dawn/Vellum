use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Borders, HighlightSpacing, List, ListItem, ListState, Padding, Paragraph,
};

use crate::app::{App, DaemonStatus, FileEntry, FileKind, Focus, Monitor};

const BORDER: Color = Color::Rgb(70, 76, 98); // muted inactive border
const ACCENT: Color = Color::Rgb(42, 195, 222); // primary accent cyan
const ACCENT_SOFT: Color = Color::Rgb(125, 207, 224); // soft cyan
const PANEL_ACTIVE: Color = Color::Rgb(42, 195, 222); // active panel border
const GOOD: Color = Color::Rgb(158, 206, 106); // #9ece6a
const WARN: Color = Color::Rgb(224, 175, 104); // #e0af68
const BAD: Color = Color::Rgb(247, 118, 142); // #f7768e
const TEXT: Color = Color::Rgb(214, 220, 235);
const MUTED: Color = Color::Rgb(113, 122, 149);
const CURSOR_TEXT: Color = Color::Rgb(245, 247, 250);
const CELL_ASPECT_COMPENSATION: f64 = 2.0;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let root = frame.area();
    frame.render_widget(Block::default(), root);

    let vertical = Layout::vertical([
        Constraint::Percentage(10),
        Constraint::Percentage(80),
        Constraint::Percentage(10),
    ])
    .spacing(1);
    let [top, middle, bottom] = vertical.areas(root);

    draw_header(frame, top, app);

    let middle_layout = Layout::horizontal([
        Constraint::Percentage(34),
        Constraint::Percentage(22),
        Constraint::Percentage(44),
    ])
    .spacing(1);
    let [browser_area, settings_area, preview_logs_area] = middle_layout.areas(middle);

    draw_browser(frame, browser_area, app, app.focus == Focus::Files);
    draw_settings_panel(frame, settings_area, app, app.focus == Focus::Scaling);
    draw_preview_and_logs(frame, preview_logs_area, app);

    draw_keybinds(frame, bottom, app);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)])
        .spacing(1)
        .split(area);

    draw_monitor_header(frame, chunks[0], app);
    draw_daemon_header(frame, chunks[1], app);
}

fn draw_monitor_header(frame: &mut Frame, area: Rect, app: &App) {
    let selector_lines = if app.monitors.is_empty() {
        vec![Line::from(vec![Span::styled(
            " No monitors detected ",
            Style::default().fg(WARN),
        )])]
    } else {
        app.monitors
            .iter()
            .enumerate()
            .map(|(index, monitor)| {
                let is_cursor = index == app.selected_monitor;
                let is_applied = monitor.wallpaper.is_some();
                let indicator = if is_cursor { "> " } else { "  " };
                let style = if is_cursor {
                    Style::default()
                        .fg(CURSOR_TEXT)
                        .add_modifier(Modifier::BOLD)
                } else if is_applied {
                    Style::default().fg(ACCENT)
                } else {
                    Style::default().fg(MUTED)
                };

                Line::from(vec![
                    Span::styled(indicator, style),
                    Span::styled(format!("{}.{}", index + 1, monitor.name), style),
                ])
            })
            .collect::<Vec<_>>()
    };

    let status_line = Line::from(vec![
        Span::styled(" Selected ", Style::default().fg(MUTED)),
        Span::styled(
            app.selected_monitor_label(),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            app.selected_monitor_metrics_label(),
            Style::default().fg(TEXT),
        ),
    ]);

    let wallpaper_line = Line::from(vec![
        Span::styled(" Wallpaper ", Style::default().fg(MUTED)),
        Span::styled(
            app.selected_wallpaper_label(),
            Style::default().fg(ACCENT_SOFT),
        ),
    ]);

    let paragraph = Paragraph::new(Text::from(vec![
        Line::from(vec![
            Span::styled(
                " 󰍹 Monitors ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled("selection", Style::default().fg(MUTED)),
        ]),
        Line::from(vec![Span::styled(
            " Select ",
            Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
        )]),
        selector_lines[0].clone(),
        selector_lines.get(1).cloned().unwrap_or_else(Line::default),
        status_line,
        if app.monitors.len() > 2 {
            Line::from(vec![Span::styled(
                format!(" +{} more monitor(s)", app.monitors.len().saturating_sub(2)),
                Style::default().fg(MUTED),
            )])
        } else {
            wallpaper_line
        },
    ]))
    .block(panel_block("", false))
    .style(Style::default().fg(TEXT));

    frame.render_widget(paragraph, area);
}

fn draw_daemon_header(frame: &mut Frame, area: Rect, app: &App) {
    let paragraph = Paragraph::new(Text::from(vec![
        Line::from(vec![
            Span::styled(
                " 󰒋 Daemon ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                daemon_status_label(app.daemon_status),
                status_style(app.daemon_status),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Resource ", Style::default().fg(MUTED)),
            Span::styled(
                app.daemon_resource_label(),
                Style::default().fg(ACCENT_SOFT),
            ),
        ]),
    ]))
    .block(panel_block("", false))
    .style(Style::default().fg(TEXT));

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

    let block = panel_block(
        Line::from(vec![
            Span::styled(
                " 󰉋 Files / Playlist ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{} items", items.len()), Style::default().fg(MUTED)),
        ]),
        active,
    );

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .fg(CURSOR_TEXT)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ")
        .highlight_spacing(HighlightSpacing::Always);

    let mut state = ListState::default();
    state.select(if app.browser_filtered_indices.is_empty() {
        None
    } else {
        Some(app.browser_selected)
    });

    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_settings_panel(frame: &mut Frame, area: Rect, app: &App, active: bool) {
    draw_scaling_modes(frame, area, app, active);
}

fn browser_item(entry: &FileEntry, is_applied: bool) -> ListItem<'static> {
    let favorite = entry.favorite;
    let icon = match entry.kind {
        FileKind::Directory => "󰉋",
        FileKind::File => "󰈔",
    };
    let support_badge = if entry.kind == FileKind::File && entry.supported {
        "󰋩"
    } else if entry.kind == FileKind::File {
        "󰈥"
    } else {
        "󰉋"
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

    let label_color = if is_applied {
        ACCENT
    } else if favorite {
        WARN
    } else {
        MUTED
    };

    spans.push(Span::styled(
        entry.name.clone(),
        Style::default().fg(label_color),
    ));

    spans.push(Span::styled(
        format!("  [{}]", support_badge),
        Style::default().fg(MUTED),
    ));

    ListItem::new(Line::from(spans))
}

fn draw_preview_and_logs(frame: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::vertical([Constraint::Percentage(56), Constraint::Percentage(44)])
        .spacing(1)
        .split(area);

    draw_preview_with_summary(frame, chunks[0], app, false);
    draw_logs(frame, chunks[1], app);
}

fn draw_preview_with_summary(frame: &mut Frame, area: Rect, app: &mut App, active: bool) {
    let block = panel_block(" 󰋩 Preview + Summary ", active);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let preview_summary =
        Layout::vertical([Constraint::Percentage(70), Constraint::Percentage(30)]).split(inner);
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
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(ACCENT))
                .style(Style::default().fg(ACCENT)),
            preview,
        );

        if let Some(stand) = stand {
            frame.render_widget(
                Block::default().style(Style::default().bg(ACCENT_SOFT)),
                stand,
            );
        }

        if let Some(base) = base {
            frame.render_widget(
                Block::default().style(Style::default().bg(ACCENT_SOFT)),
                base,
            );
        }

        let preview_inner = preview_inner_rect(preview);
        if preview_inner.width > 0 && preview_inner.height > 0 {
            app.update_preview_request(preview_inner.width, preview_inner.height);
            if let Some(image) = app.preview_image() {
                draw_halfblock_preview(frame, preview_inner, image);
            } else {
                let placeholder = Paragraph::new(app.preview_status().to_string())
                    .style(Style::default().fg(MUTED));
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
            Span::styled(" Resolution ", Style::default().fg(MUTED)),
            Span::styled(
                app.selected_monitor_metrics_label(),
                Style::default().fg(TEXT),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Current Wallpaper ", Style::default().fg(MUTED)),
            Span::styled(wallpaper, Style::default().fg(ACCENT)),
        ]),
        Line::from(vec![
            Span::styled(" Flow ", Style::default().fg(MUTED)),
            Span::styled(flow, Style::default().fg(ACCENT_SOFT)),
        ]),
    ]))
    .style(Style::default().fg(TEXT));

    frame.render_widget(summary, summary_area);
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
            let style = if *mode == app.applied_scaling_mode() {
                Style::default().fg(ACCENT)
            } else {
                Style::default().fg(MUTED)
            };

            ListItem::new(Line::from(vec![Span::styled(mode.to_string(), style)]))
        })
        .collect::<Vec<_>>();

    let block = panel_block(
        Line::from(vec![
            Span::styled(
                " 󰆞 Scaling ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled("current mode", Style::default().fg(MUTED)),
        ]),
        active,
    );

    let list = List::new(items)
        .block(block)
        .highlight_symbol("> ")
        .highlight_style(
            Style::default()
                .fg(CURSOR_TEXT)
                .add_modifier(Modifier::BOLD),
        )
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
            let style = if log.contains("[ERROR]") {
                Style::default().fg(BAD)
            } else if log.contains("[WARN]") {
                Style::default().fg(WARN)
            } else if log.contains("[SUCCESS]") {
                Style::default().fg(GOOD)
            } else if log.contains("[ACTION]") {
                Style::default().fg(ACCENT)
            } else {
                Style::default().fg(TEXT)
            };

            ListItem::new(Line::from(vec![Span::styled(log.clone(), style)]))
        })
        .collect::<Vec<_>>();

    let list = List::new(log_items).block(panel_block(" 󰈚 Logs ", false));

    frame.render_widget(list, area);
}

fn draw_keybinds(frame: &mut Frame, area: Rect, app: &App) {
    let lines = vec![Line::from(match app.focus {
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
            key_span("1..n"),
            label_span(" Monitors"),
            Span::raw("  "),
            key_span("q"),
            label_span(" Quit"),
        ],
    })];

    let paragraph = Paragraph::new(Text::from(lines))
        .block(
            panel_block(
                Line::from(vec![
                    Span::styled(
                        " 󰌌 Keys ",
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("active panel", Style::default().fg(MUTED)),
                ]),
                false,
            )
            .padding(Padding::horizontal(1)),
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

fn panel_block(title: impl Into<Line<'static>>, active: bool) -> Block<'static> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if active { PANEL_ACTIVE } else { BORDER }))
        .padding(Padding::symmetric(2, 1))
}

fn key_span(key: &'static str) -> Span<'static> {
    Span::styled(
        format!("[{}]", key),
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )
}

fn label_span(label: &'static str) -> Span<'static> {
    Span::styled(label, Style::default().fg(TEXT))
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
