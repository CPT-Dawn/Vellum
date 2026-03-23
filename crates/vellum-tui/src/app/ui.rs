use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use ratatui_image::StatefulImage;

use crate::app::state::App;
use crate::display::fit_aspect_rect;

pub(crate) fn draw_frame(frame: &mut Frame<'_>, app: &mut App) {
    let frame_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(2)])
        .split(frame.area());

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Percentage(30),
            Constraint::Percentage(40),
        ])
        .split(frame_chunks[0]);

    let mut list_state = ListState::default();
    if !app.files.is_empty() {
        list_state.select(Some(app.selected));
    }

    let browser_items: Vec<ListItem> = if app.files.is_empty() {
        vec![ListItem::new("No image files found")]
    } else {
        app.files
            .iter()
            .map(|path| {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("<invalid utf8>");
                ListItem::new(Line::from(name.to_string()))
            })
            .collect()
    };

    let browser = List::new(browser_items)
        .block(
            Block::default()
                .title("Browser [Vim Motion]")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(app.theme.panel)),
        )
        .highlight_style(
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">>");

    frame.render_stateful_widget(browser, chunks[0], &mut list_state);

    let selected = app.selected_file_name().unwrap_or("None selected");
    let assignments = app.assignments_overview();
    let metadata = Paragraph::new(format!(
        "Root: {}\nTotal images: {}\nCursor: {}\nSelected: {}\nMonitor: {}x{} ({:.2}:1) [{}]\nTarget Output: {}\nScale Mode: {}\nAssignments: {}\nPreview: {}\nDaemon: {}\n\nMode: Normal\nHint: press ? for full keymap",
        app.image_root.display(),
        app.files.len(),
        app.selected,
        selected,
        app.monitor_profile.width,
        app.monitor_profile.height,
        app.monitor_profile.aspect_ratio(),
        app.monitor_profile.source,
        app.current_target_label(),
        app.scale_mode_label(),
        assignments,
        app.preview_info,
        app.daemon_status(),
    ))
    .block(
        Block::default()
            .title("Inspector")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(app.theme.panel)),
    )
    .style(Style::default().fg(app.theme.text));

    frame.render_widget(metadata, chunks[1]);

    let preview_block = Block::default()
        .title("Preview Stage")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.panel));
    let preview_inner = preview_block.inner(chunks[2]);
    frame.render_widget(preview_block, chunks[2]);

    let monitor_rect = fit_aspect_rect(
        preview_inner,
        app.monitor_profile.width,
        app.monitor_profile.height,
    );

    let monitor_block = Block::default()
        .title(format!(
            "Monitor Frame {}x{}",
            app.monitor_profile.width, app.monitor_profile.height
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.accent_alt));

    let monitor_inner = monitor_block.inner(monitor_rect);
    frame.render_widget(monitor_block, monitor_rect);

    if let Some(state) = app.image_state.as_mut() {
        frame.render_stateful_widget(StatefulImage::default(), monitor_inner, state);
    } else {
        let empty =
            Paragraph::new("No preview available").style(Style::default().fg(app.theme.warn));
        frame.render_widget(empty, monitor_inner);
    }

    let status_line = if app.show_help {
        "h/j/k/l move  gg/G top/bottom  Ctrl-u/Ctrl-d page  Enter|Space apply  t cycle-target  s cycle-scale  m monitors  a assignments  x clear  r reload  ? help  q quit"
            .to_string()
    } else {
        app.status.clone()
    };
    let status = Paragraph::new(status_line)
        .style(
            Style::default()
                .bg(app.theme.chrome)
                .fg(app.theme.muted)
                .add_modifier(Modifier::BOLD),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(status, frame_chunks[1]);
}
