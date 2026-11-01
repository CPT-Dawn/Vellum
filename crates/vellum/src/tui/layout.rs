//! Where everything goes. Shared by the view (to draw) and update (to size
//! preview requests and hit-test clicks), so a preview is always rendered
//! for the cells it will occupy and a click lands on what is drawn there.
//!
//! Three layouts, by terminal size:
//! - wide (80 columns or more): the list beside the preview;
//! - narrow: the list or the preview, full width, `p` switching;
//! - too small (under [`MIN`]): only a message saying so.

use ratatui::layout::{Constraint, Flex, Layout, Margin, Rect, Size};

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

/// Below this many columns the list and the preview take turns.
pub const NARROW: u16 = 80;
/// The smallest terminal the TUI draws in.
pub const MIN: Size = Size::new(40, 10);

/// Whether `area` is too narrow for the list and the preview side by side.
pub fn is_narrow(area: Rect) -> bool {
    area.width < NARROW
}

/// Whether `area` is too small to draw anything but a message.
pub fn too_small(area: Rect) -> bool {
    area.width < MIN.width || area.height < MIN.height
}

/// Where everything goes in `area`. On a narrow terminal the body holds
/// the list, or the preview if `preview_only`; the other is empty.
pub fn areas(area: Rect, preview_only: bool) -> Areas {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let [list, preview] = if !is_narrow(area) {
        Layout::horizontal([Constraint::Percentage(35), Constraint::Fill(1)]).areas(body)
    } else if preview_only {
        [Rect { width: 0, ..body }, body]
    } else {
        [
            body,
            Rect {
                width: 0,
                x: body.right(),
                ..body
            },
        ]
    };
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
pub fn list_rows(area: Rect, preview_only: bool) -> u16 {
    let list = areas(area, preview_only).list;
    if list.width == 0 {
        return 0;
    }
    list.height.saturating_sub(2)
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
    let a = areas(area, false);
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

/// Rows of the Rotation panel at most: running, scope, interval, source,
/// order.
pub const PANEL_ROWS: u16 = 5;

/// Where the Rotation panel goes: a box centered over the body.
/// Returns the box (border included) and the rows inside it, one per
/// setting, then a blank line and a line for notes.
pub fn panel(area: Rect) -> (Rect, Rect) {
    let a = areas(area, false);
    let body = Rect {
        y: a.header.bottom(),
        height: a.footer.y.saturating_sub(a.header.bottom()),
        ..area
    };
    let [row] = Layout::vertical([Constraint::Length(PANEL_ROWS + 6)])
        .flex(Flex::Center)
        .areas(body);
    let [outer] = Layout::horizontal([Constraint::Length(body.width.min(56))])
        .flex(Flex::Center)
        .areas(row);
    (outer, outer.inner(Margin::new(2, 2)))
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

/// Cells of a grid tile's thumbnail.
pub const TILE_IMAGE: Size = Size::new(24, 6);
/// A tile: the thumbnail plus one row for its name.
const TILE: Size = Size::new(TILE_IMAGE.width, TILE_IMAGE.height + 1);
/// Blank cells between tiles.
const GAP: u16 = 1;

/// Where the thumbnail grid goes. The header and footer are the same as in
/// [`areas`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridAreas {
    /// The grid pane, border included.
    pub pane: Rect,
    /// Inside the pane, above the metadata strip.
    pub tiles: Rect,
    /// The metadata strip at the bottom of the pane.
    pub meta: Rect,
    /// Tiles per row (at least 1).
    pub cols: usize,
    /// Rows of tiles on screen (at least 1).
    pub rows: usize,
}

pub fn grid(area: Rect) -> GridAreas {
    let [_, pane, _] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let inner = pane.inner(ratatui::layout::Margin::new(1, 1));
    let [tiles, meta] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(META_ROWS)]).areas(inner);
    let fit = |space: u16, tile: u16| usize::from((space + GAP) / (tile + GAP)).max(1);
    GridAreas {
        pane,
        tiles,
        meta,
        // One column of padding on the left.
        cols: fit(tiles.width.saturating_sub(1), TILE.width),
        rows: fit(tiles.height, TILE.height),
    }
}

impl GridAreas {
    /// The thumbnail and name cells of the tile `index` places after the
    /// top-left one on screen, clipped to the grid.
    pub fn tile(&self, index: usize) -> (Rect, Rect) {
        let col = u16::try_from(index % self.cols).unwrap_or(u16::MAX);
        let row = u16::try_from(index / self.cols).unwrap_or(u16::MAX);
        let x = self
            .tiles
            .x
            .saturating_add(1 + col.saturating_mul(TILE.width + GAP));
        let y = self
            .tiles
            .y
            .saturating_add(row.saturating_mul(TILE.height + GAP));
        let image = Rect::new(x, y, TILE_IMAGE.width, TILE_IMAGE.height).intersection(self.tiles);
        let name = Rect::new(x, y.saturating_add(TILE_IMAGE.height), TILE.width, 1)
            .intersection(self.tiles);
        (image, name)
    }

    /// How many tiles fit on screen.
    pub fn page(&self) -> usize {
        self.cols * self.rows
    }
}
