//! The view: a pure function of the Model. It reads, never writes, and
//! never does I/O; every color comes from a [`Theme`] role.

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Wrap};
use vellum_ipc::RotationSource;

use super::keymap::{self, Action, HelpRow};
use super::layout::{self, Areas};
use super::model::{
    Daemon, Field, Meta, Mode, Model, PreviewImage, PreviewState, Rotation, Tile, ToastKind,
};
use super::theme::Theme;
use super::update::{self, spinner};
use crate::config::{Named as _, format_interval};

#[cfg(test)]
mod tests;

pub fn view(m: &Model, f: &mut Frame) {
    let t = &m.theme;
    f.render_widget(Block::new().style(t.base), f.area());
    if layout::too_small(f.area()) {
        too_small(m, f);
        return;
    }
    let areas = m.areas();
    header(m, f, areas.header);
    if m.mode == Mode::Welcome {
        welcome(m, f);
    } else if m.grid.on {
        grid(m, f);
    } else {
        if !areas.list.is_empty() {
            list(m, f, areas.list);
        }
        if !areas.preview.is_empty() {
            preview(m, f, &areas);
        }
    }
    footer(m, f, areas.footer);
    match m.mode {
        Mode::Help => help(m, f),
        Mode::Palette => palette(m, f),
        Mode::Rotation => rotation_panel(m, f),
        Mode::Normal | Mode::Search | Mode::Welcome => {}
    }
}

/// The whole screen when the terminal is below [`layout::MIN`].
fn too_small(m: &Model, f: &mut Frame) {
    let t = &m.theme;
    let area = f.area();
    let lines = vec![
        Line::styled("Terminal too small", t.text.add_modifier(Modifier::BOLD)),
        Line::styled(
            format!(
                "{}×{}, needs {}×{}",
                area.width,
                area.height,
                layout::MIN.width,
                layout::MIN.height
            ),
            t.secondary,
        ),
    ];
    let [row] = Layout::vertical([Constraint::Length(2)])
        .flex(Flex::Center)
        .areas(area);
    f.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        row,
    );
}

/// `key` in the focus role, then `text`: `o choose a folder`.
fn key_hint<'a>(t: &Theme, key: Option<String>, text: &'a str) -> Option<Line<'a>> {
    Some(Line::from(vec![
        Span::styled(key?, t.focus),
        Span::styled(format!(" {text}"), t.secondary),
    ]))
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
        Daemon::Down(_) => {
            spans.push(Span::styled("✗ vellumd is not running", t.error));
            if let Some(hint) = key_hint(t, m.keymap.primary(Action::Reconnect), "reconnect") {
                spans.push(Span::raw("  "));
                spans.extend(hint.spans);
            }
        }
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
    let left = Line::from(spans);
    let used = left.width();
    f.render_widget(left, area);
    let rotation = match m.rotation {
        Rotation::Off => "rotation off ".to_owned(),
        Rotation::On {
            interval_ms,
            source,
            overridden,
        } => {
            let from = match source {
                RotationSource::Library => "",
                RotationSource::Favorites => " · Favorites",
            };
            let own = if overridden { " · Override" } else { "" };
            format!("⟳ every {}{from}{own} ", format_interval(interval_ms))
        }
    };
    // Only where it fits beside the Monitors.
    if used + rotation.chars().count() < usize::from(area.width) {
        right_aligned(f, area, Line::styled(rotation, t.secondary));
    }
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

/// The Library pane's frame: what it shows and how many, and the search
/// query along the bottom.
fn library_block<'a>(m: &Model) -> Block<'a> {
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
    block
}

/// Why the list is empty and the key that fixes it: an empty Library,
/// no matches, no Favorites. `None` while scanning or when there is
/// something to show.
fn empty_state(m: &Model) -> Option<Vec<Line<'static>>> {
    let t = &m.theme;
    if m.scanning || !m.visible.is_empty() {
        return None;
    }
    let title = |text: String| Line::styled(text, t.text.add_modifier(Modifier::BOLD));
    let mut lines = Vec::new();
    if m.library.is_empty() {
        let folders: Vec<String> = m.folders.iter().map(|p| m.tilde(p)).collect();
        lines.push(title("No wallpapers yet".into()));
        lines.push(Line::styled(
            format!("Nothing to show in {}.", folders.join(", ")),
            t.secondary,
        ));
        lines.push(Line::styled(
            "Add JPEG, PNG or WebP images there.",
            t.secondary,
        ));
        lines.extend(key_hint(
            t,
            m.keymap.primary(Action::Library),
            "choose another folder",
        ));
    } else if !m.query.is_empty() {
        lines.push(title(format!("No matches for “{}”", m.query)));
        let clear = match m.mode {
            Mode::Search => Action::Cancel,
            _ => Action::Back,
        };
        lines.extend(key_hint(t, m.keymap.primary(clear), "clear the search"));
    } else {
        lines.push(title("No Favorites yet".into()));
        lines.extend(key_hint(
            t,
            m.keymap.primary(Action::Favorite),
            "on a wallpaper adds it",
        ));
        lines.extend(key_hint(
            t,
            m.keymap.primary(Action::FavoritesOnly),
            "show everything",
        ));
    }
    Some(lines)
}

/// Draws `lines` centered in `area`.
fn centered_lines(f: &mut Frame, area: Rect, lines: Vec<Line>) {
    let height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let [row] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    f.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        row,
    );
}

/// Says why there is nothing to show, if so: still scanning, an empty
/// Library, no matches, no Favorites. True if it did. Beside a preview
/// pane only a word goes here and the pane says the rest.
fn no_wallpapers(m: &Model, f: &mut Frame, inner: Rect, beside_preview: bool) -> bool {
    let t = &m.theme;
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
        return true;
    }
    let Some(lines) = empty_state(m) else {
        return false;
    };
    if beside_preview {
        let word = if m.library.is_empty() {
            "Empty"
        } else {
            "None"
        };
        f.render_widget(Line::styled(word, t.muted), pad(inner));
    } else {
        centered_lines(f, inner, lines);
    }
    true
}

/// The thumbnail grid: one tile per wallpaper on screen, the selection's
/// name highlighted, and the metadata strip below.
fn grid(m: &Model, f: &mut Frame) {
    let t = &m.theme;
    let areas = layout::grid(f.area());
    let block = library_block(m);
    let inner = block.inner(areas.pane);
    f.render_widget(block, areas.pane);
    if no_wallpapers(m, f, inner, false) {
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
        let mut label = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        if m.is_favorite(path) {
            label += " ★";
        }
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
    let block = library_block(m);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let beside_preview = !m.areas().preview.is_empty();
    if no_wallpapers(m, f, inner, beside_preview) {
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

/// Why a halfblock preview looks coarse, on the preview pane's border.
const COARSE_HINT: &str = " coarse: terminal shows no images ";

fn preview(m: &Model, f: &mut Frame, areas: &Areas) {
    let t = &m.theme;
    let narrow = areas.list.is_empty();
    let mut block = pane(t, " Preview ".into(), narrow);
    if m.coarse && usize::from(areas.preview.width) >= COARSE_HINT.chars().count() + 12 {
        block = block.title(Line::styled(COARSE_HINT, t.muted).right_aligned());
    }
    f.render_widget(block, areas.preview);
    let area = areas.image;
    match &m.preview.state {
        PreviewState::Empty => {
            if let Some(lines) = empty_state(m) {
                centered_lines(
                    f,
                    areas.preview.inner(ratatui::layout::Margin::new(2, 1)),
                    lines,
                );
            }
        }
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
            let moves = format!(
                "{}/{}",
                m.keymap.primary(Action::Down).unwrap_or_default(),
                m.keymap.primary(Action::Up).unwrap_or_default()
            );
            let mut lines = vec![
                Line::styled("✗ Cannot preview this wallpaper", t.error),
                Line::styled(error.as_str(), t.secondary),
                Line::raw(""),
                Line::styled("The file may be damaged or not an image.", t.secondary),
            ];
            lines.extend(key_hint(t, Some(moves), "pick another wallpaper"));
            centered_lines(f, area, lines);
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
    let mut first = vec![Span::styled(name, t.text.add_modifier(Modifier::BOLD))];
    if m.is_favorite(path) {
        first.push(Span::styled(" ★", t.focus));
    }
    first.push(Span::raw("  "));
    first.push(Span::styled(folder, t.secondary));
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
    f.render_widget(
        Paragraph::new(vec![Line::from(first), Line::from(second)]),
        pad(area),
    );
}

fn footer(m: &Model, f: &mut Frame, area: Rect) {
    let t = &m.theme;
    for hint in keymap::footer(&m.keymap, m.mode, m.footer_context(), area.width) {
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

/// The welcome screen, in place of the list and preview: where the
/// Library is, `~/Pictures`, or a typed folder with Tab completion.
fn welcome(m: &Model, f: &mut Frame) {
    let t = &m.theme;
    let a = m.areas();
    let body = Rect {
        y: a.header.bottom(),
        height: a.footer.y.saturating_sub(a.header.bottom()),
        ..f.area()
    };
    let w = &m.welcome;
    let folders: Vec<String> = m.folders.iter().map(|p| m.tilde(p)).collect();
    let folders = match folders.as_slice() {
        [] => "no folder yet".to_owned(),
        _ => folders.join(", "),
    };
    let first_run = m.library.is_empty();
    let mut lines = vec![
        Line::styled(
            if first_run {
                "Welcome to vellum"
            } else {
                "Add a Library folder"
            },
            t.focus.add_modifier(Modifier::BOLD),
        ),
        Line::styled("Wallpapers, weightless.", t.secondary),
        Line::raw(""),
        Line::styled(format!("Vellum looks for wallpapers in {folders}."), t.text),
        Line::styled(
            if first_run {
                "Nothing is there yet. Where do yours live?"
            } else {
                "Pick another folder to add to it."
            },
            t.text,
        ),
        Line::raw(""),
    ];
    let mark = |i: usize| {
        if w.choice == i {
            ("› ", t.selection)
        } else {
            ("  ", t.text)
        }
    };
    let (m0, s0) = mark(0);
    lines.push(Line::from(vec![
        Span::styled(m0, s0),
        Span::styled("Use ~/Pictures", s0),
    ]));
    let (m1, s1) = mark(1);
    let mut typed = vec![
        Span::styled(m1, s1),
        Span::styled("Another folder ", s1),
        Span::raw(" "),
    ];
    if w.input.is_empty() {
        typed.push(Span::styled("▏", t.focus));
        typed.push(Span::styled("type a path, like ~/Wallpapers", t.muted));
    } else {
        typed.push(Span::styled(w.input.clone(), t.text));
        typed.push(Span::styled("▏", t.focus));
    }
    lines.push(Line::from(typed));
    if !w.candidates.is_empty() {
        lines.push(Line::styled(
            format!("    {}", w.candidates.join("  ")),
            t.secondary,
        ));
    }
    lines.push(Line::raw(""));
    if let Some(error) = &w.error {
        lines.push(Line::styled(format!("✗ {error}"), t.error));
    } else if w.saving {
        lines.push(Line::styled("Saving…", t.muted));
    } else {
        lines.push(Line::styled("Your choice is saved to the Config.", t.muted));
    }
    let width = body.width.saturating_sub(4).min(64);
    let height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let [row] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(body);
    let [cell] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(row);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), cell);
}

/// The Rotation panel over the body: running, scope, interval, source and
/// order, the highlighted line's value between ‹ ›.
fn rotation_panel(m: &Model, f: &mut Frame) {
    let t = &m.theme;
    let (outer, rows) = layout::panel(f.area());
    f.render_widget(Clear, outer);
    f.render_widget(
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(t.panel.patch(t.focus))
            .style(t.panel)
            .title(Span::styled(
                " Rotation ",
                t.panel.patch(t.text).add_modifier(Modifier::BOLD),
            )),
        outer,
    );
    let schedule = update::panel_schedule(m);
    let running = update::rotation_running(m);
    let label_width = 12;
    let mut lines = Vec::new();
    for (i, field) in update::panel_fields(m).into_iter().enumerate() {
        let (label, value) = match field {
            Field::Running => (
                "Rotation",
                if running {
                    "● running"
                } else {
                    "○ stopped"
                }
                .to_owned(),
            ),
            Field::Scope => (
                "Applies to",
                match &m.panel.monitor {
                    None => "every monitor".to_owned(),
                    Some(name) if update::has_override(m) => format!("{name} (Override)"),
                    Some(name) => format!("{name} (no Override yet)"),
                },
            ),
            Field::Interval => (
                "Interval",
                format!("every {}", format_interval(schedule.interval_ms)),
            ),
            Field::Source => (
                "Source",
                match schedule.source {
                    RotationSource::Library => "Library".to_owned(),
                    RotationSource::Favorites => "Favorites".to_owned(),
                },
            ),
            Field::Order => (
                "Order",
                match schedule.order {
                    vellum_ipc::RotationOrder::Shuffle => "shuffle".to_owned(),
                    vellum_ipc::RotationOrder::Sequential => "sequential".to_owned(),
                },
            ),
        };
        let selected = i == m.panel.row;
        let (mark, label_style, value_style) = if selected {
            ("› ", t.panel.patch(t.selection), t.panel.patch(t.focus))
        } else {
            ("  ", t.panel.patch(t.secondary), t.panel.patch(t.text))
        };
        let value = if selected && field != Field::Running {
            format!("‹ {value} ›")
        } else {
            value
        };
        lines.push(Line::from(vec![
            Span::styled(mark, label_style),
            Span::styled(format!("{label:<label_width$}"), label_style),
            Span::styled(value, value_style),
        ]));
    }
    lines.push(Line::raw(""));
    let note = match &m.panel.monitor {
        Some(_) if update::has_override(m) => key_hint(
            t,
            m.keymap.primary(Action::RemoveOverride),
            "use the global Rotation here",
        ),
        _ => None,
    }
    .unwrap_or_else(|| Line::styled("Changes are saved to the Config.", t.muted));
    lines.push(note.patch_style(t.panel));
    f.render_widget(Paragraph::new(lines), rows);
}
