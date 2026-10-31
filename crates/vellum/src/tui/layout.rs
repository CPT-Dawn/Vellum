//! Where everything goes. Shared by the view (to draw) and update (to size
//! preview requests), so a preview is always rendered for the cells it
//! will occupy.
//!
//! #13 adds the narrow (list only) and too-small layouts here.

use ratatui::layout::{Constraint, Layout, Rect};

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
