//! The update function: the only place the Model changes.
//!
//! `update(&mut Model, Msg) -> Vec<Cmd>` is pure: no I/O, no threads, and
//! no clock (the runtime writes the time into `Model::now`). Keys go
//! through the [`Keymap`](super::keymap::Keymap) to an [`Action`], mouse
//! events are hit-tested against [`layout`], and [`perform`] turns an
//! Action into Model changes and Cmds.
//!
//! Two kinds of timer:
//! - Debounces. Moving bumps the preview generation at once (so workers
//!   drop what they were doing for the old selection) and asks the runtime
//!   for a [`Wake`] after [`DEBOUNCE`]. Only if nothing newer came by then
//!   does update ask for the render, plus prefetches of the neighbours.
//!   Resizes, typing a search and grid scrolling work the same way.
//! - What the screen shows over time: toasts expiring, spinners, a key
//!   sequence timing out. [`wake_at`] derives the next one from the Model
//!   and the runtime sends [`Msg::Tick`] then.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Margin, Position, Rect};

use super::keymap::{self, Action, CHORD_TIMEOUT, Key, Lookup, Scope};
use super::layout;
use super::model::{
    Busy, Daemon, Mode, Model, PreviewState, Prompt, Rotation, Tile, TileSpec, Toast, ToastKind,
};
use super::msg::{ApplyJob, Cmd, Msg, PreviewJob, Purpose, Screen, Wake};

#[cfg(test)]
mod tests;

/// How long the selection (or the terminal size) must stay put before
/// anything is rendered for it.
pub const DEBOUNCE: Duration = Duration::from_millis(80);
/// Neighbours of the selection to prefetch, nearest first.
const PREFETCH: [isize; 4] = [1, -1, 2, -2];
/// Slow work shows a spinner only once it has taken this long.
pub const SPINNER_DELAY: Duration = Duration::from_millis(150);
/// Time per spinner frame.
pub const SPINNER_FRAME: Duration = Duration::from_millis(80);
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// How long a confirmation stays up.
pub const TOAST_OK: Duration = Duration::from_secs(3);
/// How long an error stays up.
pub const TOAST_ERROR: Duration = Duration::from_secs(6);
/// Two clicks on one wallpaper within this are a double-click.
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// Whether to render right away or after [`DEBOUNCE`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum When {
    Now,
    Soon,
}

/// The Cmds to run at startup: scan the Library, ask the Daemon and read
/// the Favorites, in parallel, so the list shows up without waiting.
pub fn init(model: &Model) -> Vec<Cmd> {
    vec![
        Cmd::ScanLibrary(model.folders.clone()),
        Cmd::QueryDaemon,
        Cmd::LoadFavorites,
    ]
}

pub fn update(m: &mut Model, msg: Msg) -> Vec<Cmd> {
    m.dirty = true;
    match msg {
        Msg::Key(event) => {
            // A key only redraws if its Action changes something (Actions
            // set `dirty` themselves).
            m.dirty = false;
            key(m, event)
        }
        Msg::Mouse(event) => {
            m.dirty = false;
            mouse(m, event)
        }
        Msg::Tick => {
            m.dirty = false;
            tick(m)
        }
        Msg::Resize { width, height } => {
            m.area = Rect::new(0, 0, width, height);
            scroll_to_selection(m);
            refresh(m, When::Soon)
        }
        Msg::LibraryScanned(library) => {
            let names: Vec<String> = library.iter().map(|p| m.short_name(p)).collect();
            m.index = super::search::Index::new(names.iter().map(String::as_str));
            m.library = library;
            m.scanning = false;
            refilter(m, Keep::Selection, When::Now)
        }
        Msg::Daemon(Ok(info)) => {
            m.target = m.target.min(info.monitors.len().saturating_sub(1));
            m.daemon = Daemon::Up {
                monitors: info.monitors,
            };
            m.status = Some(info.status);
            retarget(m);
            refresh(m, When::Now)
        }
        Msg::Daemon(Err(err)) => {
            m.daemon = Daemon::Down(err);
            Vec::new()
        }
        Msg::Woke(wake) => {
            // Waking changes nothing on screen by itself.
            m.dirty = false;
            match wake {
                Wake::Preview { generation } if generation == m.preview.generation => {
                    preview_jobs(m)
                }
                Wake::Grid { generation } if generation == m.grid.generation => tile_jobs(m),
                Wake::Preview { .. } | Wake::Grid { .. } => Vec::new(),
            }
        }
        Msg::PreviewDone { generation, .. } if generation != m.preview.generation => {
            m.dirty = false;
            Vec::new()
        }
        Msg::PreviewDone { path, result, .. } => {
            m.preview.state = match result {
                Ok(out) => PreviewState::Ready {
                    path,
                    image: out.image,
                    meta: out.meta,
                },
                Err(error) => PreviewState::Failed { path, error },
            };
            Vec::new()
        }
        Msg::ThumbDone { generation, .. } if generation != m.grid.generation => {
            m.dirty = false;
            Vec::new()
        }
        Msg::ThumbDone { path, result, .. } => {
            m.dirty = m.grid.on && visible(m).any(|p| *p == path);
            if let Some(tile) = m.grid.tiles.get_mut(&path) {
                *tile = match result {
                    Ok(out) => Tile::Ready {
                        image: out.image,
                        meta: out.meta,
                    },
                    Err(_) => Tile::Failed,
                };
            }
            Vec::new()
        }
        Msg::Applied { monitor, result } => {
            let label = applying_label(monitor.as_deref());
            if let Some(i) = m.applying.iter().position(|b| b.label == label) {
                m.applying.remove(i);
            }
            match result {
                Ok(()) => {
                    let text = match monitor {
                        Some(name) => format!("Applied to {name}"),
                        None => "Applied to every monitor".into(),
                    };
                    toast(m, ToastKind::Success, text);
                }
                Err(err) => toast(m, ToastKind::Error, err),
            }
            Vec::new()
        }
        Msg::FavoritesLoaded(favorites) => {
            m.favorites = favorites;
            if m.favorites_only {
                return refilter(m, Keep::Selection, When::Now);
            }
            Vec::new()
        }
        Msg::FavoriteSaved {
            path,
            favorite,
            result,
        } => favorite_saved(m, path, favorite, result),
        Msg::HookFailed(why) => {
            toast(m, ToastKind::Error, why);
            Vec::new()
        }
    }
}

/// When the runtime should send the next [`Msg::Tick`], as a time since
/// start (compare with `Model::now`), or `None` to sleep until something
/// else happens. Only a visible toast, a spinner, or a key sequence that
/// can time out ask for one.
pub fn wake_at(m: &Model) -> Option<Duration> {
    let mut at: Option<Duration> = None;
    let mut wake = |t: Duration| at = Some(at.map_or(t, |a| a.min(t)));
    if m.toast.is_some() {
        wake(m.toast_until);
    }
    if !m.pending.is_empty()
        && matches!(
            m.keymap.lookup(Scope::of(m.mode), &m.pending),
            Lookup::Prefix { alone: Some(_) }
        )
    {
        wake(m.pending_since + CHORD_TIMEOUT);
    }
    for since in busy(m) {
        let start = since + SPINNER_DELAY;
        if m.now < start {
            wake(start);
        } else {
            let frames = (m.now - start).as_millis() / SPINNER_FRAME.as_millis() + 1;
            wake(start + SPINNER_FRAME * u32::try_from(frames).unwrap_or(u32::MAX));
        }
    }
    at
}

/// When each piece of slow work in flight started.
fn busy(m: &Model) -> Vec<Duration> {
    let mut out = Vec::new();
    if m.scanning {
        out.push(Duration::ZERO);
    }
    if matches!(m.daemon, Daemon::Connecting) {
        out.push(Duration::ZERO);
    }
    if !m.grid.on
        && let PreviewState::Loading { since, .. } = m.preview.state
    {
        out.push(since);
    }
    out.extend(m.applying.first().map(|b| b.since));
    out
}

/// The spinner frame for work that started at `since`, or `None` while it
/// is still too young to deserve one.
pub fn spinner(m: &Model, since: Duration) -> Option<&'static str> {
    let age = m.now.checked_sub(since + SPINNER_DELAY)?;
    let frame = age.as_millis() / SPINNER_FRAME.as_millis();
    Some(SPINNER[usize::try_from(frame).unwrap_or(0) % SPINNER.len()])
}

fn tick(m: &mut Model) -> Vec<Cmd> {
    if m.toast.is_some() && m.now >= m.toast_until {
        m.toast = None;
        m.dirty = true;
    }
    let mut cmds = Vec::new();
    if !m.pending.is_empty() && m.now >= m.pending_since + CHORD_TIMEOUT {
        let pending = std::mem::take(&mut m.pending);
        m.dirty = true;
        if let Lookup::Prefix {
            alone: Some(action),
        } = m.keymap.lookup(Scope::of(m.mode), &pending)
        {
            cmds = perform(m, action);
        }
    }
    if busy(m).into_iter().any(|since| spinner(m, since).is_some()) {
        m.dirty = true;
    }
    cmds
}

fn toast(m: &mut Model, kind: ToastKind, text: String) {
    m.toast_until = m.now
        + match kind {
            ToastKind::Success => TOAST_OK,
            ToastKind::Error => TOAST_ERROR,
        };
    m.toast = Some(Toast { kind, text });
    m.dirty = true;
}

fn key(m: &mut Model, event: KeyEvent) -> Vec<Cmd> {
    let key = Key::from_event(event);
    // Ctrl-C always quits, whatever the keymap says.
    if key.code == KeyCode::Char('c') && key.mods == KeyModifiers::CONTROL {
        return perform(m, Action::Quit);
    }
    let scope = Scope::of(m.mode);
    let mut keys = m.pending.clone();
    keys.push(key);
    match m.keymap.lookup(scope, &keys) {
        Lookup::Action(action) => {
            m.dirty |= !m.pending.is_empty();
            m.pending.clear();
            perform(m, action)
        }
        Lookup::Prefix { .. } => {
            if m.pending.is_empty() {
                m.pending_since = m.now;
            }
            m.pending = keys;
            m.dirty = true;
            Vec::new()
        }
        Lookup::Unbound if !m.pending.is_empty() => {
            // The sequence broke off: the keys so far count on their own,
            // then this key starts afresh.
            let pending = std::mem::take(&mut m.pending);
            m.dirty = true;
            let mut cmds = match m.keymap.lookup(scope, &pending) {
                Lookup::Prefix {
                    alone: Some(action),
                } => perform(m, action),
                _ => Vec::new(),
            };
            cmds.extend(self::key(m, event));
            cmds
        }
        Lookup::Unbound if scope == Scope::Prompt => edit_prompt(m, key),
        Lookup::Unbound => Vec::new(),
    }
}

/// Typing in the search, help or palette prompt.
fn edit_prompt(m: &mut Model, key: Key) -> Vec<Cmd> {
    let query = match m.mode {
        Mode::Search => &mut m.query,
        _ => &mut m.prompt.query,
    };
    let changed = match key.code {
        KeyCode::Char(c) if key.mods.is_empty() => {
            query.push(c);
            true
        }
        KeyCode::Backspace => query.pop().is_some(),
        KeyCode::Char('u') if key.mods == KeyModifiers::CONTROL => {
            let had = !query.is_empty();
            query.clear();
            had
        }
        KeyCode::Char('w') if key.mods == KeyModifiers::CONTROL => {
            let trimmed = query.trim_end();
            let cut = trimmed.rfind(' ').map_or(0, |i| i + 1);
            let had = !query.is_empty();
            query.truncate(cut);
            had
        }
        _ => false,
    };
    if !changed {
        return Vec::new();
    }
    m.dirty = true;
    match m.mode {
        Mode::Search => refilter(m, Keep::Best, When::Soon),
        _ => {
            m.prompt.selected = 0;
            Vec::new()
        }
    }
}

/// Carries out `action`. Sets `m.dirty` if the screen changes.
fn perform(m: &mut Model, action: Action) -> Vec<Cmd> {
    let half_page = (usize::from(layout::list_rows(m.area)) / 2).max(1);
    // In the grid, up and down move a whole row.
    let row = if m.grid.on {
        layout::grid(m.area).cols
    } else {
        1
    };
    match action {
        Action::Down => {
            let last = m.visible.len().saturating_sub(1);
            match m.selected.checked_add(row) {
                Some(i) if i <= last => select(m, i),
                // From the row above the last, short row: its last tile.
                _ if last / row > m.selected / row => select(m, last),
                _ => Vec::new(),
            }
        }
        Action::Up => match m.selected.checked_sub(row) {
            Some(i) => select(m, i),
            None => Vec::new(),
        },
        Action::Left if m.grid.on => select(m, m.selected.saturating_sub(1)),
        Action::Right if m.grid.on => select(m, m.selected.saturating_add(1)),
        Action::Left | Action::Right => Vec::new(),
        Action::ToggleGrid => {
            m.grid.on = !m.grid.on;
            m.dirty = true;
            scroll_to_selection(m);
            refresh(m, When::Now)
        }
        Action::Top => select(m, 0),
        Action::Bottom => select(m, usize::MAX),
        Action::PageDown => select(m, m.selected.saturating_add(half_page)),
        Action::PageUp => select(m, m.selected.saturating_sub(half_page)),
        Action::Apply => {
            let target = m.targeted().map(|t| t.name.clone());
            apply(m, target)
        }
        Action::ApplyAll => apply(m, None),
        Action::Random => random(m),
        Action::Monitor => next_monitor(m),
        Action::Favorite => toggle_favorite(m),
        Action::FavoritesOnly => {
            m.favorites_only = !m.favorites_only;
            m.dirty = true;
            refilter(m, Keep::Selection, When::Now)
        }
        Action::Search => enter(m, Mode::Search),
        Action::Help => enter(m, Mode::Help),
        Action::Palette => enter(m, Mode::Palette),
        Action::Back if m.filtered() => {
            m.query.clear();
            m.favorites_only = false;
            m.dirty = true;
            refilter(m, Keep::Selection, When::Now)
        }
        Action::Back | Action::Quit => {
            m.quit = true;
            Vec::new()
        }
        Action::Next => step(m, 1),
        Action::Prev => step(m, -1),
        Action::Confirm => confirm(m),
        Action::Cancel => {
            let was = m.mode;
            m.mode = Mode::Normal;
            m.dirty = true;
            if was == Mode::Search && !m.query.is_empty() {
                m.query.clear();
                return refilter(m, Keep::Selection, When::Now);
            }
            Vec::new()
        }
    }
}

fn enter(m: &mut Model, mode: Mode) -> Vec<Cmd> {
    m.mode = mode;
    m.prompt = Prompt::default();
    m.dirty = true;
    Vec::new()
}

/// Next/Prev in a prompt: the list for search, the rows of help and the
/// palette.
fn step(m: &mut Model, by: isize) -> Vec<Cmd> {
    match m.mode {
        Mode::Normal | Mode::Search => select(m, m.selected.saturating_add_signed(by)),
        Mode::Help => {
            let rows = keymap::help_rows(&m.keymap, &m.prompt.query).len();
            let shown = usize::from(layout::overlay(m.area).rows.height);
            let last = rows.saturating_sub(shown);
            let next = m.prompt.selected.saturating_add_signed(by).min(last);
            m.dirty |= next != m.prompt.selected;
            m.prompt.selected = next;
            Vec::new()
        }
        Mode::Palette => {
            let rows = keymap::palette_rows(&m.keymap, &m.prompt.query).len();
            let next = m
                .prompt
                .selected
                .saturating_add_signed(by)
                .min(rows.saturating_sub(1));
            m.dirty |= next != m.prompt.selected;
            m.prompt.selected = next;
            Vec::new()
        }
    }
}

fn confirm(m: &mut Model) -> Vec<Cmd> {
    let was = m.mode;
    m.mode = Mode::Normal;
    m.dirty = true;
    if was != Mode::Palette {
        return Vec::new();
    }
    let rows = keymap::palette_rows(&m.keymap, &m.prompt.query);
    match rows.get(m.prompt.selected) {
        Some(row) => perform(m, row.action),
        None => Vec::new(),
    }
}

/// Which row to highlight after the list changes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Keep {
    /// The same wallpaper, if it is still shown.
    Selection,
    /// The best match.
    Best,
}

/// Rebuilds `visible` from the query and the Favorites filter, and renders
/// what is on screen anew.
fn refilter(m: &mut Model, keep: Keep, when: When) -> Vec<Cmd> {
    let before = m.selection().cloned();
    let candidates: Vec<usize> = (0..m.library.len())
        .filter(|&i| !m.favorites_only || m.is_favorite(&m.library[i]))
        .collect();
    m.visible = m.index.filter(&m.query, candidates.into_iter());
    m.selected = match (keep, &before) {
        (Keep::Selection, Some(path)) => m
            .visible
            .iter()
            .position(|v| &m.library[v.index] == path)
            .unwrap_or(0),
        _ => 0,
    };
    m.list_offset = 0;
    m.grid.offset_row = 0;
    scroll_to_selection(m);
    m.dirty = true;
    let unchanged =
        m.selection() == before.as_ref() && !matches!(m.preview.state, PreviewState::Empty);
    if unchanged && !m.grid.on {
        return Vec::new();
    }
    refresh(m, when)
}

/// Highlights `index` (clamped to the list) and, once it settles, renders
/// its preview (or the grid's new tiles, if the grid scrolled).
fn select(m: &mut Model, index: usize) -> Vec<Cmd> {
    let index = index.min(m.visible.len().saturating_sub(1));
    if index == m.selected {
        return Vec::new();
    }
    m.selected = index;
    m.dirty = true;
    let scrolled = scroll_to_selection(m);
    if !m.grid.on || scrolled {
        refresh(m, When::Soon)
    } else {
        Vec::new()
    }
}

/// Moves the list offset and the grid's first row just enough to keep the
/// selection in view. Returns whether the grid scrolled.
fn scroll_to_selection(m: &mut Model) -> bool {
    let rows = usize::from(layout::list_rows(m.area)).max(1);
    m.list_offset = scrolled(m.list_offset, m.selected, rows);
    let grid = layout::grid(m.area);
    let first = scrolled(m.grid.offset_row, m.selected / grid.cols, grid.rows);
    let moved = first != m.grid.offset_row;
    m.grid.offset_row = first;
    moved
}

/// The first of `rows` visible lines after the least scrolling from
/// `offset` that shows line `target`.
fn scrolled(offset: usize, target: usize, rows: usize) -> usize {
    if target < offset {
        target
    } else if target >= offset + rows {
        target + 1 - rows
    } else {
        offset
    }
}

/// Renders whatever is on screen anew: the grid's tiles or the preview.
fn refresh(m: &mut Model, when: When) -> Vec<Cmd> {
    if m.grid.on {
        request_tiles(m, when)
    } else {
        request_preview(m, when)
    }
}

fn applying_label(monitor: Option<&str>) -> String {
    match monitor {
        Some(name) => format!("Applying to {name}"),
        None => "Applying to every monitor".into(),
    }
}

/// Applies the selection to `monitor`, or to every Monitor.
fn apply(m: &mut Model, monitor: Option<String>) -> Vec<Cmd> {
    let Some(path) = m.selection().cloned() else {
        return Vec::new();
    };
    if m.monitors().is_empty() {
        let why = match &m.daemon {
            Daemon::Down(err) => err.clone(),
            _ => "vellumd has not reported any monitor yet".into(),
        };
        toast(m, ToastKind::Error, why);
        return Vec::new();
    }
    m.applying.push(Busy {
        label: applying_label(monitor.as_deref()),
        since: m.now,
    });
    vec![Cmd::Apply(ApplyJob {
        path,
        monitor,
        fit: m.fit,
    })]
}

/// Highlights a random wallpaper from the list (never the current one, if
/// there is a choice) and applies it to the targeted Monitor.
fn random(m: &mut Model) -> Vec<Cmd> {
    let n = m.visible.len();
    if n == 0 {
        return Vec::new();
    }
    let pick = if n == 1 {
        0
    } else {
        let i = m.rng.usize(..n - 1);
        if i >= m.selected { i + 1 } else { i }
    };
    let mut cmds = select(m, pick);
    cmds.extend(perform(m, Action::Apply));
    cmds
}

/// Targets the next Monitor, wrapping around.
fn next_monitor(m: &mut Model) -> Vec<Cmd> {
    let n = m.monitors().len();
    if n < 2 {
        return Vec::new();
    }
    target(m, (m.target + 1) % n)
}

fn target(m: &mut Model, index: usize) -> Vec<Cmd> {
    if index == m.target || index >= m.monitors().len() {
        return Vec::new();
    }
    m.target = index;
    retarget(m);
    m.dirty = true;
    refresh(m, When::Now)
}

/// Refreshes what depends on the targeted Monitor.
fn retarget(m: &mut Model) {
    let name = m.targeted().map(|t| t.name.clone());
    m.rotation = m
        .status
        .as_ref()
        .map_or(Rotation::Off, |s| Rotation::from_status(s, name.as_deref()));
}

fn toggle_favorite(m: &mut Model) -> Vec<Cmd> {
    let Some(path) = m.selection().cloned() else {
        return Vec::new();
    };
    let favorite = !m.is_favorite(&path);
    if favorite {
        m.favorites.push(path.clone());
    } else {
        m.favorites.retain(|f| f != &path);
    }
    m.dirty = true;
    let mut cmds = vec![Cmd::SaveFavorite { path, favorite }];
    if m.favorites_only {
        cmds.extend(refilter(m, Keep::Selection, When::Now));
    }
    cmds
}

fn favorite_saved(
    m: &mut Model,
    path: PathBuf,
    favorite: bool,
    result: Result<(), String>,
) -> Vec<Cmd> {
    let wanted = m.is_favorite(&path);
    match result {
        // Toggled again while this save was in flight: save the latest.
        Ok(()) if wanted != favorite => vec![Cmd::SaveFavorite {
            path,
            favorite: wanted,
        }],
        Ok(()) => {
            let name = m.short_name(&path);
            let text = match favorite {
                true => format!("★ {name} is a Favorite"),
                false => format!("{name} is no longer a Favorite"),
            };
            toast(m, ToastKind::Success, text);
            Vec::new()
        }
        Err(err) => {
            if wanted == favorite {
                // Put the list back the way the file is.
                if favorite {
                    m.favorites.retain(|f| f != &path);
                } else {
                    m.favorites.push(path);
                }
            }
            toast(m, ToastKind::Error, err);
            if m.favorites_only {
                return refilter(m, Keep::Selection, When::Now);
            }
            Vec::new()
        }
    }
}

fn mouse(m: &mut Model, event: MouseEvent) -> Vec<Cmd> {
    let at = Position::new(event.column, event.row);
    let overlay = matches!(m.mode, Mode::Help | Mode::Palette);
    match event.kind {
        MouseEventKind::ScrollDown if overlay => step(m, 1),
        MouseEventKind::ScrollUp if overlay => step(m, -1),
        // In the list one row per notch, in the grid one row of tiles.
        MouseEventKind::ScrollDown => perform(m, Action::Down),
        MouseEventKind::ScrollUp => perform(m, Action::Up),
        MouseEventKind::Down(MouseButton::Left) if overlay => click_overlay(m, at),
        MouseEventKind::Down(MouseButton::Left) => click_main(m, at),
        _ => Vec::new(),
    }
}

fn click_overlay(m: &mut Model, at: Position) -> Vec<Cmd> {
    let overlay = layout::overlay(m.area);
    if !overlay.outer.contains(at) {
        return perform(m, Action::Cancel);
    }
    if m.mode == Mode::Palette && overlay.rows.contains(at) {
        let offset = layout::palette_offset(m.prompt.selected, overlay.rows.height);
        let row = offset + usize::from(at.y - overlay.rows.y);
        if row < keymap::palette_rows(&m.keymap, &m.prompt.query).len() {
            m.prompt.selected = row;
            return confirm(m);
        }
    }
    Vec::new()
}

fn click_main(m: &mut Model, at: Position) -> Vec<Cmd> {
    if let Some(index) = wallpaper_at(m, at) {
        let double = m
            .last_click
            .is_some_and(|(i, t)| i == index && m.now.saturating_sub(t) <= DOUBLE_CLICK);
        if double {
            m.last_click = None;
            let mut cmds = select(m, index);
            cmds.extend(perform(m, Action::Apply));
            return cmds;
        }
        m.last_click = Some((index, m.now));
        return select(m, index);
    }
    let areas = layout::areas(m.area);
    if areas.header.contains(at) {
        let tabs = layout::monitor_tabs(m.monitors());
        if let Some(i) = tabs.iter().position(|(x, label)| {
            (*x..x + layout::TAB_MARK + layout::width(label)).contains(&at.x)
        }) {
            return target(m, i);
        }
        return Vec::new();
    }
    if let Some(action) = hint_at(m, at) {
        return perform(m, action);
    }
    Vec::new()
}

/// The index into `visible` of the list row or grid tile under `at`.
fn wallpaper_at(m: &Model, at: Position) -> Option<usize> {
    let index = if m.grid.on {
        let grid = layout::grid(m.area);
        (0..grid.page()).find_map(|i| {
            let (image, name) = grid.tile(i);
            (image.contains(at) || name.contains(at)).then(|| m.grid.offset_row * grid.cols + i)
        })?
    } else {
        let rows = layout::areas(m.area).list.inner(Margin::new(1, 1));
        if !rows.contains(at) {
            return None;
        }
        m.list_offset + usize::from(at.y - rows.y)
    };
    (index < m.visible.len()).then_some(index)
}

/// The footer hint under `at`, if any.
fn hint_at(m: &Model, at: Position) -> Option<Action> {
    let footer = layout::areas(m.area).footer;
    if !footer.contains(at) {
        return None;
    }
    keymap::footer(&m.keymap, m.mode, m.filtered(), m.grid.on, footer.width)
        .into_iter()
        .find(|h| (h.x..h.x + h.width).contains(&at.x))
        .map(|h| h.action)
}

/// Asks for a preview of the selection, for the targeted Monitor, sized to
/// the preview pane. Bumps the generation so older work is dropped, then
/// renders now or once things settle.
fn request_preview(m: &mut Model, when: When) -> Vec<Cmd> {
    let Some(path) = m.selection().cloned() else {
        m.preview.state = PreviewState::Empty;
        return Vec::new();
    };
    if layout::areas(m.area).image.is_empty() {
        return Vec::new();
    }
    m.preview.generation += 1;
    m.preview.state = PreviewState::Loading { path, since: m.now };
    match when {
        When::Now => preview_jobs(m),
        When::Soon => vec![Cmd::After {
            delay: DEBOUNCE,
            wake: Wake::Preview {
                generation: m.preview.generation,
            },
        }],
    }
}

/// The selection's preview, then its neighbours' in the list as
/// prefetches.
fn preview_jobs(m: &Model) -> Vec<Cmd> {
    let PreviewState::Loading { path, .. } = &m.preview.state else {
        return Vec::new();
    };
    let cells = layout::areas(m.area).image.as_size();
    let monitor = m.targeted().map_or(Screen::FALLBACK, Screen::from);
    let job = |purpose, path: &PathBuf| {
        Cmd::RenderPreview(PreviewJob {
            generation: m.preview.generation,
            purpose,
            path: path.clone(),
            cells,
            monitor,
            fit: m.fit,
            letterbox: m.letterbox,
        })
    };
    let neighbours = PREFETCH
        .iter()
        .filter_map(|d| m.selected.checked_add_signed(*d))
        .filter_map(|i| m.visible.get(i))
        .map(|v| &m.library[v.index]);
    std::iter::once(job(Purpose::Preview, path))
        .chain(neighbours.map(|p| job(Purpose::Prefetch, p)))
        .collect()
}

/// The wallpapers the list shows, in order.
fn shown(m: &Model) -> impl Iterator<Item = &PathBuf> {
    m.visible.iter().map(|v| &m.library[v.index])
}

/// The wallpapers on screen in the grid, in order.
pub fn visible(m: &Model) -> impl Iterator<Item = &PathBuf> {
    let grid = layout::grid(m.area);
    shown(m)
        .skip(m.grid.offset_row * grid.cols)
        .take(grid.page())
}

/// Asks for thumbnails of the tiles on screen that have none yet. Bumps
/// the grid generation, so tiles that scrolled away stop rendering.
fn request_tiles(m: &mut Model, when: When) -> Vec<Cmd> {
    let grid = layout::grid(m.area);
    if grid.tiles.is_empty() || m.visible.is_empty() {
        return Vec::new();
    }
    let spec = TileSpec {
        cells: layout::TILE_IMAGE,
        monitor: m.targeted().map_or(Screen::FALLBACK, Screen::from),
        fit: m.fit,
        letterbox: m.letterbox,
    };
    if m.grid.spec != Some(spec) {
        m.grid.tiles.clear();
        m.grid.spec = Some(spec);
    }
    m.grid.generation += 1;
    // Keep finished tiles about a page either side, so scrolling back is
    // instant but memory stays bounded.
    let page = grid.page();
    let first = (m.grid.offset_row * grid.cols).saturating_sub(page);
    let kept: HashSet<PathBuf> = shown(m).skip(first).take(3 * page).cloned().collect();
    m.grid
        .tiles
        .retain(|path, tile| kept.contains(path) && !matches!(tile, Tile::Loading));
    let missing: Vec<PathBuf> = visible(m)
        .filter(|p| !m.grid.tiles.contains_key(*p))
        .cloned()
        .collect();
    for path in missing {
        m.grid.tiles.insert(path, Tile::Loading);
    }
    match when {
        When::Now => tile_jobs(m),
        When::Soon => vec![Cmd::After {
            delay: DEBOUNCE,
            wake: Wake::Grid {
                generation: m.grid.generation,
            },
        }],
    }
}

/// Thumbnail jobs for the tiles on screen that are still loading.
fn tile_jobs(m: &Model) -> Vec<Cmd> {
    let Some(spec) = m.grid.spec else {
        return Vec::new();
    };
    visible(m)
        .filter(|p| matches!(m.grid.tiles.get(*p), Some(Tile::Loading)))
        .map(|path| {
            Cmd::RenderPreview(PreviewJob {
                generation: m.grid.generation,
                purpose: Purpose::Thumb,
                path: path.clone(),
                cells: spec.cells,
                monitor: spec.monitor,
                fit: spec.fit,
                letterbox: spec.letterbox,
            })
        })
        .collect()
}
