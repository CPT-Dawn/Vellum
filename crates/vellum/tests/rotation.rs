//! Rotation and Favorites end to end, with short intervals: the Daemon's
//! timer ticks, `vellum` picks, and both survive a Daemon restart.

mod support;

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use image::{Rgb, RgbImage};
use support::*;

const SMALL: Monitor = Monitor {
    width: 320,
    height: 240,
};

/// A Library folder holding one flat wallpaper per name, in order.
fn library(sway: &Sway, names: &[&str]) -> PathBuf {
    let walls = sway.dir().join("walls");
    std::fs::create_dir_all(&walls).unwrap();
    let colors = [RED, GREEN, BLUE, WHITE, [255, 255, 0], [0, 255, 255]];
    for (name, color) in names.iter().zip(colors) {
        RgbImage::from_pixel(32, 24, Rgb(color))
            .save(walls.join(name))
            .unwrap();
    }
    walls
}

/// Config with the Library at `walls`, no Transition, and `extra`.
fn configure(sway: &Sway, walls: &std::path::Path, extra: &str) {
    sway.write_config(&format!(
        "library = [\"{}\"]\n[transition]\ntype = \"none\"\n{extra}",
        walls.display()
    ));
}

/// The wallpaper `monitor` shows, from `vellum status`.
fn shown(sway: &Sway, monitor: &str) -> Option<String> {
    let status = sway.vellum_stdout(["status"]);
    let path = status
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{monitor} ")))?
        .split_whitespace()
        .nth(1)?;
    (path != "-").then(|| file_name(path))
}

fn file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap().to_owned()
}

fn rotation_status(sway: &Sway) -> String {
    sway.vellum_stdout(["rotation", "status"])
}

/// Waits until the Daemon holds the Config's Rotation (the startup sync).
fn wait_synced(sway: &Sway, interval: &str) {
    sway.wait_until("the Config to reach the Daemon", || {
        rotation_status(sway).contains(interval)
    });
}

/// The next `n` wallpapers `monitor` changes to, watched through status.
fn watch_changes(sway: &Sway, monitor: &str, n: usize) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut last = shown(sway, monitor);
    let deadline = Instant::now() + Duration::from_secs(20);
    while seen.len() < n {
        assert!(Instant::now() < deadline, "only saw {seen:?}");
        let now = shown(sway, monitor);
        if now != last {
            seen.extend(now.clone());
            last = now;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    seen
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

const ABC: [&str; 3] = ["a.png", "b.png", "c.png"];

#[test]
fn shuffle_shows_every_wallpaper_before_any_repeats() {
    let mut sway = Sway::start(&[SMALL]);
    let walls = library(&sway, &ABC);
    configure(&sway, &walls, "");
    sway.start_daemon();

    let drawn: Vec<String> = (0..9)
        .map(|_| {
            sway.vellum_ok(["next"]);
            shown(&sway, "HEADLESS-1").unwrap()
        })
        .collect();
    for round in drawn.chunks(3) {
        assert_eq!(sorted(round.to_vec()), ABC, "{drawn:?}");
    }
    for pair in drawn.windows(2) {
        assert_ne!(pair[0], pair[1], "a repeat across the reshuffle: {drawn:?}");
    }
}

#[test]
fn rotation_ticks_on_its_interval_until_stopped() {
    let mut sway = Sway::start(&[SMALL]);
    let walls = library(&sway, &ABC);
    configure(&sway, &walls, "[rotation]\ninterval = \"300ms\"\n");
    sway.start_daemon();
    wait_synced(&sway, "300ms");
    let status = rotation_status(&sway);
    assert!(status.starts_with("rotation: stopped\n"), "{status}");
    assert!(
        status.contains("HEADLESS-1  library  shuffle  300ms     -"),
        "{status}"
    );

    sway.vellum_ok(["rotation", "start"]);
    assert!(rotation_status(&sway).contains("rotation: running"));
    assert!(
        sway.vellum_stdout(["status"])
            .contains("rotation: running, every 300ms, shuffle from the Library")
    );

    // The first round of ticks is one shuffled deck.
    let ticks = watch_changes(&sway, "HEADLESS-1", 3);
    assert_eq!(sorted(ticks), ABC);
    let color = sway.screenshot("HEADLESS-1").get_pixel(160, 120).0;
    assert!([RED, GREEN, BLUE].contains(&color), "{color:?}");

    sway.vellum_ok(["rotation", "stop"]);
    let stopped_on = shown(&sway, "HEADLESS-1");
    std::thread::sleep(Duration::from_millis(900));
    assert_eq!(shown(&sway, "HEADLESS-1"), stopped_on);
    assert!(rotation_status(&sway).starts_with("rotation: stopped"));
}

#[test]
fn rotation_and_its_decks_survive_a_daemon_restart() {
    let mut sway = Sway::start(&[SMALL]);
    let walls = library(&sway, &ABC);
    configure(
        &sway,
        &walls,
        "[rotation]\ninterval = \"1h\"\norder = \"sequential\"\n",
    );
    sway.start_daemon();
    wait_synced(&sway, "1h");
    sway.vellum_ok(["rotation", "start"]);
    sway.vellum_ok(["next"]);
    sway.vellum_ok(["next"]);
    assert_eq!(shown(&sway, "HEADLESS-1").as_deref(), Some("b.png"));

    sway.vellum_ok(["kill"]);
    // Any command brings a Daemon back; Rotation is still on.
    assert!(rotation_status(&sway).contains("rotation: running"));
    // The sequential position carried over.
    sway.vellum_ok(["next"]);
    assert_eq!(shown(&sway, "HEADLESS-1").as_deref(), Some("c.png"));

    // And the restarted Daemon still ticks.
    configure(
        &sway,
        &walls,
        "[rotation]\ninterval = \"300ms\"\norder = \"sequential\"\n",
    );
    let ticks = watch_changes(&sway, "HEADLESS-1", 2);
    assert_eq!(ticks, ["a.png", "b.png"]);
}

#[test]
fn a_shuffled_deck_is_not_restarted_by_a_daemon_restart() {
    let mut sway = Sway::start(&[SMALL]);
    let walls = library(&sway, &ABC);
    configure(&sway, &walls, "");
    sway.start_daemon();
    sway.vellum_ok(["next"]);
    let first = shown(&sway, "HEADLESS-1").unwrap();
    sway.vellum_ok(["next"]);
    let second = shown(&sway, "HEADLESS-1").unwrap();
    sway.vellum_ok(["kill"]);
    sway.vellum_ok(["next"]);
    let third = shown(&sway, "HEADLESS-1").unwrap();
    assert_eq!(sorted(vec![first, second, third]), ABC);
}

#[test]
fn an_override_rotates_its_monitor_on_its_own_schedule_and_source() {
    let mut sway = Sway::start(&[SMALL, SMALL]);
    let walls = library(&sway, &["a.png", "b.png", "c.png", "d.png"]);
    configure(
        &sway,
        &walls,
        "[rotation]\ninterval = \"1h\"\n\
         [rotation.monitor.HEADLESS-2]\ninterval = \"300ms\"\nsource = \"favorites\"\n",
    );
    sway.start_daemon();
    sway.vellum_ok(["fav", "add", "a.png"]);
    sway.vellum_ok(["fav", "add", "b.png"]);
    wait_synced(&sway, "300ms");
    let status = rotation_status(&sway);
    assert!(
        status.contains("HEADLESS-2  favorites  shuffle  300ms"),
        "{status}"
    );

    sway.vellum_ok(["rotation", "start"]);
    let ticks = watch_changes(&sway, "HEADLESS-2", 4);
    assert!(
        ticks.iter().all(|t| t == "a.png" || t == "b.png"),
        "drew outside Favorites: {ticks:?}"
    );
    assert_eq!(
        shown(&sway, "HEADLESS-1"),
        None,
        "the global 1h Rotation ticked"
    );
}

#[test]
fn monitors_get_different_wallpapers_unless_synced() {
    let mut sway = Sway::start(&[SMALL, SMALL]);
    let walls = library(&sway, &ABC);
    configure(&sway, &walls, "");
    sway.start_daemon();
    for _ in 0..4 {
        sway.vellum_ok(["next"]);
        assert_ne!(shown(&sway, "HEADLESS-1"), shown(&sway, "HEADLESS-2"));
        sway.vellum_ok(["random"]);
        assert_ne!(shown(&sway, "HEADLESS-1"), shown(&sway, "HEADLESS-2"));
    }

    configure(
        &sway,
        &walls,
        "[rotation]\nsync = true\ninterval = \"300ms\"\n",
    );
    wait_synced(&sway, "300ms");
    for _ in 0..3 {
        sway.vellum_ok(["next"]);
        assert_eq!(shown(&sway, "HEADLESS-1"), shown(&sway, "HEADLESS-2"));
    }
    // Ticks keep the sync group together too.
    sway.vellum_ok(["rotation", "start"]);
    watch_changes(&sway, "HEADLESS-1", 2);
    sway.vellum_ok(["rotation", "stop"]);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(shown(&sway, "HEADLESS-1"), shown(&sway, "HEADLESS-2"));
}

#[test]
fn a_manual_apply_restarts_that_monitors_timer() {
    let mut sway = Sway::start(&[SMALL]);
    let walls = library(&sway, &ABC);
    let pick = sway.dir().join("pick.png");
    RgbImage::from_pixel(32, 24, Rgb(WHITE))
        .save(&pick)
        .unwrap();
    configure(&sway, &walls, "[rotation]\ninterval = \"2s\"\n");
    sway.start_daemon();
    wait_synced(&sway, "2s");

    sway.vellum_ok(["rotation", "start"]);
    let started = Instant::now();
    std::thread::sleep(Duration::from_millis(1200));
    sway.vellum_ok(["set", pick.to_str().unwrap()]);
    let status = rotation_status(&sway);
    assert!(
        status.contains("in 2s"),
        "the timer was not reset:\n{status}"
    );

    // Past the original deadline, the pick is still up.
    let original = Duration::from_millis(2500);
    std::thread::sleep(original.saturating_sub(started.elapsed()));
    assert_eq!(shown(&sway, "HEADLESS-1").as_deref(), Some("pick.png"));
    // A full interval after the pick, Rotation moves on.
    let next = watch_changes(&sway, "HEADLESS-1", 1);
    assert!(ABC.contains(&next[0].as_str()));
    assert!(started.elapsed() >= Duration::from_millis(3100));

    // `vellum next` counts as a manual apply too.
    sway.vellum_ok(["next"]);
    assert!(rotation_status(&sway).contains("in 2s"));
}

#[test]
fn undecodable_files_are_skipped_and_rotation_never_stalls() {
    let mut sway = Sway::start(&[SMALL]);
    let walls = library(&sway, &["b.png"]);
    std::fs::write(walls.join("a.png"), b"not a png at all").unwrap();
    configure(&sway, &walls, "[rotation]\norder = \"sequential\"\n");
    sway.start_daemon();

    let out = sway.vellum(["next"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("skipping") && stderr.contains("a.png"),
        "{stderr}"
    );
    assert_eq!(shown(&sway, "HEADLESS-1").as_deref(), Some("b.png"));

    // With nothing decodable left, the command fails clearly and the
    // Daemon carries on.
    std::fs::write(walls.join("b.png"), b"broken too").unwrap();
    let out = sway.vellum(["next"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("could be decoded"));
    assert!(sway.daemon_alive());
}

#[test]
fn favorites_persist_and_missing_files_are_skipped() {
    let mut sway = Sway::start(&[SMALL]);
    let walls = library(&sway, &ABC);
    configure(&sway, &walls, "[rotation]\nsource = \"favorites\"\n");
    sway.start_daemon();

    let out = sway.vellum(["next"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("vellum fav add"));

    sway.vellum_ok(["fav", "add", "a.png"]);
    sway.vellum_ok(["fav", "add", walls.join("c.png").to_str().unwrap()]);
    sway.vellum_ok(["set", "b.png"]);
    sway.vellum_ok(["fav", "add"]); // the wallpaper on screen
    sway.vellum_ok(["fav", "add", "a.png"]); // no duplicate
    let list = sway.vellum_stdout(["fav", "list"]);
    let names: Vec<String> = list.lines().map(file_name).collect();
    assert_eq!(names, ["a.png", "c.png", "b.png"]);
    // Favorites live in user data, not in State.
    assert!(sway.dir().join("data/vellum/favorites").is_file());

    std::fs::remove_file(walls.join("c.png")).unwrap();
    let out = sway.vellum(["fav", "list"]);
    let names: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(file_name)
        .collect();
    assert_eq!(names, ["a.png", "b.png"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("c.png"));

    // Rotation from Favorites skips the missing one.
    let mut seen = HashSet::new();
    for _ in 0..4 {
        sway.vellum_ok(["next"]);
        seen.insert(shown(&sway, "HEADLESS-1").unwrap());
    }
    assert_eq!(
        seen,
        HashSet::from(["a.png".to_owned(), "b.png".to_owned()])
    );

    sway.vellum_ok(["fav", "remove", walls.join("c.png").to_str().unwrap()]);
    sway.vellum_ok(["fav", "remove", "a.png"]);
    assert_eq!(sway.vellum_stdout(["fav", "list"]).lines().count(), 1);
}

#[test]
fn prev_goes_back_through_what_was_shown() {
    let mut sway = Sway::start(&[SMALL]);
    let walls = library(&sway, &ABC);
    configure(&sway, &walls, "[rotation]\norder = \"sequential\"\n");
    sway.start_daemon();
    for _ in 0..3 {
        sway.vellum_ok(["next"]);
    }
    sway.vellum_ok(["prev"]);
    assert_eq!(shown(&sway, "HEADLESS-1").as_deref(), Some("b.png"));
    sway.vellum_ok(["prev", "-m", "HEADLESS-1"]);
    assert_eq!(shown(&sway, "HEADLESS-1").as_deref(), Some("a.png"));
    let out = sway.vellum(["prev"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no earlier wallpaper"));
}

#[test]
fn the_daemon_makes_no_wakeups_between_ticks() {
    let mut sway = Sway::start(&[SMALL]);
    let walls = library(&sway, &ABC);
    configure(&sway, &walls, "[rotation]\ninterval = \"1h\"\n");
    let pid = sway.start_daemon();
    wait_synced(&sway, "1h");
    sway.vellum_ok(["rotation", "start"]);
    std::thread::sleep(Duration::from_millis(300));

    let switches = || {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
        status
            .lines()
            .filter(|l| l.contains("ctxt_switches"))
            .filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
            .sum::<u64>()
    };
    let before = switches();
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(switches(), before, "the Daemon woke up while idle");
}
