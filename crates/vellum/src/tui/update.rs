//! The update function: the only place the Model changes.
//!
//! `update(&mut Model, Msg) -> Vec<Cmd>` is pure: no I/O, no clocks, no
//! threads. Keys go through [`keymap::action`] to an [`Action`], and
//! [`perform`] turns an Action into Model changes and Cmds.
//!
//! Previews are debounced: moving bumps the preview generation at once (so
//! workers drop what they were doing for the old selection) and asks the
//! runtime for a [`Wake`] after [`DEBOUNCE`]. Only if nothing newer came by
//! then does update ask for the render, plus prefetches of the neighbours.
//! Resizes and grid scrolling work the same way.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use ratatui::layout::Rect;

use super::keymap::{self, Action};
use super::layout;
use super::model::{Daemon, Model, PreviewState, Rotation, Tile, TileSpec, Toast, ToastKind};
use super::msg::{ApplyJob, Cmd, Msg, PreviewJob, Purpose, Screen, Wake};

#[cfg(test)]
mod tests;

/// How long the selection (or the terminal size) must stay put before
/// anything is rendered for it.
pub const DEBOUNCE: Duration = Duration::from_millis(80);

/// Neighbours of the selection to prefetch, nearest first.
const PREFETCH: [isize; 4] = [1, -1, 2, -2];

/// Whether to render right away or after [`DEBOUNCE`].
#[derive(Clone, Copy)]
enum When {
    Now,
    Soon,
}

/// The Cmds to run at startup: scan the Library and ask the Daemon, in
/// parallel, so the list shows up without waiting for either.
pub fn init(model: &Model) -> Vec<Cmd> {
    vec![Cmd::ScanLibrary(model.folders.clone()), Cmd::QueryDaemon]
}

pub fn update(m: &mut Model, msg: Msg) -> Vec<Cmd> {
    m.dirty = true;
    match msg {
        Msg::Key(key) => {
            // A key only redraws if it clears a toast or its Action
            // changes something (Actions set `dirty` themselves).
            m.dirty = m.toast.take().is_some();
            match keymap::action(m.mode, key) {
                Some(action) => perform(m, action),
                None => Vec::new(),
            }
        }
        Msg::Resize { width, height } => {
            m.area = Rect::new(0, 0, width, height);
            scroll_to_selection(m);
            refresh(m, When::Soon)
        }
        Msg::LibraryScanned(library) => {
            m.library = library;
            m.scanning = false;
            m.selected = m.selected.min(m.library.len().saturating_sub(1));
            scroll_to_selection(m);
            refresh(m, When::Now)
        }
        Msg::Daemon(Ok(info)) => {
            m.rotation = Rotation::from_status(&info.status);
            m.target = m.target.min(info.monitors.len().saturating_sub(1));
            m.daemon = Daemon::Up {
                monitors: info.monitors,
            };
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
            m.toast = Some(match result {
                Ok(()) => Toast {
                    kind: ToastKind::Success,
                    text: format!("Applied to {monitor}"),
                },
                Err(err) => Toast {
                    kind: ToastKind::Error,
                    text: err,
                },
            });
            Vec::new()
        }
    }
}

/// Carries out `action`. Sets `m.dirty` if the screen changes.
fn perform(m: &mut Model, action: Action) -> Vec<Cmd> {
    // In the grid, up and down move a whole row.
    let row = if m.grid.on {
        layout::grid(m.area).cols
    } else {
        1
    };
    match action {
        Action::Down => {
            let last = m.library.len().saturating_sub(1);
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
        Action::Apply => apply(m),
        Action::Quit => {
            m.quit = true;
            Vec::new()
        }
    }
}

/// Highlights `index` (clamped to the list) and, once it settles, renders
/// its preview (or the grid's new tiles, if the grid scrolled).
fn select(m: &mut Model, index: usize) -> Vec<Cmd> {
    let index = index.min(m.library.len().saturating_sub(1));
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

fn apply(m: &mut Model) -> Vec<Cmd> {
    let Some(path) = m.selection().cloned() else {
        return Vec::new();
    };
    let Some(monitor) = m.targeted() else {
        let why = match &m.daemon {
            Daemon::Down(err) => err.clone(),
            _ => "vellumd has not reported any monitor yet".into(),
        };
        m.toast = Some(Toast {
            kind: ToastKind::Error,
            text: why,
        });
        m.dirty = true;
        return Vec::new();
    };
    vec![Cmd::Apply(ApplyJob {
        path,
        monitor: monitor.name.clone(),
        fit: m.fit,
    })]
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
    m.preview.state = PreviewState::Loading { path };
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

/// The selection's preview, then its neighbours' as prefetches.
fn preview_jobs(m: &Model) -> Vec<Cmd> {
    let PreviewState::Loading { path } = &m.preview.state else {
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
        .filter_map(|i| m.library.get(i));
    std::iter::once(job(Purpose::Preview, path))
        .chain(neighbours.map(|p| job(Purpose::Prefetch, p)))
        .collect()
}

/// The wallpapers on screen in the grid, in order.
pub fn visible(m: &Model) -> impl Iterator<Item = &PathBuf> {
    let grid = layout::grid(m.area);
    m.library
        .iter()
        .skip(m.grid.offset_row * grid.cols)
        .take(grid.page())
}

/// Asks for thumbnails of the tiles on screen that have none yet. Bumps
/// the grid generation, so tiles that scrolled away stop rendering.
fn request_tiles(m: &mut Model, when: When) -> Vec<Cmd> {
    let grid = layout::grid(m.area);
    if grid.tiles.is_empty() || m.library.is_empty() {
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
    let kept: HashSet<&PathBuf> = m.library.iter().skip(first).take(3 * page).collect();
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
