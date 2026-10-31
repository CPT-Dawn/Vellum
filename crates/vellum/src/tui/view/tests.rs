//! Snapshots of the screen at 80x24 and 120x40, plus the theme roles the
//! screen uses. Review changes with `cargo insta review`.

use ratatui::style::{Color, Modifier};

use super::super::harness::*;
use super::super::msg::Msg;
use super::super::theme::Theme;

/// Snapshots `h` at both standard sizes, built by `setup`.
fn both_sizes(name: &str, setup: impl Fn(u16, u16) -> Harness) {
    for (w, h) in [(80, 24), (120, 40)] {
        let harness = setup(w, h);
        insta::assert_snapshot!(format!("{name}_{w}x{h}"), harness.screen());
    }
}

#[test]
fn list_and_preview() {
    both_sizes("list_and_preview", |w, h| {
        let mut harness = Harness::started(w, h);
        harness.press("j");
        harness
    });
}

#[test]
fn scanning_before_anything_answers() {
    both_sizes("scanning", |w, h| {
        let mut harness = Harness::new(w, h);
        harness.start();
        harness
    });
}

#[test]
fn list_before_the_preview_finishes() {
    both_sizes("preview_loading", |w, h| {
        let mut harness = Harness::new(w, h);
        let library = harness.fake.library.clone();
        let daemon = harness.fake.daemon.clone();
        harness.update(Msg::Daemon(daemon));
        harness.update(Msg::LibraryScanned(library));
        harness
    });
}

#[test]
fn applied_toast() {
    both_sizes("applied", |w, h| {
        let mut harness = Harness::started(w, h);
        harness.press("enter");
        harness
    });
}

#[test]
fn daemon_down() {
    both_sizes("daemon_down", |w, h| {
        let fake = Fake {
            daemon: Err("vellumd did not start".into()),
            ..Fake::default()
        };
        let mut harness = Harness::started_with(w, h, fake);
        harness.press("enter");
        harness
    });
}

#[test]
fn corrupt_wallpaper() {
    both_sizes("decode_error", |w, h| {
        let fake = Fake {
            broken: vec![wallpaper("aurora.png")],
            ..Fake::default()
        };
        Harness::started_with(w, h, fake)
    });
}

#[test]
fn empty_library() {
    both_sizes("empty_library", |w, h| {
        let fake = Fake {
            library: Vec::new(),
            ..Fake::default()
        };
        Harness::started_with(w, h, fake)
    });
}

/// A Library of `n` wallpapers named 000.png, 001.png, ...
/// with `broken` ones failing to decode, in the grid.
fn grid_harness(w: u16, h: u16, n: usize, broken: &[&str]) -> Harness {
    let fake = Fake {
        library: (0..n).map(|i| wallpaper(&format!("{i:03}.png"))).collect(),
        broken: broken.iter().map(|b| wallpaper(b)).collect(),
        ..Fake::default()
    };
    let mut harness = Harness::started_with(w, h, fake);
    harness.press("g");
    harness
}

#[test]
fn thumbnail_grid() {
    both_sizes("grid", |w, h| {
        let mut harness = grid_harness(w, h, 50, &[]);
        harness.press("l");
        harness
    });
}

#[test]
fn thumbnail_grid_scrolled_with_tiles_loading_and_broken() {
    both_sizes("grid_scrolled", |w, h| {
        let mut harness = grid_harness(w, h, 50, &["006.png", "013.png"]);
        let rows = super::super::layout::grid(harness.model.area).rows;
        for _ in 0..rows {
            harness.press("j");
        }
        // The next scroll is still settling: its new row shows as loading.
        harness.update(Msg::Key(key("j")));
        harness
    });
}

#[test]
fn the_list_scrolls_to_keep_the_selection_in_view() {
    let fake = Fake {
        library: (0..40)
            .map(|i| wallpaper(&format!("wall-{i:02}.png")))
            .collect(),
        ..Fake::default()
    };
    let mut h = Harness::started_with(80, 24, fake);
    for _ in 0..25 {
        h.press("j");
    }
    let screen = h.screen();
    assert!(screen.contains("› wall-25.png"), "{screen}");
    assert!(!screen.contains("wall-00.png"), "{screen}");
    // Moving back up moves the cursor, not the list.
    h.press("k");
    let screen = h.screen();
    assert!(screen.contains("› wall-24.png"), "{screen}");
    assert!(screen.contains("wall-25.png"), "{screen}");
}

/// The cell holding the first character of `text` on screen.
fn find(buffer: &ratatui::buffer::Buffer, text: &str) -> (u16, u16) {
    let first = text.chars().next().unwrap().to_string();
    for y in 0..buffer.area.height {
        let row: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        if let Some(byte) = row.find(text) {
            let x = row[..byte].chars().count() as u16;
            assert_eq!(buffer[(x, y)].symbol(), first);
            return (x, y);
        }
    }
    panic!("{text:?} is not on screen");
}

const MINT: Color = Color::Rgb(0x8D, 0xE3, 0xDC);
const BLUE: Color = Color::Rgb(0x7A, 0xA2, 0xF7);
const TEXT: Color = Color::Rgb(0xC0, 0xCA, 0xF5);
const SUBTEXT: Color = Color::Rgb(0xA9, 0xB1, 0xD6);
const INK: Color = Color::Rgb(0x0B, 0x0D, 0x14);

#[test]
fn midnight_bloom_roles_on_screen() {
    let mut h = Harness::started(80, 24);
    h.press("j");
    let buffer = h.buffer();

    // Selection: blue, and bold so it never relies on color alone.
    let selected = &buffer[find(&buffer, "beach/dunes.jpg")];
    assert_eq!(selected.bg, BLUE);
    assert!(selected.modifier.contains(Modifier::BOLD));
    // Other rows: plain text on ink.
    let row = &buffer[find(&buffer, "aurora.png")];
    assert_eq!((row.fg, row.bg), (TEXT, INK));
    // Focus: the focused pane's border, the targeted Monitor, key glyphs.
    assert_eq!(buffer[(0, 1)].fg, MINT);
    assert_eq!(buffer[find(&buffer, "● DP-1")].fg, MINT);
    assert_eq!(buffer[find(&buffer, "⏎")].fg, MINT);
    // Secondary: metadata and footer labels.
    assert_eq!(buffer[find(&buffer, "3840×2160")].fg, SUBTEXT);
    assert_eq!(buffer[find(&buffer, "apply")].fg, SUBTEXT);
}

#[test]
fn semantic_colors_only_for_semantic_states() {
    let rose = Color::Rgb(0xF7, 0x76, 0x8E);
    let green = Color::Rgb(0x9E, 0xCE, 0x6A);
    let orange = Color::Rgb(0xFF, 0x9E, 0x64);
    let semantic = [rose, green, orange];

    let h = Harness::started(120, 40);
    let buffer = h.buffer();
    assert!(
        buffer.content.iter().all(|c| !semantic.contains(&c.fg)),
        "a calm screen uses no semantic color"
    );

    let mut h = Harness::started(120, 40);
    h.press("enter");
    let buffer = h.buffer();
    assert_eq!(buffer[find(&buffer, "✓ Applied to DP-1")].fg, green);
}

#[test]
fn no_color_marks_the_selection_by_style() {
    let mut h = Harness::started(80, 24);
    h.model.theme = Theme::no_color();
    let buffer = h.buffer();
    assert!(
        buffer
            .content
            .iter()
            .all(|c| c.fg == Color::Reset && c.bg == Color::Reset),
        "NO_COLOR draws no color"
    );
    let selected = &buffer[find(&buffer, "aurora.png")];
    assert!(selected.modifier.contains(Modifier::REVERSED));
}
