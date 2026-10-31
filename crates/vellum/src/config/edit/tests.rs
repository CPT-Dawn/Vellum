use std::path::Path;

use super::*;

const HAND_WRITTEN: &str = r#"# My wallpapers
library = ["~/Pictures/Wallpapers"]  # synced with Nextcloud

fit = "fill"

[rotation]
# change often
interval = "30m"   # half an hour
source = "library"

[transition]
type = "wipe"
"#;

#[test]
fn a_set_changes_only_its_value_and_keeps_comments() {
    let out = apply(HAND_WRITTEN, &[Edit::set(&["rotation"], "interval", "10m")]).unwrap();
    assert_eq!(
        out,
        HAND_WRITTEN.replace("interval = \"30m\"", "interval = \"10m\"")
    );
}

#[test]
fn a_new_key_joins_its_existing_table() {
    let out = apply(
        HAND_WRITTEN,
        &[Edit::set(&["rotation"], "order", "sequential")],
    )
    .unwrap();
    assert_eq!(
        out,
        HAND_WRITTEN.replace(
            "source = \"library\"\n",
            "source = \"library\"\norder = \"sequential\"\n"
        )
    );
}

#[test]
fn an_override_gets_its_own_header_and_nothing_else_moves() {
    let out = apply(
        HAND_WRITTEN,
        &[Edit::set(
            &["rotation", "monitor", "DP-1"],
            "source",
            "favorites",
        )],
    )
    .unwrap();
    let block = "\n[rotation.monitor.DP-1]\nsource = \"favorites\"\n";
    assert!(out.contains(block), "{out}");
    assert_eq!(out.replace(block, ""), HAND_WRITTEN, "nothing else moved");
    assert!(!out.contains("[rotation.monitor]\n"), "{out}");
    let config = Config::parse(&out, Some(Path::new("/home/me"))).unwrap();
    assert_eq!(config.rotation.overrides[0].monitor, "DP-1");
}

#[test]
fn removing_an_override_leaves_the_rest() {
    let text = format!("{HAND_WRITTEN}\n[rotation.monitor.DP-1]\ninterval = \"5m\"\n");
    let out = apply(
        &text,
        &[Edit::Remove {
            table: vec!["rotation".into(), "monitor".into(), "DP-1".into()],
        }],
    )
    .unwrap();
    assert!(!out.contains("DP-1"), "{out}");
    assert!(out.starts_with(HAND_WRITTEN.trim_end()), "{out}");
}

#[test]
fn a_top_level_list_replaces_in_place_even_with_tables_below() {
    let edit = Edit::Set {
        table: Vec::new(),
        key: "library".into(),
        value: Value::List(vec!["~/Pictures".into()]),
    };
    let out = apply(HAND_WRITTEN, std::slice::from_ref(&edit)).unwrap();
    assert_eq!(
        out,
        HAND_WRITTEN.replace("[\"~/Pictures/Wallpapers\"]", "[\"~/Pictures\"]")
    );
    // And into an empty file.
    assert_eq!(apply("", &[edit]).unwrap(), "library = [\"~/Pictures\"]\n");
}

#[test]
fn an_inline_table_is_edited_where_it_is() {
    let out = apply(
        "rotation = { interval = \"30m\" }\n",
        &[Edit::set(&["rotation"], "interval", "1h")],
    )
    .unwrap();
    assert_eq!(out, "rotation = { interval = \"1h\" }\n");
}

#[test]
fn invalid_toml_is_never_rewritten() {
    let err = apply("fit = \n", &[Edit::set(&[], "fit", "fit")]).unwrap_err();
    assert!(err.contains("not valid TOML"), "{err}");
}

#[test]
fn save_refuses_to_write_a_config_that_would_not_parse() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config.toml");
    std::fs::write(&file, "fit = \"bogus\"\n").unwrap();
    let err = save_to(&file, &[Edit::set(&["rotation"], "interval", "5m")], None).unwrap_err();
    assert!(err.contains("not saved"), "{err}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "fit = \"bogus\"\n");
}

#[test]
fn save_creates_the_file_and_folder_and_returns_the_new_config() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("vellum/config.toml");
    let config = save_to(&file, &[Edit::set(&["rotation"], "interval", "5m")], None).unwrap();
    assert_eq!(config.rotation.schedule.interval_ms, 300_000);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "[rotation]\ninterval = \"5m\"\n"
    );
    let leftovers: Vec<_> = std::fs::read_dir(file.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, ["config.toml"], "no temp files left behind");
}

#[test]
fn save_writes_through_a_symlinked_config() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("dotfiles.toml");
    std::fs::write(&real, "# mine\n").unwrap();
    let link = dir.path().join("config.toml");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    save_to(&link, &[Edit::set(&[], "fit", "fit")], None).unwrap();
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(
        std::fs::read_to_string(&real).unwrap(),
        "# mine\nfit = \"fit\"\n"
    );
}

#[test]
fn paths_are_written_with_a_tilde_under_home() {
    let home = Path::new("/home/me");
    assert_eq!(
        tilde(Path::new("/home/me/Pictures"), Some(home)),
        "~/Pictures"
    );
    assert_eq!(tilde(Path::new("/srv/walls"), Some(home)), "/srv/walls");
}
