//! Where everything goes. Shared by the view (to draw) and update (to size
//! preview requests and hit-test clicks), so a preview is always rendered
//! for the cells it will occupy and a click lands on what is drawn there.
//!
//! #13 adds the narrow (list only) and too-small layouts here.

use ratatui::layout::{Constraint, Flex, Layout, Margin, Rect};

/// Rows under the preview image for the metadata strip.
pub const META_ROWS: u16 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Areas {
    pub header: Rect,
    /// The Library list, border included.
    pub list: Rect,
    /// The preview pane, border included.
    pub preview: Rect,
    /// Inside the preview pane, above the metadata strip.
    pub image: Rect,
    /// The metadata strip at the bottom of the preview pane.
    pub meta: Rect,
    pub footer: Rect,
}

pub fn areas(area: Rect) -> Areas {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let [list, preview] =
        Layout::horizontal([Constraint::Percentage(35), Constraint::Fill(1)]).areas(body);
    let inner = preview.inner(ratatui::layout::Margin::new(1, 1));
    let [image, meta] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(META_ROWS)]).areas(inner);
    Areas {
        header,
        list,
        preview,
        image,
        meta,
        footer,
    }
}

/// How many wallpapers the list shows at once in `area`.
pub fn list_rows(area: Rect) -> u16 {
    areas(area).list.height.saturating_sub(2)
}

/// Where the help overlay and the palette go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overlay {
    /// The box, border included.
    pub outer: Rect,
    /// The query line.
    pub prompt: Rect,
    /// The rows under it.
    pub rows: Rect,
}

/// A box centered over the body, about two thirds of it.
pub fn overlay(area: Rect) -> Overlay {
    let a = areas(area);
    let body = Rect {
        y: a.header.bottom(),
        height: a.footer.y.saturating_sub(a.header.bottom()),
        ..area
    };
    let width = (body.width * 2 / 3).clamp(body.width.min(44), body.width.min(72));
    let height = (body.height * 4 / 5).max(body.height.min(8));
    let [row] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(body);
    let [outer] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(row);
    let inner = outer.inner(Margin::new(2, 1));
    let [prompt, _, rows] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(inner);
    Overlay {
        outer,
        prompt,
        rows,
    }
}

/// The first palette row shown, so `selected` stays within `height` rows.
pub fn palette_offset(selected: usize, height: u16) -> usize {
    selected.saturating_sub(usize::from(height).saturating_sub(1))
}

/// Column where the header's Monitor labels start, after " vellum  ".
const TABS_X: u16 = 9;
/// Columns before a label (the `●` and a space) and after it.
pub const TAB_MARK: u16 = 2;
const TAB_GAP: u16 = 2;

/// Each Monitor's header label (`DP-1 2560×1440`) and the column its `●`
/// starts at. Shared by the view (to draw) and update (to hit-test
/// clicks); a label is clickable from its `●` to its end.
pub fn monitor_tabs(monitors: &[vellum_ipc::MonitorInfo]) -> Vec<(u16, String)> {
    let mut x = TABS_X;
    monitors
        .iter()
        .map(|m| {
            let label = format!("{} {}×{}", m.name, m.physical_width, m.physical_height);
            let at = x;
            x = x.saturating_add(TAB_MARK + width(&label) + TAB_GAP);
            (at, label)
        })
        .collect()
}

/// Columns `text` takes.
pub fn width(text: &str) -> u16 {
    u16::try_from(text.chars().count()).unwrap_or(u16::MAX)
}
