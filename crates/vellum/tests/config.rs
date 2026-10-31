//! The Config end to end: `vellum set` honors it, edits apply live through
//! the Daemon's inotify watch, and a malformed Config keeps the last good
//! values.

mod support;

use image::{Rgb, RgbImage};
use support::*;

const SMALL: Monitor = Monitor {
    width: 320,
    height: 240,
};

/// A wide, flat green wallpaper: Fit mode leaves bars above and below it.
fn wide_green(sway: &Sway, name: &str) -> String {
    let path = sway.dir().join(name);
    RgbImage::from_pixel(64, 16, Rgb(GREEN))
        .save(&path)
        .unwrap();
    path.to_string_lossy().into_owned()
}

fn status(sway: &Sway) -> String {
    sway.vellum_stdout(["status"])
}

#[test]
fn editing_the_transition_default_applies_live() {
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon();
    assert!(status(&sway).contains("transition: fade, 600 ms, ease-out"));

    // No Config directory existed when the Daemon started.
    sway.write_config("[transition]\ntype = \"wipe\"\ndirection = \"up\"\nduration = 250\n");
    sway.wait_until("the Daemon to pick up the new Transition", || {
        status(&sway).contains("transition: wipe up, 250 ms, ease-out")
    });

    // An editor saving in place, rather than by rename.
    std::fs::write(sway.config_path(), "[transition]\ntype = \"none\"\n").unwrap();
    sway.wait_until("the Daemon to pick up the second edit", || {
        status(&sway).contains("transition: none")
    });

    // Deleting the Config brings the defaults back.
    std::fs::remove_file(sway.config_path()).unwrap();
    sway.wait_until("the Daemon to fall back to defaults", || {
        status(&sway).contains("transition: fade, 600 ms, ease-out")
    });
    assert!(sway.daemon_alive());
}

#[test]
fn a_config_symlinked_from_dotfiles_reloads_when_its_target_changes() {
    let mut sway = Sway::start(&[SMALL]);
    let dotfiles = sway.dir().join("dotfiles");
    std::fs::create_dir_all(&dotfiles).unwrap();
    let target = dotfiles.join("vellum.toml");
    std::fs::write(&target, "[transition]\ntype = \"wipe\"\n").unwrap();
    std::fs::create_dir_all(sway.config_path().parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, sway.config_path()).unwrap();
    sway.start_daemon();
    sway.wait_until("the first sync", || {
        status(&sway).contains("transition: wipe")
    });

    std::fs::write(&target, "[transition]\ntype = \"slide\"\n").unwrap();
    sway.wait_until("an edit to the symlink target to apply", || {
        status(&sway).contains("transition: slide")
    });
}

#[test]
fn a_malformed_config_is_reported_precisely_and_the_last_good_one_stays() {
    let mut sway = Sway::start(&[SMALL]);
    sway.write_config(
        "fit = \"fit\"\n\
         letterbox = \"#0000ff\"\n\
         \n\
         [transition]\n\
         type = \"none\"\n",
    );
    sway.start_daemon();
    sway.wait_until("the first sync", || {
        status(&sway).contains("transition: none")
    });

    sway.write_config(
        "fit = \"fill\"\n\
         letterbox = \"#ff0000\"\n\
         \n\
         [transition]\n\
         type = \"wipe\"\n\
         duration = \"slow\"\n",
    );
    let out = sway.vellum(["reload"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "reload accepted a broken Config");
    assert!(stderr.contains("config.toml"), "{stderr}");
    assert!(stderr.contains("line 6"), "{stderr}");
    assert!(stderr.contains("`transition.duration`"), "{stderr}");
    assert!(stderr.contains("expected"), "{stderr}");
    // The inotify-triggered sync failed too, and said so in the Daemon log.
    sway.wait_until("the Daemon log to mention the error", || {
        sway.daemon_log().contains("transition.duration")
    });
    assert!(status(&sway).contains("transition: none"));

    // `set` warns but keeps using the last good Fit mode and letterbox.
    let path = wide_green(&sway, "wide.png");
    let out = sway.vellum(["set", &path]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.contains("line 6"), "{stderr}");
    assert!(stderr.contains("last good Config"), "{stderr}");
    let shot = sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(160, 5).0 == BLUE);
    assert_pixel(&shot, 160, 120, GREEN, 0);
    assert_pixel(&shot, 160, 5, BLUE, 0);

    // Fixing the file and reloading on demand applies it.
    sway.write_config("[transition]\ntype = \"slide\"\n");
    sway.vellum_ok(["reload"]);
    assert!(status(&sway).contains("transition: slide right, 600 ms, ease-out"));
    assert!(sway.daemon_alive());
}

#[test]
fn set_finds_a_wallpaper_by_name_in_the_library() {
    let mut sway = Sway::start(&[SMALL]);
    let library = sway.dir().join("walls");
    std::fs::create_dir_all(library.join("nested/deeper")).unwrap();
    RgbImage::from_pixel(32, 24, Rgb(RED))
        .save(library.join("nested/deeper/red.png"))
        .unwrap();
    sway.write_config(&format!(
        "library = [\"{}\"]\n[transition]\ntype = \"none\"\n",
        library.display()
    ));
    sway.start_daemon();

    sway.vellum_ok(["set", "red.png"]);
    let shot = sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(160, 120).0 == RED);
    assert_pixel(&shot, 160, 120, RED, 0);
    assert!(status(&sway).contains("walls/nested/deeper/red.png"));

    let out = sway.vellum(["set", "missing.png"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Library"), "{stderr}");
}

#[test]
fn without_a_library_folder_the_cli_prints_a_one_line_hint() {
    // No Config, and the sandbox has no ~/Pictures/Wallpapers.
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon();
    for args in [&["next"][..], &["set", "dunes.jpg"]] {
        let out = sway.vellum(args);
        assert!(!out.status.success());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(stderr.lines().count(), 1, "{stderr}");
        assert!(
            stderr.contains("~/Pictures/Wallpapers does not exist; run `vellum` to choose one"),
            "{stderr}"
        );
    }
}

#[test]
fn changing_the_letterbox_color_repaints_fit_mode_wallpapers_live() {
    let mut sway = Sway::start(&[SMALL]);
    sway.write_config("[transition]\ntype = \"none\"\n");
    sway.start_daemon();
    let path = wide_green(&sway, "wide.png");
    sway.vellum_ok(["set", &path, "--fit", "fit"]);
    sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(160, 120).0 == GREEN);
    assert_pixel(&sway.screenshot("HEADLESS-1"), 160, 5, [0, 0, 0], 0);

    sway.write_config("letterbox = \"#ffffff\"\n[transition]\ntype = \"none\"\n");

    let shot = sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(160, 5).0 == WHITE);
    assert_pixel(&shot, 160, 5, WHITE, 0);
    assert_pixel(&shot, 160, 120, GREEN, 0);
}

#[test]
fn set_uses_the_configured_fit_mode_and_letterbox_color() {
    let mut sway = Sway::start(&[SMALL]);
    sway.write_config(
        "fit = \"fit\"\n\
         letterbox = \"#ff0000\"\n\
         \n\
         [transition]\n\
         type = \"none\"\n",
    );
    sway.start_daemon();
    let path = wide_green(&sway, "wide.png");

    sway.vellum_ok(["set", &path]);

    let shot = sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(160, 5).0 == RED);
    assert_pixel(&shot, 160, 120, GREEN, 0);
    assert_pixel(&shot, 160, 5, RED, 0);
    assert_pixel(&shot, 160, 235, RED, 0);
}
