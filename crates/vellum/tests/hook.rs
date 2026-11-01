//! The `on_change` hook end to end: `vellum` runs it after each new
//! wallpaper with `{path}` and `{monitor}` substituted, never through a
//! shell, and a failing hook never touches the wallpaper.

mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use image::{Rgb, RgbImage};
use support::*;

const SMALL: Monitor = Monitor {
    width: 320,
    height: 240,
};

/// A file name that would run `touch pwned` if it ever reached a shell.
const HOSTILE: &str = "$(touch pwned); `touch pwned` 'it''s' \"x\" {monitor}.png";

fn solid(sway: &Sway, name: &str, color: [u8; 3]) -> String {
    let path = sway.dir().join(name);
    RgbImage::from_pixel(32, 24, Rgb(color))
        .save(&path)
        .unwrap();
    path.to_string_lossy().into_owned()
}

/// Writes a hook script that appends `$1|$2|$VELLUM_PATH|$VELLUM_MONITOR`
/// to the marker file, and a Config that runs it.
fn install_hook(sway: &Sway, extra: &str) {
    let script = sway.dir().join("hook.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf '%s|%s|%s|%s\\n' \"$1\" \"$2\" \"$VELLUM_PATH\" \"$VELLUM_MONITOR\" >> \"$HOME/marker\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    sway.write_config(&format!(
        "on_change = \"{} {{path}} {{monitor}}\"\n{extra}[transition]\ntype = \"none\"\n",
        script.display()
    ));
}

fn marker(sway: &Sway) -> PathBuf {
    sway.dir().join("marker")
}

fn marker_lines(sway: &Sway) -> Vec<String> {
    std::fs::read_to_string(marker(sway))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn line(path: &str, monitor: &str) -> String {
    format!("{path}|{monitor}|{path}|{monitor}")
}

#[test]
fn set_runs_the_hook_with_the_path_and_monitor_substituted() {
    let mut sway = Sway::start(&[SMALL, SMALL]);
    install_hook(&sway, "");
    sway.start_daemon();
    let path = solid(&sway, HOSTILE, RED);

    sway.vellum_ok(["set", &path]);

    // `vellum set` waits for its hooks, so the marker is complete now.
    assert_eq!(
        marker_lines(&sway),
        [line(&path, "HEADLESS-1"), line(&path, "HEADLESS-2")]
    );
    assert!(
        !sway.dir().join("pwned").exists(),
        "a file name reached a shell"
    );
    sway.screenshot_until("HEADLESS-2", |s| s.get_pixel(160, 120).0 == RED);
}

#[test]
fn render_with_a_file_presents_it_on_one_monitor_like_set() {
    let mut sway = Sway::start(&[SMALL, SMALL]);
    install_hook(&sway, "");
    // Never starts a Daemon: the Daemon is what runs `vellum render`.
    let path = solid(&sway, "red.png", RED);
    let out = sway.vellum(["render", &path, "--monitor", "HEADLESS-2"]);
    assert!(!out.status.success());
    assert!(!sway.daemon_answers());

    sway.start_daemon();
    sway.vellum_ok(["render", &path, "--monitor", "HEADLESS-2"]);

    sway.screenshot_until("HEADLESS-2", |s| s.get_pixel(160, 120).0 == RED);
    let status = sway.vellum_stdout(["status"]);
    let shown: Vec<&str> = status.lines().filter(|l| l.contains("red.png")).collect();
    assert_eq!(shown.len(), 1, "{status}");
    assert!(shown[0].contains("HEADLESS-2"), "{status}");
    // A new wallpaper, so the on_change hook ran, as for `set`.
    assert_eq!(marker_lines(&sway), [line(&path, "HEADLESS-2")]);
}

#[test]
fn set_on_one_monitor_runs_the_hook_for_that_monitor_only() {
    let mut sway = Sway::start(&[SMALL, SMALL]);
    install_hook(&sway, "");
    sway.start_daemon();
    let path = solid(&sway, "green.png", GREEN);

    sway.vellum_ok(["set", &path, "-m", "HEADLESS-2"]);

    assert_eq!(marker_lines(&sway), [line(&path, "HEADLESS-2")]);
}

#[test]
fn a_failing_hook_is_reported_and_the_wallpaper_still_applies() {
    let mut sway = Sway::start(&[SMALL]);
    sway.write_config("on_change = \"sh -c 'exit 3'\"\n[transition]\ntype = \"none\"\n");
    sway.start_daemon();
    let red = solid(&sway, "red.png", RED);

    let out = sway.vellum(["set", &red]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(
        stderr.contains("on_change hook `sh` failed on HEADLESS-1"),
        "{stderr}"
    );
    assert!(stderr.contains("exit status: 3"), "{stderr}");
    sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(160, 120).0 == RED);

    // A hook that cannot even start is reported the same way.
    sway.write_config("on_change = \"/nonexistent/hook {path}\"\n[transition]\ntype = \"none\"\n");
    let green = solid(&sway, "green.png", GREEN);
    let out = sway.vellum(["set", &green]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(
        stderr.contains("`/nonexistent/hook` could not run"),
        "{stderr}"
    );
    sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(160, 120).0 == GREEN);
    assert!(sway.daemon_alive());
}

#[test]
fn restoring_a_wallpaper_does_not_run_the_hook() {
    let mut sway = Sway::start(&[SMALL]);
    install_hook(&sway, "");
    sway.start_daemon();
    let path = solid(&sway, "red.png", RED);
    sway.vellum_ok(["set", &path]);
    assert_eq!(marker_lines(&sway).len(), 1);

    // A restore at Daemon startup brings back what was already applied.
    sway.vellum_ok(["kill"]);
    sway.wait_daemon_exit();
    sway.wait_until("the wallpaper to go away", || {
        sway.screenshot("HEADLESS-1").get_pixel(160, 120).0 != RED
    });
    sway.start_daemon();
    sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(160, 120).0 == RED);
    std::thread::sleep(Duration::from_millis(300)); // a hook would have run by now

    // And a restore run by hand, which waits for any hook it starts.
    sway.vellum_ok(["render", "--restore", "-m", "HEADLESS-1"]);
    assert_eq!(marker_lines(&sway), [line(&path, "HEADLESS-1")]);
}

#[test]
fn a_letterbox_repaint_does_not_run_the_hook() {
    let mut sway = Sway::start(&[SMALL]);
    install_hook(&sway, "fit = \"fit\"\nletterbox = \"#0000ff\"\n");
    sway.start_daemon();
    let wide = sway.dir().join("wide.png");
    RgbImage::from_pixel(64, 16, Rgb(GREEN))
        .save(&wide)
        .unwrap();
    let wide = wide.to_string_lossy().into_owned();
    sway.vellum_ok(["set", &wide]);
    sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(160, 5).0 == BLUE);

    install_hook(&sway, "fit = \"fit\"\nletterbox = \"#ff0000\"\n");
    sway.vellum_ok(["reload"]);
    let shot = sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(160, 5).0 == RED);
    assert_pixel(&shot, 160, 120, GREEN, 0);
    assert_eq!(marker_lines(&sway), [line(&wide, "HEADLESS-1")]);
}

/// A Library of two flat wallpapers, as Config text for `install_hook`.
fn library(sway: &Sway) -> String {
    let walls = sway.dir().join("walls");
    std::fs::create_dir_all(&walls).unwrap();
    for (name, color) in [("a.png", RED), ("b.png", GREEN)] {
        RgbImage::from_pixel(32, 24, Rgb(color))
            .save(walls.join(name))
            .unwrap();
    }
    format!("library = [\"{}\"]\n", walls.display())
}

#[test]
fn next_runs_the_hook_for_the_wallpaper_it_picked() {
    let mut sway = Sway::start(&[SMALL]);
    let extra = library(&sway);
    install_hook(&sway, &extra);
    sway.start_daemon();

    sway.vellum_ok(["next"]);

    let status = sway.vellum_stdout(["status"]);
    let shown = status
        .lines()
        .find_map(|l| l.strip_prefix("HEADLESS-1"))
        .and_then(|l| l.split_whitespace().nth(1))
        .expect("a wallpaper on HEADLESS-1")
        .to_owned();
    assert_eq!(marker_lines(&sway), [line(&shown, "HEADLESS-1")]);
}

#[test]
fn a_rotation_tick_runs_the_hook() {
    let mut sway = Sway::start(&[SMALL]);
    let extra = library(&sway) + "[rotation]\ninterval = \"300ms\"\n";
    install_hook(&sway, &extra);
    sway.start_daemon();
    sway.wait_until("the Config to reach the Daemon", || {
        sway.vellum_stdout(["rotation", "status"]).contains("300ms")
    });

    sway.vellum_ok(["rotation", "start"]);

    sway.wait_until("a tick to run the hook", || {
        marker_lines(&sway)
            .iter()
            .any(|l| l.ends_with("|HEADLESS-1") && l.contains("/walls/"))
    });
}
