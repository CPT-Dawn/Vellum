use std::path::{Path, PathBuf};

use vellum_ipc::{Direction, Easing, Transition, TransitionKind};

use super::*;
use crate::render::FitMode;

const HOME: &str = "/home/me";

fn parse(text: &str) -> Result<Config, ConfigError> {
    Config::parse(text, Some(Path::new(HOME)))
}

#[test]
fn an_empty_config_is_all_defaults() {
    let config = parse("").unwrap();
    assert_eq!(
        config.library,
        [PathBuf::from("/home/me/Pictures/Wallpapers")]
    );
    assert_eq!(config.fit, FitMode::Fill);
    assert_eq!(config.letterbox, [0, 0, 0]);
    assert_eq!(config.transition, Transition::default());
    assert_eq!(config.preview_cache_mb, 200);
    assert_eq!(config, Config::defaults(Some(Path::new(HOME))));
}

#[test]
fn every_key_is_read() {
    let config = parse(
        r##"
library = ["~/Pictures/Walls", "/srv/art"]
fit = "center"
letterbox = "#1A1b2e"
on_change = "matugen image {path}"

[preview]
cache_mb = 50

[transition]
type = "slide"
direction = "up"
duration = 250
easing = "ease-in-out"
"##,
    )
    .unwrap();
    assert_eq!(
        config.library,
        [
            PathBuf::from("/home/me/Pictures/Walls"),
            PathBuf::from("/srv/art")
        ]
    );
    assert_eq!(config.fit, FitMode::Center);
    assert_eq!(config.letterbox, [0x1a, 0x1b, 0x2e]);
    assert_eq!(
        config.transition,
        Transition {
            kind: TransitionKind::Slide,
            direction: Direction::Up,
            duration_ms: 250,
            easing: Easing::EaseInOut,
        }
    );
    assert_eq!(config.settings().transition, config.transition);
    assert_eq!(config.preview_cache_mb, 50);
    let hook = config.on_change.expect("on_change was read");
    assert_eq!(hook.argv("/w.png", "DP-1"), ["matugen", "image", "/w.png"]);
}

#[test]
fn an_unparseable_on_change_points_at_it() {
    let err = parse("fit = \"fit\"\non_change = \"wal -i '{path}\"\n").unwrap_err();
    assert_eq!(err.line, Some(2));
    assert_eq!(err.key.as_deref(), Some("on_change"));
    assert!(err.message.contains("unclosed quote"), "{err}");
}

#[test]
fn a_partial_section_keeps_the_other_defaults() {
    let config = parse("[transition]\nduration = 0\n").unwrap();
    assert_eq!(config.transition.kind, TransitionKind::Fade);
    assert_eq!(config.transition.duration_ms, 0);
}

#[test]
fn a_wrong_type_names_the_line_the_key_and_what_was_expected() {
    let text = "fit = \"fill\"\n\n[transition]\nduration = \"slow\"\n";
    let err = parse(text).unwrap_err();
    assert_eq!(err.line, Some(4));
    assert_eq!(err.key.as_deref(), Some("transition.duration"));
    assert_eq!(
        err.message,
        "invalid type: string \"slow\", expected a whole number of milliseconds, like 600"
    );
    let negative = parse("[transition]\nduration = -5\n").unwrap_err();
    assert!(negative.message.contains("milliseconds"), "{negative}");
    let text = err.to_string();
    assert!(
        text.starts_with("line 4, `transition.duration`: "),
        "{text}"
    );
    assert!(text.contains("4 | duration = \"slow\""), "{text}");
    assert!(text.contains("^^^^^^"), "{text}");
}

#[test]
fn an_unknown_name_lists_the_valid_ones() {
    let err = parse("fit = \"zoom\"\n").unwrap_err();
    assert_eq!(err.line, Some(1));
    assert_eq!(err.key.as_deref(), Some("fit"));
    assert_eq!(
        err.message,
        "unknown Fit mode \"zoom\", expected one of \"fill\", \"fit\", \"center\", \"stretch\""
    );

    let err = parse("[transition]\neasing = \"bouncy\"\n").unwrap_err();
    assert_eq!(err.key.as_deref(), Some("transition.easing"));
    assert!(err.message.contains("\"ease-in-out\""), "{err}");
}

#[test]
fn an_unknown_key_is_an_error_not_ignored() {
    let err = parse("[transition]\ntyp = \"wipe\"\n").unwrap_err();
    assert_eq!(err.line, Some(2));
    assert_eq!(err.key.as_deref(), Some("transition.typ"));
    assert!(err.message.contains("`type`"), "{err}");

    let err = parse("[transitions]\n").unwrap_err();
    assert_eq!(err.key.as_deref(), Some("transitions"));
}

#[test]
fn a_bad_color_shows_the_expected_format() {
    let err = parse("letterbox = \"red\"\n").unwrap_err();
    assert_eq!(err.key.as_deref(), Some("letterbox"));
    assert!(err.message.contains("\"#rrggbb\""), "{err}");
}

#[test]
fn a_relative_library_folder_is_rejected() {
    let err = parse("library = [\n  \"/ok\",\n  \"Pictures\",\n]\n").unwrap_err();
    assert_eq!(err.line, Some(3));
    assert!(
        err.message
            .contains("absolute path or one starting with ~/")
    );
}

#[test]
fn broken_toml_points_at_the_line() {
    let err = parse("fit = \"fill\"\n[transition\n").unwrap_err();
    assert_eq!(err.line, Some(2));
}

#[test]
fn names_round_trip() {
    for (name, kind) in TransitionKind::NAMES {
        assert_eq!(kind.name(), *name);
    }
    assert_eq!(Easing::EaseOut.name(), "ease-out");
}

#[test]
fn the_theme_defaults_to_midnight_and_can_follow_the_terminal() {
    assert_eq!(parse("").unwrap().theme, ThemeName::Midnight);
    assert_eq!(
        parse("theme = \"terminal\"\n").unwrap().theme,
        ThemeName::Terminal
    );
    let err = parse("theme = \"solarized\"\n").unwrap_err();
    assert_eq!(err.key.as_deref(), Some("theme"));
    assert_eq!(
        err.message,
        "unknown theme \"solarized\", expected one of \"midnight\", \"terminal\""
    );
}
