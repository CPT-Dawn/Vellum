//! The view: a pure function of the Model. It reads, never writes, and
//! never does I/O; every color comes from a [`Theme`] role.

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, List, ListItem, ListState, Paragraph, Wrap};

use super::keymap;
use super::layout::{self, Areas};
use super::model::{Daemon, Meta, Model, PreviewImage, PreviewState, Rotation, Tile, ToastKind};
use super::theme::Theme;
use crate::config::Named as _;

#[cfg(test)]
mod tests;

pub fn view(m: &Model, f: &mut Frame) {
    let t = &m.theme;
    f.render_widget(Block::new().style(t.base), f.area());
    let areas = layout::areas(f.area());
    header(m, f, areas.header);
    if m.grid.on {
        grid(m, f);
    } else {
        list(m, f, areas.list);
        preview(m, f, &areas);
    }
    footer(m, f, areas.footer);
}

fn header(m: &Model, f: &mut Frame, area: Rect) {
    let t = &m.theme;
    let mut spans = vec![
        Span::styled(" vellum", t.focus.add_modifier(Modifier::BOLD)),
        Span::raw("  "),
    ];
    match &m.daemon {
        Daemon::Connecting => spans.push(Span::styled("connecting to vellumd…", t.muted)),
        Daemon::Down(_) => spans.push(Span::styled("✗ vellumd is not running", t.error)),
        Daemon::Up { monitors, .. } => {
            for (i, monitor) in monitors.iter().enumerate() {
                let label = format!(
                    "{} {} {}×{}  ",
                    if i == m.target { "●" } else { "○" },
                    monitor.name,
                    monitor.physical_width,
                    monitor.physical_height
                );
                let style = if i == m.target {
                    t.focus.add_modifier(Modifier::BOLD)
                } else {
                    t.secondary
                };
                spans.push(Span::styled(label, style));
            }
        }
    }
    f.render_widget(Line::from(spans), area);
    let rotation = match m.rotation {
        Rotation::Off => "rotation off ".to_owned(),
        Rotation::On { interval_secs } => format!("rotation every {} ", duration(interval_secs)),
    };
    right_aligned(f, area, Line::styled(rotation, t.secondary));
}

/// Draws `line` at the right end of `area`, leaving the rest untouched.
fn right_aligned(f: &mut Frame, area: Rect, line: Line) {
    let width = u16::try_from(line.width())
        .unwrap_or(u16::MAX)
        .min(area.width);
    let [cell] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::End)
        .areas(area);
    f.render_widget(line, cell);
}

/// "30 min", "2 h", "45 s".
fn duration(secs: u64) -> String {
    match secs {
        s if s % 3600 == 0 && s > 0 => format!("{} h", s / 3600),
        s if s % 60 == 0 && s > 0 => format!("{} min", s / 60),
        s => format!("{s} s"),
    }
}

fn pane<'a>(t: &Theme, title: String, focused: bool) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(if focused { t.focus } else { t.muted })
        .title(Span::styled(title, t.text.add_modifier(Modifier::BOLD)))
}

fn library_title(m: &Model) -> String {
    if m.scanning {
        " Library ".to_owned()
    } else {
        format!(" Library · {} ", m.library.len())
    }
}

/// Says why there is nothing to show yet, if so. True if it did.
fn no_wallpapers(m: &Model, f: &mut Frame, inner: Rect) -> bool {
    let t = &m.theme;
    if m.scanning {
        f.render_widget(
            Paragraph::new(Span::styled("Scanning the Library…", t.muted)),
            pad(inner),
        );
        return true;
    }
    if m.library.is_empty() {
        let folders: Vec<String> = m.folders.iter().map(|p| m.tilde(p)).collect();
        f.render_widget(
            Paragraph::new(vec![
                Line::styled("No wallpapers found in", t.secondary),
                Line::styled(folders.join(", "), t.text),
            ])
            .wrap(Wrap { trim: true }),
            pad(inner),
        );
        return true;
    }
    false
}

/// The thumbnail grid: one tile per wallpaper on screen, the selection's
/// name highlighted, and the metadata strip below.
fn grid(m: &Model, f: &mut Frame) {
    let t = &m.theme;
    let areas = layout::grid(f.area());
    let block = pane(t, library_title(m), true);
    let inner = block.inner(areas.pane);
    f.render_widget(block, areas.pane);
    if no_wallpapers(m, f, inner) {
        return;
    }
    let first = m.grid.offset_row * areas.cols;
    for (i, path) in super::update::visible(m).enumerate() {
        let (image, name) = areas.tile(i);
        match m.grid.tiles.get(path) {
            Some(Tile::Ready { image: thumb, .. }) => draw_image(m, f, image, thumb),
            Some(Tile::Failed) => centered_text(f, image, Line::styled("✗", t.error)),
            Some(Tile::Loading) | None => centered_text(f, image, Line::styled("…", t.muted)),
        }
        let selected = first + i == m.selected;
        let label = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let (mark, style) = if selected {
            ("› ", t.selection)
        } else {
            ("  ", t.secondary)
        };
        f.render_widget(Line::styled(format!("{mark}{label}"), style), name);
    }
    let meta = match m.selection().and_then(|p| m.grid.tiles.get(p)) {
        Some(Tile::Ready { meta, .. }) => Some(*meta),
        _ => None,
    };
    if let Some(path) = m.selection() {
        meta_strip(m, f, areas.meta, path, meta);
    }
}

fn list(m: &Model, f: &mut Frame, area: Rect) {
    let t = &m.theme;
    let block = pane(t, library_title(m), true);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if no_wallpapers(m, f, inner) {
        return;
    }
    let items: Vec<ListItem> = m
        .library
        .iter()
        .map(|path| ListItem::new(Span::styled(short_name(m, path), t.text)))
        .collect();
    let mut state = ListState::default()
        .with_offset(m.list_offset)
        .with_selected(Some(m.selected));
    f.render_stateful_widget(
        List::new(items)
            .highlight_style(t.selection)
            .highlight_symbol("› "),
        inner,
        &mut state,
    );
}

/// `path` relative to the Library folder holding it: `beach/dunes.jpg`.
fn short_name(m: &Model, path: &Path) -> String {
    m.folders
        .iter()
        .find_map(|folder| path.strip_prefix(folder).ok())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// One column of breathing room on the left.
fn pad(area: Rect) -> Rect {
    area.inner(ratatui::layout::Margin::new(1, 0))
}

fn preview(m: &Model, f: &mut Frame, areas: &Areas) {
    let t = &m.theme;
    f.render_widget(pane(t, " Preview ".into(), false), areas.preview);
    let area = areas.image;
    match &m.preview.state {
        PreviewState::Empty => {}
        PreviewState::Loading { .. } => {
            centered_text(f, area, Line::styled("Loading preview…", t.muted));
        }
        PreviewState::Failed { error, .. } => {
            let [row] = Layout::vertical([Constraint::Length(3)])
                .flex(Flex::Center)
                .areas(area);
            f.render_widget(
                Paragraph::new(vec![
                    Line::styled("✗ Cannot preview this wallpaper", t.error),
                    Line::styled(error.as_str(), t.secondary),
                ])
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
                row,
            );
        }
        PreviewState::Ready { image, .. } => draw_image(m, f, area, image),
    }
    if let Some(path) = m.preview.state.path() {
        let meta = match &m.preview.state {
            PreviewState::Ready { meta, .. } => Some(*meta),
            _ => None,
        };
        meta_strip(m, f, areas.meta, path, meta);
    }
}

/// Draws `image` centered in `area`.
fn draw_image(m: &Model, f: &mut Frame, area: Rect, image: &PreviewImage) {
    let t = &m.theme;
    let size = image.size();
    let [row] = Layout::vertical([Constraint::Length(size.height)])
        .flex(Flex::Center)
        .areas(area);
    let [cell] = Layout::horizontal([Constraint::Length(size.width)])
        .flex(Flex::Center)
        .areas(row);
    match image {
        PreviewImage::Protocol(protocol) => {
            f.render_widget(ratatui_image::Image::new(protocol), cell);
        }
        PreviewImage::Placeholder { label, .. } => {
            let fill = "░".repeat(usize::from(cell.width));
            let lines = vec![Line::styled(fill, t.muted); usize::from(cell.height)];
            f.render_widget(Paragraph::new(lines), cell);
            centered_text(f, cell, Line::styled(label.as_str(), t.text));
        }
    }
}

fn centered_text(f: &mut Frame, area: Rect, line: Line) {
    let [row] = Layout::vertical([Constraint::Length(1)])
        .flex(Flex::Center)
        .areas(area);
    f.render_widget(line.alignment(Alignment::Center), row);
}

/// The metadata strip: name and folder, then resolution, size, Fit mode
/// and target.
fn meta_strip(m: &Model, f: &mut Frame, area: Rect, path: &Path, meta: Option<Meta>) {
    let t = &m.theme;
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let folder = path.parent().map(|p| m.tilde(p)).unwrap_or_default();
    let first = Line::from(vec![
        Span::styled(name, t.text.add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(folder, t.secondary),
    ]);
    let mut second = Vec::new();
    if let Some(meta) = meta {
        second.push(Span::styled(
            format!(
                "{}×{} · {} · ",
                meta.width,
                meta.height,
                crate::status::memory(meta.bytes)
            ),
            t.secondary,
        ));
    }
    second.push(Span::styled(m.fit.name(), t.text));
    if let Some(monitor) = m.targeted() {
        second.push(Span::styled(" on ", t.secondary));
        second.push(Span::styled(monitor.name.as_str(), t.text));
    }
    f.render_widget(Paragraph::new(vec![first, Line::from(second)]), pad(area));
}

fn footer(m: &Model, f: &mut Frame, area: Rect) {
    let t = &m.theme;
    let mut spans = vec![Span::raw(" ")];
    for (keys, label) in keymap::hints(m.mode, m.grid.on) {
        spans.push(Span::styled(*keys, t.focus));
        spans.push(Span::styled(format!(" {label}  "), t.secondary));
    }
    f.render_widget(Line::from(spans), area);
    if let Some(toast) = &m.toast {
        let (mark, style) = match toast.kind {
            ToastKind::Success => ("✓", t.success),
            ToastKind::Error => ("✗", t.error),
        };
        let text = format!(" {mark} {} ", toast.text);
        right_aligned(f, area, Line::styled(text, t.panel.patch(style)));
    }
}
