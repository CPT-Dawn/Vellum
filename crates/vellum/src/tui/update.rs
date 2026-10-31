//! The update function: the only place the Model changes.
//!
//! `update(&mut Model, Msg) -> Vec<Cmd>` is pure: no I/O, no clocks, no
//! threads. Keys go through [`keymap::action`] to an [`Action`], and
//! [`perform`] turns an Action into Model changes and Cmds.

use ratatui::layout::Rect;

use super::keymap::{self, Action};
use super::layout;
use super::model::{Daemon, Model, PreviewState, Rotation, Toast, ToastKind};
use super::msg::{ApplyJob, Cmd, Msg, PreviewJob, Screen};

#[cfg(test)]
mod tests;

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
            request_preview(m)
        }
        Msg::LibraryScanned(library) => {
            m.library = library;
            m.scanning = false;
            m.selected = m.selected.min(m.library.len().saturating_sub(1));
            request_preview(m)
        }
        Msg::Daemon(Ok(info)) => {
            m.rotation = Rotation::from_status(&info.status);
            m.target = m.target.min(info.monitors.len().saturating_sub(1));
            m.daemon = Daemon::Up {
                monitors: info.monitors,
            };
            request_preview(m)
        }
        Msg::Daemon(Err(err)) => {
            m.daemon = Daemon::Down(err);
            Vec::new()
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
    match action {
        Action::Down => select(m, m.selected.saturating_add(1)),
        Action::Up => select(m, m.selected.saturating_sub(1)),
        Action::Apply => apply(m),
        Action::Quit => {
            m.quit = true;
            Vec::new()
        }
    }
}

/// Highlights `index` (clamped to the list) and asks for its preview.
fn select(m: &mut Model, index: usize) -> Vec<Cmd> {
    let index = index.min(m.library.len().saturating_sub(1));
    if index == m.selected {
        return Vec::new();
    }
    m.selected = index;
    m.dirty = true;
    scroll_to_selection(m);
    request_preview(m)
}

/// Moves `list_offset` just enough to keep the selection in view.
fn scroll_to_selection(m: &mut Model) {
    let rows = usize::from(layout::list_rows(m.area)).max(1);
    if m.selected < m.list_offset {
        m.list_offset = m.selected;
    } else if m.selected >= m.list_offset + rows {
        m.list_offset = m.selected + 1 - rows;
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
/// the preview pane. Bumps the generation so older results are dropped.
fn request_preview(m: &mut Model) -> Vec<Cmd> {
    let Some(path) = m.selection().cloned() else {
        m.preview.state = PreviewState::Empty;
        return Vec::new();
    };
    let image = layout::areas(m.area).image;
    if image.is_empty() {
        return Vec::new();
    }
    m.preview.generation += 1;
    m.preview.state = PreviewState::Loading { path: path.clone() };
    vec![Cmd::RenderPreview(PreviewJob {
        generation: m.preview.generation,
        path,
        cells: image.as_size(),
        monitor: m.targeted().map_or(Screen::FALLBACK, Screen::from),
        fit: m.fit,
        letterbox: m.letterbox,
    })]
}
