use std::path::{Path, PathBuf};

use vellum_ipc::{
    Direction, Easing, RotationOrder, RotationSettings, RotationSource, Schedule, Transition,
    TransitionKind,
};

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

#[test]
fn rotation_defaults_to_a_30_minute_library_shuffle() {
    let config = parse("").unwrap();
    assert_eq!(config.rotation, RotationSettings::default());
    assert_eq!(config.settings().rotation, config.rotation);
}

#[test]
fn rotation_and_its_overrides_are_read() {
    let config = parse(
        r#"
[rotation]
interval = "1h30m"
source = "favorites"
order = "sequential"
sync = true

[rotation.monitor.DP-2]
interval = 5
order = "shuffle"

[rotation.monitor."HDMI-A-1"]
source = "library"
"#,
    )
    .unwrap();
    let r = &config.rotation;
    assert_eq!(
        r.schedule,
        Schedule {
            interval_ms: 90 * 60 * 1000,
            source: RotationSource::Favorites,
            order: RotationOrder::Sequential,
        }
    );
    assert!(r.sync);
    // An Override inherits what it does not set from the global Rotation.
    assert_eq!(
        r.schedule_for("DP-2"),
        &Schedule {
            interval_ms: 5 * 60 * 1000,
            source: RotationSource::Favorites,
            order: RotationOrder::Shuffle,
        }
    );
    assert_eq!(r.schedule_for("HDMI-A-1").source, RotationSource::Library);
    assert_eq!(r.schedule_for("HDMI-A-1").interval_ms, 90 * 60 * 1000);
}

#[test]
fn intervals_take_units_or_whole_minutes() {
    for (text, ms) in [
        ("\"30m\"", 1_800_000),
        ("\"90s\"", 90_000),
        ("\"2h\"", 7_200_000),
        ("\"1m30s\"", 90_000),
        ("\"250ms\"", 250),
        ("\"1h 5m\"", 3_900_000),
        ("15", 900_000),
    ] {
        let config = parse(&format!("[rotation]\ninterval = {text}\n")).unwrap();
        assert_eq!(config.rotation.schedule.interval_ms, ms, "{text}");
    }
    for bad in ["\"soon\"", "\"10\"", "\"0s\"", "0", "\"5d\"", "-3"] {
        let err = parse(&format!("[rotation]\ninterval = {bad}\n")).unwrap_err();
        assert_eq!(
            err.key.as_deref(),
            Some("rotation.interval"),
            "{bad}: {err}"
        );
        assert!(err.message.contains("\"30m\""), "{bad}: {err}");
    }
}

#[test]
fn rotation_mistakes_name_their_override() {
    let err = parse("[rotation.monitor.DP-1]\nintervall = \"5m\"\n").unwrap_err();
    assert_eq!(err.key.as_deref(), Some("rotation.monitor.DP-1.intervall"));
    let err = parse("[rotation]\norder = \"random\"\n").unwrap_err();
    assert!(err.message.contains("\"sequential\""), "{err}");
    // `sync` is global only.
    let err = parse("[rotation.monitor.DP-1]\nsync = true\n").unwrap_err();
    assert_eq!(err.key.as_deref(), Some("rotation.monitor.DP-1.sync"));
}

#[test]
fn intervals_print_the_way_they_are_written() {
    assert_eq!(format_interval(1_800_000), "30m");
    assert_eq!(format_interval(90_000), "1m30s");
    assert_eq!(format_interval(5_400_000), "1h30m");
    assert_eq!(format_interval(250), "250ms");
    assert_eq!(format_interval(1_500), "1.5s");
}

#[test]
fn concurrent_saves_of_the_last_good_config_all_succeed() {
    // `vellum reload` and the Daemon's inotify `render --sync` can save the
    // last good Config at the same moment; neither may lose its rename.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state/vellum/config.last-good.toml");
    std::thread::scope(|scope| {
        for writer in 0..8 {
            let path = &path;
            scope.spawn(move || {
                for round in 0..200 {
                    let text = format!("# writer {writer} round {round}\n");
                    write_atomically(path, &text).unwrap();
                }
            });
        }
    });
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("# writer "), "{text}");
}

#[test]
fn keymap_entries_take_one_key_a_list_or_none() {
    let config = parse(
        "[keymap]\napply_all = \"A\"\ndown = [\"n\", \"ctrl-j\"]\ntop = \"g g\"\nquit = []\n",
    )
    .unwrap();
    let chord = |t: &str| keymap::parse_chord(t).unwrap();
    assert_eq!(
        config.keymap,
        [
            Remap {
                action: Action::ApplyAll,
                keys: vec![chord("A")]
            },
            Remap {
                action: Action::Down,
                keys: vec![chord("n"), chord("ctrl-j")]
            },
            Remap {
                action: Action::Quit,
                keys: vec![]
            },
            Remap {
                action: Action::Top,
                keys: vec![chord("g g")]
            },
        ]
    );
    assert!(parse("").unwrap().keymap.is_empty());
}

#[test]
fn keymap_mistakes_point_at_the_entry() {
    let err = parse("fit = \"fit\"\n\n[keymap]\napply = \"enterr\"\n").unwrap_err();
    assert_eq!(err.line, Some(4));
    assert_eq!(err.key.as_deref(), Some("keymap.apply"));
    assert!(err.message.contains("\"enterr\" is not a key"), "{err}");

    let err = parse("[keymap]\napplyy = \"a\"\n").unwrap_err();
    assert_eq!(err.key.as_deref(), Some("keymap.applyy"));
    assert!(err.message.contains("unknown action `applyy`"), "{err}");
    assert!(err.message.contains("apply_all"), "{err}");

    let err = parse("[keymap]\napply = \"x\"\nrandom = \"x\"\n").unwrap_err();
    assert_eq!(err.line, Some(3));
    assert!(err.message.contains("already bound to `apply`"), "{err}");
}

#[test]
fn the_same_key_may_mean_different_things_in_the_list_and_in_prompts() {
    let config = parse("[keymap]\napply = \"ctrl-o\"\nconfirm = \"ctrl-o\"\n").unwrap();
    assert_eq!(config.keymap.len(), 2);
}

const CONFIGURATION_DOC: &str = include_str!("../../../../docs/configuration.md");
const README: &str = include_str!("../../../../README.md");

/// The ```toml blocks in a Markdown document, in order.
fn toml_blocks(doc: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in doc.lines() {
        match (&mut current, line.trim_end()) {
            (None, "```toml") => current = Some(String::new()),
            (Some(block), "```") => {
                blocks.push(std::mem::take(block));
                current = None;
            }
            (Some(block), _) => {
                block.push_str(line);
                block.push('\n');
            }
            (None, _) => {}
        }
    }
    blocks
}

#[test]
fn every_config_example_in_the_docs_parses() {
    for (doc, name, at_least) in [
        (CONFIGURATION_DOC, "docs/configuration.md", 10),
        (README, "README.md", 1),
    ] {
        let blocks = toml_blocks(doc);
        assert!(
            blocks.len() >= at_least,
            "{name}: only {} examples",
            blocks.len()
        );
        for block in blocks {
            if let Err(err) = parse(&block) {
                panic!("{name}: this example is rejected:\n{block}\n{err}");
            }
        }
    }
}

#[test]
fn the_documented_complete_example_is_exactly_the_defaults() {
    let complete = &toml_blocks(CONFIGURATION_DOC)[0];
    assert_eq!(
        parse(complete).unwrap(),
        Config::defaults(Some(Path::new(HOME)))
    );
}

#[test]
fn every_tui_action_is_documented_in_the_keys_section() {
    let doc = CONFIGURATION_DOC;
    let keys = &doc[doc.find("## Keys").expect("a Keys section")..];
    for name in Action::names() {
        assert!(
            keys.contains(&format!("`{name}`")),
            "docs/configuration.md \"Keys\" does not mention the `{name}` Action"
        );
    }
}
