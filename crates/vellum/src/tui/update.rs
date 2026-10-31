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
use vellum_ipc::{RotationOrder, RotationSource, Schedule};

use super::model::{
    Busy, Daemon, Field, Mode, Model, Panel, PreviewState, Prompt, Rotation, Tile, TileSpec, Toast,
    ToastKind, Welcome,
};
use super::msg::{ApplyJob, Cmd, Msg, PreviewJob, Purpose, Screen, Wake};
use crate::config::edit::Edit;
use crate::config::{Named, format_interval};

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
/// Config edits are written once changes pause this long, so holding a
/// key down writes the file once.
pub const SAVE_DELAY: Duration = Duration::from_millis(300);
/// The Rotation intervals the panel steps through.
pub const INTERVALS: [u64; 11] = [
    60_000,
    5 * 60_000,
    10 * 60_000,
    15 * 60_000,
    30 * 60_000,
    3_600_000,
    2 * 3_600_000,
    3 * 3_600_000,
    6 * 3_600_000,
    12 * 3_600_000,
    24 * 3_600_000,
];

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
                Wake::Config { generation }
                    if generation == m.config_generation && !m.config_edits.is_empty() =>
                {
                    vec![Cmd::SaveConfig {
                        generation,
                        edits: std::mem::take(&mut m.config_edits),
                    }]
                }
                Wake::Preview { .. } | Wake::Grid { .. } | Wake::Config { .. } => Vec::new(),
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
        Msg::ConfigSaved { generation, result } => config_saved(m, generation, result),
        Msg::RotationUpdated { toggled, result } => rotation_updated(m, toggled, result),
        Msg::Completed { input, candidates } => completed(m, input, candidates),
        Msg::LibrarySaved(result) => library_saved(m, result),
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
        Mode::Welcome if m.welcome.saving => return Vec::new(),
        Mode::Welcome => &mut m.welcome.input,
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
        Mode::Welcome => {
            // Typing a folder chooses it.
            m.welcome.choice = 1;
            m.welcome.candidates.clear();
            m.welcome.error = None;
            Vec::new()
        }
        _ => {
            m.prompt.selected = 0;
            Vec::new()
        }
    }
}

/// Carries out `action`. Sets `m.dirty` if the screen changes.
fn perform(m: &mut Model, action: Action) -> Vec<Cmd> {
    let half_page = (usize::from(layout::list_rows(m.area, m.preview_only)) / 2).max(1);
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
        Action::Rotation => {
            m.panel = Panel {
                row: 0,
                monitor: None,
            };
            enter(m, Mode::Rotation)
        }
        Action::Library => {
            m.welcome = Welcome::default();
            enter(m, Mode::Welcome)
        }
        Action::TogglePreview if layout::is_narrow(m.area) && !m.grid.on => {
            m.preview_only = !m.preview_only;
            m.dirty = true;
            scroll_to_selection(m);
            refresh(m, When::Now)
        }
        Action::TogglePreview => Vec::new(),
        Action::Reconnect => {
            m.daemon = Daemon::Connecting;
            m.dirty = true;
            vec![Cmd::QueryDaemon]
        }
        Action::Complete if m.mode == Mode::Welcome && !m.welcome.saving => {
            m.welcome.choice = 1;
            m.dirty = true;
            vec![Cmd::Complete(m.welcome.input.clone())]
        }
        Action::Complete => Vec::new(),
        Action::FieldDown => move_field(m, 1),
        Action::FieldUp => move_field(m, -1),
        Action::Decrease => change_field(m, -1),
        Action::Increase | Action::Toggle => change_field(m, 1),
        Action::RemoveOverride => remove_override(m),
        Action::Close => {
            m.mode = Mode::Normal;
            m.dirty = true;
            Vec::new()
        }
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
        Mode::Welcome => {
            let next = m.welcome.choice.saturating_add_signed(by).min(1);
            m.dirty |= next != m.welcome.choice;
            m.welcome.choice = next;
            Vec::new()
        }
        Mode::Rotation => move_field(m, by),
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
    if m.mode == Mode::Welcome {
        return choose_folder(m);
    }
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
    let rows = usize::from(layout::list_rows(m.area, m.preview_only)).max(1);
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
    if layout::too_small(m.area) || m.mode == Mode::Welcome {
        return Vec::new();
    }
    if m.mode == Mode::Rotation {
        let outside = !layout::panel(m.area).0.contains(at);
        return match event.kind {
            MouseEventKind::Down(MouseButton::Left) if outside => perform(m, Action::Close),
            MouseEventKind::ScrollDown => move_field(m, 1),
            MouseEventKind::ScrollUp => move_field(m, -1),
            _ => Vec::new(),
        };
    }
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
    let areas = m.areas();
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
        let rows = m.areas().list.inner(Margin::new(1, 1));
        if !rows.contains(at) {
            return None;
        }
        m.list_offset + usize::from(at.y - rows.y)
    };
    (index < m.visible.len()).then_some(index)
}

/// The footer hint under `at`, if any.
fn hint_at(m: &Model, at: Position) -> Option<Action> {
    let footer = m.areas().footer;
    if !footer.contains(at) {
        return None;
    }
    keymap::footer(&m.keymap, m.mode, m.footer_context(), footer.width)
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
    if m.areas().image.is_empty() || layout::too_small(m.area) {
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
    let cells = m.areas().image.as_size();
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

/// The Rotation panel's lines. The scope line only once Monitors are
/// known, since an Override names one.
pub fn panel_fields(m: &Model) -> Vec<Field> {
    let mut fields = vec![Field::Running];
    if !m.monitors().is_empty() {
        fields.push(Field::Scope);
    }
    fields.extend([Field::Interval, Field::Source, Field::Order]);
    fields
}

/// The schedule the panel shows: the global one, or the Monitor's.
pub fn panel_schedule(m: &Model) -> Schedule {
    match &m.panel.monitor {
        Some(name) => *m.schedules.schedule_for(name),
        None => m.schedules.schedule,
    }
}

/// Whether the panel's Monitor has an Override in the Config.
pub fn has_override(m: &Model) -> bool {
    m.panel
        .monitor
        .as_ref()
        .is_some_and(|name| m.schedules.overrides.iter().any(|o| &o.monitor == name))
}

/// Whether the Daemon says Rotation is running.
pub fn rotation_running(m: &Model) -> bool {
    m.status.as_ref().is_some_and(|s| s.rotation_running)
}

fn move_field(m: &mut Model, by: isize) -> Vec<Cmd> {
    let last = panel_fields(m).len() - 1;
    let next = m.panel.row.saturating_add_signed(by).min(last);
    m.dirty |= next != m.panel.row;
    m.panel.row = next;
    Vec::new()
}

/// The value `by` steps from `current` in `values`, wrapping around.
fn cycle<T: Clone + PartialEq>(values: &[T], current: &T, by: isize) -> T {
    let i = values.iter().position(|v| v == current).unwrap_or(0);
    let n = values.len();
    let step = by.rem_euclid(n as isize) as usize;
    values[(i + step) % n].clone()
}

/// The next interval preset above (`by` > 0) or below `ms`, staying put
/// at either end.
fn step_interval(ms: u64, by: isize) -> u64 {
    if by > 0 {
        INTERVALS.iter().copied().find(|&i| i > ms).unwrap_or(ms)
    } else {
        INTERVALS
            .iter()
            .copied()
            .rev()
            .find(|&i| i < ms)
            .unwrap_or(ms)
    }
}

/// Changes the highlighted setting by one step.
fn change_field(m: &mut Model, by: isize) -> Vec<Cmd> {
    let Some(field) = panel_fields(m).get(m.panel.row).copied() else {
        return Vec::new();
    };
    let schedule = panel_schedule(m);
    match field {
        Field::Running => {
            m.dirty = true;
            vec![Cmd::RotationControl(!rotation_running(m))]
        }
        Field::Scope => {
            let mut scopes: Vec<Option<String>> = vec![None];
            scopes.extend(m.monitors().iter().map(|mon| Some(mon.name.clone())));
            m.panel.monitor = cycle(&scopes, &m.panel.monitor, by);
            m.dirty = true;
            Vec::new()
        }
        Field::Interval => {
            let ms = step_interval(schedule.interval_ms, by);
            if ms == schedule.interval_ms {
                return Vec::new();
            }
            edit_schedule(m, "interval", format_interval(ms), |s| s.interval_ms = ms)
        }
        Field::Source => {
            let all = [RotationSource::Library, RotationSource::Favorites];
            let source = cycle(&all, &schedule.source, by);
            edit_schedule(m, "source", source.name(), |s| s.source = source)
        }
        Field::Order => {
            let all = [RotationOrder::Shuffle, RotationOrder::Sequential];
            let order = cycle(&all, &schedule.order, by);
            edit_schedule(m, "order", order.name(), |s| s.order = order)
        }
    }
}

/// Sets one Rotation key in the panel's scope: at once in what the panel
/// shows, and in the Config once edits pause.
fn edit_schedule(
    m: &mut Model,
    key: &str,
    value: impl Into<String>,
    change: impl Fn(&mut Schedule),
) -> Vec<Cmd> {
    let edit = match m.panel.monitor.clone() {
        None => {
            let before = m.schedules.schedule;
            change(&mut m.schedules.schedule);
            // Overrides that left this key out follow the global value.
            // Until the saved Config says which did, guess: those that had
            // the old global value.
            for o in &mut m.schedules.overrides {
                let mut mine = o.schedule;
                let mut probe = before;
                change(&mut probe);
                if probe.interval_ms != before.interval_ms && mine.interval_ms == before.interval_ms
                {
                    mine.interval_ms = probe.interval_ms;
                }
                if probe.source != before.source && mine.source == before.source {
                    mine.source = probe.source;
                }
                if probe.order != before.order && mine.order == before.order {
                    mine.order = probe.order;
                }
                o.schedule = mine;
            }
            Edit::set(&["rotation"], key, value)
        }
        Some(name) => {
            let global = m.schedules.schedule;
            let i = match m.schedules.overrides.iter().position(|o| o.monitor == name) {
                Some(i) => i,
                None => {
                    m.schedules.overrides.push(vellum_ipc::Override {
                        monitor: name.clone(),
                        schedule: global,
                    });
                    m.schedules.overrides.len() - 1
                }
            };
            change(&mut m.schedules.overrides[i].schedule);
            Edit::set(&["rotation", "monitor", &name], key, value)
        }
    };
    m.dirty = true;
    retarget_schedules(m);
    queue_edit(m, edit)
}

/// Drops the panel's Monitor's Override, so it follows the global
/// Rotation again.
fn remove_override(m: &mut Model) -> Vec<Cmd> {
    let Some(name) = m.panel.monitor.clone() else {
        return Vec::new();
    };
    if !has_override(m) {
        return Vec::new();
    }
    m.schedules.overrides.retain(|o| o.monitor != name);
    m.dirty = true;
    retarget_schedules(m);
    queue_edit(
        m,
        Edit::Remove {
            table: vec!["rotation".into(), "monitor".into(), name],
        },
    )
}

/// Shows an edit in the header at once: the Daemon will hold these
/// settings as soon as they are saved and pushed.
fn retarget_schedules(m: &mut Model) {
    if let Some(status) = &mut m.status {
        status.settings.rotation = m.schedules.clone();
    }
    retarget(m);
}

/// Adds `edit` to those waiting (replacing one with the same target) and
/// restarts the save timer.
fn queue_edit(m: &mut Model, edit: Edit) -> Vec<Cmd> {
    m.config_edits.retain(|e| e.target() != edit.target());
    m.config_edits.push(edit);
    m.config_generation += 1;
    vec![Cmd::After {
        delay: SAVE_DELAY,
        wake: Wake::Config {
            generation: m.config_generation,
        },
    }]
}

fn config_saved(
    m: &mut Model,
    generation: u64,
    result: Result<crate::config::Config, String>,
) -> Vec<Cmd> {
    match result {
        Ok(config) => {
            // Newer edits are waiting: what the panel shows is newer.
            if generation == m.config_generation && m.config_edits.is_empty() {
                m.schedules = config.rotation.clone();
            }
            toast(m, ToastKind::Success, "Saved to the Config".into());
            if matches!(m.daemon, Daemon::Up { .. }) {
                return vec![Cmd::PushSettings(config.settings())];
            }
            Vec::new()
        }
        Err(err) => {
            toast(m, ToastKind::Error, err);
            Vec::new()
        }
    }
}

fn rotation_updated(
    m: &mut Model,
    toggled: bool,
    result: Result<vellum_ipc::Status, String>,
) -> Vec<Cmd> {
    match result {
        Ok(status) => {
            let running = status.rotation_running;
            m.status = Some(status);
            // Edits still waiting to be saved stay on screen.
            if !m.config_edits.is_empty() {
                retarget_schedules(m);
            }
            retarget(m);
            if toggled {
                let text = match &m.rotation {
                    Rotation::On { interval_ms, .. } if running => {
                        format!("Rotation started: every {}", format_interval(*interval_ms))
                    }
                    _ => "Rotation stopped".into(),
                };
                toast(m, ToastKind::Success, text);
            }
        }
        Err(err) => toast(m, ToastKind::Error, err),
    }
    Vec::new()
}

/// Enter on the welcome screen: save the chosen folder.
fn choose_folder(m: &mut Model) -> Vec<Cmd> {
    if m.welcome.saving {
        return Vec::new();
    }
    m.dirty = true;
    let folder = match m.welcome.choice {
        0 => m.home.as_ref().map(|h| h.join("Pictures")),
        _ if m.welcome.input.trim().is_empty() => {
            m.welcome.error = Some("Type a folder, like ~/Wallpapers".into());
            return Vec::new();
        }
        _ => super::complete::expand(&m.welcome.input, m.home.as_deref()),
    };
    let Some(folder) = folder else {
        m.welcome.error = Some("Type a full path, starting with ~/ or /".into());
        return Vec::new();
    };
    m.welcome.error = None;
    m.welcome.saving = true;
    vec![Cmd::SaveLibrary(folder)]
}

fn completed(m: &mut Model, input: String, candidates: Vec<String>) -> Vec<Cmd> {
    // Typed on since: this is about an older input.
    if m.mode != Mode::Welcome || input != m.welcome.input {
        m.dirty = false;
        return Vec::new();
    }
    let w = &mut m.welcome;
    match candidates.as_slice() {
        [] => {
            w.candidates.clear();
            w.error = Some("No folder starts with that".into());
        }
        [one] => {
            w.input = one.clone();
            w.candidates.clear();
            w.error = None;
        }
        many => {
            let common = super::complete::common_prefix(many);
            if common.len() > w.input.len() {
                w.input = common;
            }
            w.candidates = candidates;
            w.error = None;
        }
    }
    Vec::new()
}

fn library_saved(m: &mut Model, result: Result<crate::config::Config, String>) -> Vec<Cmd> {
    m.welcome.saving = false;
    match result {
        Ok(config) => {
            m.folders = config.library.clone();
            m.schedules = config.rotation.clone();
            m.mode = Mode::Normal;
            m.scanning = true;
            m.library.clear();
            m.visible.clear();
            m.selected = 0;
            m.preview.state = PreviewState::Empty;
            let folders: Vec<String> = m.folders.iter().map(|f| m.tilde(f)).collect();
            toast(
                m,
                ToastKind::Success,
                format!("Library: {}, saved to the Config", folders.join(", ")),
            );
            vec![Cmd::ScanLibrary(m.folders.clone())]
        }
        Err(err) => {
            m.welcome.error = Some(err);
            Vec::new()
        }
    }
}
