//! Previews: the shared render function, run at preview size.
//!
//! A preview is a scaled-down frame for the targeted Monitor: same aspect
//! ratio, same Fit mode, same letterbox, rendered by [`crate::render::render`].
//! So the preview shows exactly what Enter will put on screen.

use image::{RgbImage, RgbaImage};
use ratatui::layout::Size;
use ratatui_image::FontSize;

use super::msg::{PreviewJob, Screen};
use crate::render::{Target, render};

/// The largest frame with `monitor`'s aspect ratio that fits in `cells`
/// at `font`, in pixels. Never zero.
pub fn frame_size(cells: Size, font: FontSize, monitor: Screen) -> (u32, u32) {
    let avail_w = u64::from(cells.width) * u64::from(font.width);
    let avail_h = u64::from(cells.height) * u64::from(font.height);
    let (mw, mh) = (
        u64::from(monitor.width.max(1)),
        u64::from(monitor.height.max(1)),
    );
    let (w, h) = if avail_w * mh <= avail_h * mw {
        (avail_w, avail_w * mh / mw)
    } else {
        (avail_h * mw / mh, avail_h)
    };
    (w.max(1) as u32, h.max(1) as u32)
}

/// The cells a `pixels`-sized image covers at `font`.
pub fn cells_for(pixels: (u32, u32), font: FontSize) -> Size {
    let cells =
        |px: u32, cell: u16| u16::try_from(px.div_ceil(u32::from(cell.max(1)))).unwrap_or(u16::MAX);
    Size::new(cells(pixels.0, font.width), cells(pixels.1, font.height))
}

/// `image` as `job.monitor` would show it, sized to fit `job.cells`.
pub fn render_preview(image: &RgbaImage, job: &PreviewJob, font: FontSize) -> RgbImage {
    let (width, height) = frame_size(job.cells, font, job.monitor);
    let monitor_scale = f64::from(job.monitor.scale_120.max(1)) / 120.0;
    let target = Target {
        width,
        height,
        // Center draws one wallpaper pixel per logical pixel; shrink that
        // by the same factor as the whole frame.
        scale: monitor_scale * f64::from(width) / f64::from(job.monitor.width.max(1)),
    };
    let mut frame = vec![0u8; width as usize * height as usize * 4];
    render(image, target, job.fit, job.letterbox, &mut frame);
    // XRGB8888 is B, G, R, X in memory.
    let rgb = frame
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[b, g, r, _]| [r, g, b])
        .collect();
    RgbImage::from_raw(width, height, rgb).expect("frame is width*height pixels")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use image::Rgba;

    use super::*;
    use crate::render::FitMode;

    const FONT: FontSize = FontSize::new(10, 20);

    fn job(fit: FitMode, monitor: Screen) -> PreviewJob {
        PreviewJob {
            generation: 1,
            path: PathBuf::from("/w/x.png"),
            cells: Size::new(40, 15),
            monitor,
            fit,
            letterbox: [0x1a, 0x1b, 0x2e],
        }
    }

    const FOUR_BY_THREE: Screen = Screen {
        width: 1600,
        height: 1200,
        scale_120: 120,
    };

    #[test]
    fn the_preview_has_the_monitors_shape_and_fits_its_pane() {
        // 40x15 cells of 10x20 px = 400x300 px; a 4:3 Monitor fills it.
        assert_eq!(
            frame_size(Size::new(40, 15), FONT, FOUR_BY_THREE),
            (400, 300)
        );
        // A 16:9 Monitor in the same pane is width-bound.
        assert_eq!(
            frame_size(Size::new(40, 15), FONT, Screen::FALLBACK),
            (400, 225)
        );
        // A tall pane is height-bound.
        assert_eq!(
            frame_size(Size::new(80, 5), FONT, FOUR_BY_THREE),
            (133, 100)
        );
        assert_eq!(cells_for((400, 225), FONT), Size::new(40, 12));
    }

    #[test]
    fn a_fit_preview_shows_the_letterbox_bars_the_monitor_will() {
        // A 2:1 red wallpaper on a 4:3 Monitor in Fit mode: bars above and
        // below in the letterbox color, red in the middle.
        let red = RgbaImage::from_pixel(200, 100, Rgba([255, 0, 0, 255]));
        let preview = render_preview(&red, &job(FitMode::Fit, FOUR_BY_THREE), FONT);
        assert_eq!(preview.dimensions(), (400, 300));
        assert_eq!(preview.get_pixel(200, 10).0, [0x1a, 0x1b, 0x2e]);
        assert_eq!(preview.get_pixel(200, 150).0, [255, 0, 0]);
        assert_eq!(preview.get_pixel(200, 290).0, [0x1a, 0x1b, 0x2e]);

        // Fill covers the whole preview instead.
        let preview = render_preview(&red, &job(FitMode::Fill, FOUR_BY_THREE), FONT);
        assert_eq!(preview.get_pixel(200, 10).0, [255, 0, 0]);
    }

    #[test]
    fn a_center_preview_is_true_to_the_monitors_scale() {
        // 800x600 px on a 1600x1200 Monitor at 1x covers half of each axis,
        // so in a 400x300 preview it covers 200x150.
        let red = RgbaImage::from_pixel(800, 600, Rgba([255, 0, 0, 255]));
        let preview = render_preview(&red, &job(FitMode::Center, FOUR_BY_THREE), FONT);
        assert_eq!(preview.get_pixel(200, 150).0, [255, 0, 0]);
        assert_eq!(preview.get_pixel(110, 150).0, [255, 0, 0]);
        assert_eq!(preview.get_pixel(90, 150).0, [0x1a, 0x1b, 0x2e]);
    }
}
