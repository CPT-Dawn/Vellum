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
        harness.advance(150);
        harness
    });
}

#[test]
fn nothing_flashes_before_the_spinner_delay() {
    let mut h = Harness::new(80, 24);
    h.start();
    let screen = h.screen();
    assert!(!screen.contains("Scanning"), "{screen}");
    assert!(!screen.contains('⠋'), "{screen}");
}

#[test]
fn search_with_highlighted_matches() {
    both_sizes("search", |w, h| {
        let mut harness = Harness::started(w, h);
        harness.press("/");
        harness.typed("be");
        harness
    });
}

#[test]
fn search_kept_after_enter() {
    both_sizes("search_kept", |w, h| {
        let mut harness = Harness::started(w, h);
        harness.press("/");
        harness.typed("tide");
        harness.press("enter");
        harness
    });
}

#[test]
fn search_with_no_matches() {
    both_sizes("search_no_matches", |w, h| {
        let mut harness = Harness::started(w, h);
        harness.press("/");
        harness.typed("xyz");
        harness
    });
}

#[test]
fn favorites_filter() {
    both_sizes("favorites_only", |w, h| {
        let fake = Fake::default();
        *fake.favorites.lock().unwrap() =
            vec![wallpaper("beach/tide.webp"), wallpaper("aurora.png")];
        let mut harness = Harness::started_with(w, h, fake);
        harness.press("F");
        harness
    });
}

#[test]
fn favorites_marked_in_the_list() {
    let mut h = Harness::started(80, 24);
    h.press("j");
    h.press("f");
    let screen = h.screen();
    assert!(screen.contains("beach/dunes.jpg ★"), "{screen}");
    assert!(screen.contains("dunes.jpg ★  ~/Pictures"), "{screen}");
}

#[test]
fn help_overlay() {
    both_sizes("help", |w, h| {
        let mut harness = Harness::started(w, h);
        harness.press("?");
        harness
    });
}

#[test]
fn help_overlay_searched() {
    both_sizes("help_searched", |w, h| {
        let mut harness = Harness::started(w, h);
        harness.press("?");
        harness.typed("monitor");
        harness
    });
}

#[test]
fn command_palette() {
    both_sizes("palette", |w, h| {
        let mut harness = Harness::started(w, h);
        harness.press(":");
        harness
    });
}

#[test]
fn command_palette_filtered() {
    both_sizes("palette_filtered", |w, h| {
        let mut harness = Harness::started(w, h);
        harness.press(":");
        harness.typed("fav");
        harness.press("down");
        harness
    });
}

#[test]
fn a_remap_shows_in_the_footer_help_and_palette() {
    let toml = "[keymap]\napply = [\"o\", \"enter\"]\nhelp = \"f1\"\n";
    let h = Harness::started_with_config(120, 40, toml);
    let footer = h.screen().lines().last().unwrap().to_owned();
    assert!(footer.contains("o apply"), "{footer}");
    assert!(footer.contains("f1 help"), "{footer}");
    assert!(!footer.contains("? help"), "{footer}");

    let mut h = Harness::started_with_config(120, 40, toml);
    h.press("f1");
    let help = h.screen();
    assert!(help.contains("o, ⏎"), "{help}");

    let mut h = Harness::started_with_config(120, 40, toml);
    h.press(":");
    h.typed("targeted");
    let palette = h.screen();
    let row = palette
        .lines()
        .find(|l| l.contains("Apply to the targeted monitor"))
        .unwrap();
    assert!(row.contains("o, ⏎"), "{row}");
}

#[test]
fn a_search_match_is_drawn_in_the_focus_role() {
    let mut h = Harness::started(80, 24);
    h.press("/");
    // dunes and tide match equally; dunes (first) is selected, so look at
    // tide, drawn without the selection style.
    h.typed("beach");
    let buffer = h.buffer();
    let (x, y) = find(&buffer, "beach/tide.webp");
    let hit = &buffer[(x, y)];
    assert_eq!(hit.fg, MINT);
    assert!(hit.modifier.contains(Modifier::UNDERLINED));
    assert_eq!(
        buffer[(x + 6, y)].fg,
        TEXT,
        "unmatched characters stay plain"
    );
}

#[test]
fn list_before_the_preview_finishes() {
    both_sizes("preview_loading", |w, h| {
        let mut harness = Harness::new(w, h);
        let library = harness.fake.library.clone();
        let daemon = harness.fake.daemon.clone();
        harness.update(Msg::Daemon(daemon));
        harness.update(Msg::LibraryScanned(library));
        harness.model.now += std::time::Duration::from_millis(200);
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
    let cmds = harness.toggle_grid();
    harness.run(cmds);
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

#[test]
fn midnight_bloom_roles_on_screen() {
    let mut h = Harness::started(80, 24);
    h.press("j");
    let buffer = h.buffer();

    // Selection: blue, and bold so it never relies on color alone.
    let selected = &buffer[find(&buffer, "beach/dunes.jpg")];
    assert_eq!(selected.bg, BLUE);
    assert!(selected.modifier.contains(Modifier::BOLD));
    // Other rows: plain text on the terminal's own background.
    let row = &buffer[find(&buffer, "aurora.png")];
    assert_eq!((row.fg, row.bg), (TEXT, Color::Reset));
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

#[test]
fn rotation_panel() {
    both_sizes("rotation_panel", |w, h| {
        let mut harness = Harness::started(w, h);
        harness.press("s");
        harness.press("j");
        harness.press("j");
        harness
    });
}

#[test]
fn rotation_panel_editing_an_override() {
    both_sizes("rotation_panel_override", |w, h| {
        let mut harness = Harness::started(w, h);
        harness.press("s");
        harness.press("enter");
        harness.press("j");
        harness.press("l");
        harness.press("j");
        harness.press("j");
        harness.press("l");
        harness
    });
}

#[test]
fn welcome_screen() {
    both_sizes("welcome", Harness::first_run);
}

#[test]
fn welcome_screen_completing_a_typed_folder() {
    both_sizes("welcome_completing", |w, h| {
        let mut harness = Harness::first_run(w, h);
        harness.typed("~/Pi");
        harness.press("tab");
        harness
    });
}

#[test]
fn no_favorites_yet() {
    both_sizes("no_favorites", |w, h| {
        let mut harness = Harness::started(w, h);
        harness.press("F");
        harness
    });
}

#[test]
fn narrow_layouts() {
    for (w, h) in [(60, 24), (40, 10)] {
        let mut harness = Harness::started(w, h);
        insta::assert_snapshot!(format!("narrow_list_{w}x{h}"), harness.screen());
        harness.press("p");
        insta::assert_snapshot!(format!("narrow_preview_{w}x{h}"), harness.screen());
    }
}

#[test]
fn too_small() {
    for (w, h) in [(39, 24), (80, 9)] {
        let harness = Harness::started(w, h);
        insta::assert_snapshot!(format!("too_small_{w}x{h}"), harness.screen());
    }
}

#[test]
fn the_rotation_panel_marks_the_line_being_edited() {
    let mut h = Harness::started(120, 40);
    h.press("s");
    let buffer = h.buffer();
    let line = &buffer[find(&buffer, "Rotation  ")];
    assert_eq!(line.bg, BLUE);
    assert!(line.modifier.contains(Modifier::BOLD));
    assert_eq!(buffer[find(&buffer, "every 30m")].fg, TEXT);
}

#[test]
fn halfblock_previews_say_why_they_are_coarse() {
    both_sizes("coarse_preview", |w, h| {
        let mut harness = Harness::started(w, h);
        harness.model.coarse = true;
        harness.press("j");
        assert!(harness.screen().contains("coarse"), "{}", harness.screen());
        harness
    });
    // Narrow, the preview pane alone still has room for the hint.
    let mut harness = Harness::started(60, 24);
    harness.model.coarse = true;
    harness.press("p");
    insta::assert_snapshot!("coarse_preview_narrow_60x24", harness.screen());
}
