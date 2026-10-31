//! The view: a pure function of the Model. It reads, never writes, and
//! never does I/O; every color comes from a [`Theme`] role.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Wrap};
use vellum_ipc::RotationSource;

use super::keymap::{self, Action, HelpRow};
use super::layout::{self, Areas};
use super::model::{Daemon, Mode, Model, PreviewImage, PreviewState, Rotation, ToastKind};
use super::theme::Theme;
use super::update::spinner;
use crate::config::{Named as _, format_interval};

#[cfg(test)]
mod tests;

pub fn view(m: &Model, f: &mut Frame) {
    let t = &m.theme;
    f.render_widget(Block::new().style(t.base), f.area());
    let areas = layout::areas(f.area());
    header(m, f, areas.header);
    list(m, f, areas.list);
    preview(m, f, &areas);
    footer(m, f, areas.footer);
    match m.mode {
        Mode::Help => help(m, f),
        Mode::Palette => palette(m, f),
        Mode::Normal | Mode::Search => {}
    }
}

fn header(m: &Model, f: &mut Frame, area: Rect) {
    let t = &m.theme;
    let mut spans = vec![
        Span::styled(" vellum", t.focus.add_modifier(Modifier::BOLD)),
        Span::raw("  "),
    ];
    match &m.daemon {
        Daemon::Connecting => {
            let mark = spinner(m, std::time::Duration::ZERO).unwrap_or(" ");
            spans.push(Span::styled(
                format!("{mark} connecting to vellumd…"),
                t.muted,
            ));
        }
        Daemon::Down(_) => spans.push(Span::styled("✗ vellumd is not running", t.error)),
        Daemon::Up { monitors, .. } => {
            for (i, (_, label)) in layout::monitor_tabs(monitors).into_iter().enumerate() {
                let (mark, style) = if i == m.target {
                    ("●", t.focus.add_modifier(Modifier::BOLD))
                } else {
                    ("○", t.secondary)
                };
                spans.push(Span::styled(format!("{mark} {label}  "), style));
            }
        }
    }
    f.render_widget(Line::from(spans), area);
    let rotation = match m.rotation {
        Rotation::Off => "rotation off ".to_owned(),
        Rotation::On {
            interval_ms,
            source,
        } => {
            let from = match source {
                RotationSource::Library => "",
                RotationSource::Favorites => " · Favorites",
            };
            format!("⟳ every {}{from} ", format_interval(interval_ms))
        }
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

fn pane<'a>(t: &Theme, title: String, focused: bool) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(if focused { t.focus } else { t.muted })
        .title(Span::styled(title, t.text.add_modifier(Modifier::BOLD)))
}

fn list(m: &Model, f: &mut Frame, area: Rect) {
    let t = &m.theme;
    let what = if m.favorites_only {
        "Favorites"
    } else {
        "Library"
    };
    let title = match (m.scanning, m.query.is_empty()) {
        (true, _) => format!(" {what} "),
        (false, true) => format!(" {what} · {} ", m.visible.len()),
        (false, false) => match m.visible.len() {
            1 => format!(" {what} · 1 match "),
            n => format!(" {what} · {n} matches "),
        },
    };
    let mut block = pane(t, title, true);
    if m.mode == Mode::Search || !m.query.is_empty() {
        let (style, cursor) = match m.mode {
            Mode::Search => (t.focus, "▏"),
            _ => (t.secondary, ""),
        };
        block = block.title_bottom(Span::styled(format!(" /{}{cursor} ", m.query), style));
    }
    let inner = block.inner(area);
    f.render_widget(block, area);

    if m.scanning {
        if let Some(mark) = spinner(m, std::time::Duration::ZERO) {
            f.render_widget(
                Paragraph::new(Span::styled(
                    format!("{mark} Scanning the Library…"),
                    t.muted,
                )),
                pad(inner),
            );
        }
        return;
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
        return;
    }
    if m.visible.is_empty() {
        let (what, fix) = if !m.query.is_empty() {
            (format!("No matches for “{}”", m.query), Action::Back)
        } else {
            ("No Favorites yet".to_owned(), Action::Favorite)
        };
        let mut lines = vec![Line::styled(what, t.secondary)];
        if let Some(key) = m.keymap.primary(fix) {
            let how = match fix {
                Action::Favorite => " on a wallpaper to add it",
                _ => " to show everything",
            };
            lines.push(Line::from(vec![
                Span::styled(key, t.focus),
                Span::styled(how, t.secondary),
            ]));
        }
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), pad(inner));
        return;
    }
    let rows = usize::from(inner.height);
    let items: Vec<ListItem> = m
        .visible
        .iter()
        .enumerate()
        .map(|(i, v)| {
            // Only rows on screen need their text built.
            if i < m.list_offset || i >= m.list_offset + rows {
                return ListItem::new("");
            }
            let path = &m.library[v.index];
            let mut spans = highlighted(&m.short_name(path), &v.matched, t.text, t.focus);
            if m.is_favorite(path) {
                spans.push(Span::styled(" ★", t.focus));
            }
            ListItem::new(Line::from(spans))
        })
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

/// `text` as spans, with the characters at `matched` (sorted char
/// indices) in the match style.
fn highlighted(text: &str, matched: &[u32], plain: Style, hit: Style) -> Vec<Span<'static>> {
    let hit = hit.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    let mut spans = Vec::new();
    let mut run = String::new();
    let mut run_hit = false;
    let mut next = matched.iter().peekable();
    for (i, c) in text.chars().enumerate() {
        let is_hit = next.peek().is_some_and(|&&m| m as usize == i);
        if is_hit {
            next.next();
        }
        if is_hit != run_hit && !run.is_empty() {
            spans.push(Span::styled(
                std::mem::take(&mut run),
                if run_hit { hit } else { plain },
            ));
        }
        run_hit = is_hit;
        run.push(c);
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, if run_hit { hit } else { plain }));
    }
    spans
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
        PreviewState::Loading { since, .. } => {
            if let Some(mark) = spinner(m, *since) {
                centered_text(
                    f,
                    area,
                    Line::styled(format!("{mark} Loading preview…"), t.muted),
                );
            }
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
        PreviewState::Ready { image, .. } => {
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
    }
    meta(m, f, areas.meta);
}

fn centered_text(f: &mut Frame, area: Rect, line: Line) {
    let [row] = Layout::vertical([Constraint::Length(1)])
        .flex(Flex::Center)
        .areas(area);
    f.render_widget(line.alignment(Alignment::Center), row);
}

/// The metadata strip: name and folder, then resolution, size, Fit mode
/// and target.
fn meta(m: &Model, f: &mut Frame, area: Rect) {
    let t = &m.theme;
    let Some(path) = m.preview.state.path() else {
        return;
    };
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let folder = path.parent().map(|p| m.tilde(p)).unwrap_or_default();
    let mut first = vec![Span::styled(name, t.text.add_modifier(Modifier::BOLD))];
    if m.is_favorite(path) {
        first.push(Span::styled(" ★", t.focus));
    }
    first.push(Span::raw("  "));
    first.push(Span::styled(folder, t.secondary));
    let mut second = Vec::new();
    if let PreviewState::Ready { meta, .. } = &m.preview.state {
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
    f.render_widget(
        Paragraph::new(vec![Line::from(first), Line::from(second)]),
        pad(area),
    );
}

fn footer(m: &Model, f: &mut Frame, area: Rect) {
    let t = &m.theme;
    for hint in keymap::footer(&m.keymap, m.mode, m.filtered(), area.width) {
        let cell = Rect {
            x: area.x + hint.x,
            width: hint.width.min(area.width.saturating_sub(hint.x)),
            ..area
        };
        f.render_widget(
            Line::from(vec![
                Span::styled(hint.keys, t.focus),
                Span::styled(format!(" {}", hint.label), t.secondary),
            ]),
            cell,
        );
    }
    let right = if let Some(toast) = &m.toast {
        let (mark, style) = match toast.kind {
            ToastKind::Success => ("✓", t.success),
            ToastKind::Error => ("✗", t.error),
        };
        Some(Line::styled(
            format!(" {mark} {} ", toast.text),
            t.panel.patch(style),
        ))
    } else if let Some(busy) = m.applying.first()
        && let Some(mark) = spinner(m, busy.since)
    {
        Some(Line::styled(
            format!(" {mark} {}… ", busy.label),
            t.panel.patch(t.secondary),
        ))
    } else if !m.pending.is_empty() {
        let keys: String = m.pending.iter().map(keymap::Key::display).collect();
        Some(Line::styled(format!(" {keys}… "), t.muted))
    } else {
        None
    };
    if let Some(line) = right {
        right_aligned(f, area, line);
    }
}

/// The box shared by help and the palette, with its prompt drawn.
fn overlay_frame(m: &Model, f: &mut Frame, title: &str, sigil: &str, placeholder: &str) {
    let t = &m.theme;
    let o = layout::overlay(f.area());
    f.render_widget(Clear, o.outer);
    f.render_widget(
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(t.panel.patch(t.focus))
            .style(t.panel)
            .title(Span::styled(
                format!(" {title} "),
                t.panel.patch(t.text).add_modifier(Modifier::BOLD),
            )),
        o.outer,
    );
    let mut prompt = vec![Span::styled(format!("{sigil} "), t.panel.patch(t.focus))];
    if m.prompt.query.is_empty() {
        prompt.push(Span::styled("▏", t.panel.patch(t.focus)));
        prompt.push(Span::styled(placeholder.to_owned(), t.panel.patch(t.muted)));
    } else {
        prompt.push(Span::styled(m.prompt.query.clone(), t.panel.patch(t.text)));
        prompt.push(Span::styled("▏", t.panel.patch(t.focus)));
    }
    f.render_widget(Line::from(prompt), o.prompt);
}

fn help(m: &Model, f: &mut Frame) {
    let t = &m.theme;
    overlay_frame(m, f, "Keys", "/", "type to filter");
    let rows = layout::overlay(f.area()).rows;
    let all = keymap::help_rows(&m.keymap, &m.prompt.query);
    if all.is_empty() {
        f.render_widget(
            Line::styled("Nothing matches", t.panel.patch(t.muted)),
            rows,
        );
        return;
    }
    let keys_width = 14;
    let lines: Vec<Line> = all
        .into_iter()
        .skip(m.prompt.selected)
        .take(usize::from(rows.height))
        .map(|row| match row {
            HelpRow::Section(title) => {
                Line::styled(title, t.panel.patch(t.text).add_modifier(Modifier::BOLD))
            }
            HelpRow::Entry { keys, about } => Line::from(vec![
                Span::styled(format!("  {keys:<keys_width$}"), t.panel.patch(t.focus)),
                Span::styled(about, t.panel.patch(t.secondary)),
            ]),
        })
        .collect();
    f.render_widget(Paragraph::new(lines), rows);
}

fn palette(m: &Model, f: &mut Frame) {
    let t = &m.theme;
    overlay_frame(m, f, "Commands", "›", "type a command");
    let rows = layout::overlay(f.area()).rows;
    let all = keymap::palette_rows(&m.keymap, &m.prompt.query);
    if all.is_empty() {
        f.render_widget(
            Line::styled("No command matches", t.panel.patch(t.muted)),
            rows,
        );
        return;
    }
    let offset = layout::palette_offset(m.prompt.selected, rows.height);
    for (i, row) in all
        .into_iter()
        .enumerate()
        .skip(offset)
        .take(usize::from(rows.height))
    {
        let cell = Rect {
            y: rows.y + u16::try_from(i - offset).unwrap_or(0),
            height: 1,
            ..rows
        };
        let selected = i == m.prompt.selected;
        let base = if selected {
            t.panel.patch(t.selection)
        } else {
            t.panel.patch(t.text)
        };
        let mut spans = vec![Span::styled(if selected { "› " } else { "  " }, base)];
        spans.extend(highlighted(
            row.about,
            &row.matched,
            base,
            base.patch(t.focus),
        ));
        f.render_widget(Line::from(spans).style(base), cell);
        let keys_style = if selected {
            base
        } else {
            t.panel.patch(t.secondary)
        };
        right_aligned(f, cell, Line::styled(format!("{} ", row.keys), keys_style));
    }
}
