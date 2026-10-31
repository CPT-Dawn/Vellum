//! The Rotation panel, the welcome screen, and the narrow, too-small and
//! Daemon-down states.

use std::path::PathBuf;

use vellum_ipc::{RotationOrder, RotationSource};

use super::super::super::harness::*;
use super::super::super::model::{Daemon, Mode, Rotation, ToastKind};
use super::super::super::msg::{Cmd, Msg, Purpose, Wake};
use super::super::SAVE_DELAY;
use crate::config::edit::Edit;

/// Started, with the Rotation panel open.
fn panel(fake: Fake) -> Harness {
    let mut h = Harness::started_with(120, 40, fake);
    h.press("s");
    assert_eq!(h.model.mode, Mode::Rotation);
    h
}

/// Moves the panel cursor to the line labelled `label`.
fn go_to(h: &mut Harness, label: &str) {
    for _ in 0..5 {
        let screen = h.screen();
        if screen.lines().any(|l| l.contains(&format!("› {label}"))) {
            return;
        }
        h.press("j");
    }
    panic!("no panel line {label}:\n{}", h.screen());
}

#[test]
fn the_panel_edits_the_interval_into_the_config_and_the_daemon() {
    let fake = Fake {
        config_text: "# my setup\n[rotation]\ninterval = \"30m\" # half an hour\n"
            .to_owned()
            .into(),
        ..Fake::default()
    };
    let mut h = panel(fake);
    go_to(&mut h, "Interval");
    h.press("l");
    assert_eq!(
        *h.fake.config_text.lock().unwrap(),
        "# my setup\n[rotation]\ninterval = \"1h\" # half an hour\n",
        "only the value changed; comments survive"
    );
    assert_eq!(
        h.fake
            .settings
            .lock()
            .unwrap()
            .rotation
            .schedule
            .interval_ms,
        3_600_000,
        "pushed to the Daemon at once"
    );
    assert!(h.screen().contains("‹ every 1h ›"), "{}", h.screen());
    assert_eq!(h.model.toast.as_ref().unwrap().text, "Saved to the Config");
    h.press("h");
    h.press("h");
    assert!(h.fake.config_text.lock().unwrap().contains("\"15m\""));
}

#[test]
fn holding_a_key_writes_the_config_once_edits_pause() {
    let mut h = panel(Fake::default());
    go_to(&mut h, "Interval");
    let mut wakes = Vec::new();
    for _ in 0..3 {
        match h.update(Msg::Key(key("l"))).as_slice() {
            [Cmd::After { delay, wake }] => {
                assert_eq!(*delay, SAVE_DELAY);
                wakes.push(*wake);
            }
            other => panic!("expected a save timer, got {other:?}"),
        }
    }
    // The first two timers are stale.
    for wake in &wakes[..2] {
        assert!(h.update(Msg::Woke(*wake)).is_empty());
    }
    match h.update(Msg::Woke(wakes[2])).as_slice() {
        [Cmd::SaveConfig { edits, .. }] => {
            assert_eq!(edits, &[Edit::set(&["rotation"], "interval", "3h")]);
        }
        other => panic!("expected one save, got {other:?}"),
    }
    assert!(matches!(wakes[2], Wake::Config { .. }));
}

#[test]
fn enter_on_the_first_line_starts_and_stops_rotation() {
    let mut h = panel(Fake::default());
    assert!(h.screen().contains("○ stopped"));
    h.press("enter");
    assert!(*h.fake.rotation_running.lock().unwrap());
    assert!(h.screen().contains("● running"));
    assert_eq!(
        h.model.toast.as_ref().unwrap().text,
        "Rotation started: every 30m"
    );
    h.press("esc");
    assert_eq!(h.model.mode, Mode::Normal);
    assert!(h.screen().lines().next().unwrap().contains("⟳ every 30m"));
    h.press("s");
    h.press("space");
    assert!(!*h.fake.rotation_running.lock().unwrap());
    assert_eq!(h.model.rotation, Rotation::Off);
}

#[test]
fn an_override_is_written_under_its_monitor_and_shown_in_the_header() {
    let mut h = panel(Fake::default());
    h.press("enter"); // start Rotation
    go_to(&mut h, "Applies to");
    h.press("l");
    assert!(h.screen().contains("DP-1 (no Override yet)"));
    go_to(&mut h, "Source");
    h.press("l");
    assert_eq!(
        *h.fake.config_text.lock().unwrap(),
        "[rotation.monitor.DP-1]\nsource = \"favorites\"\n"
    );
    let header = h.screen().lines().next().unwrap().to_owned();
    assert!(
        header.contains("⟳ every 30m · Favorites · Override"),
        "{header}"
    );
    // The global Rotation is untouched.
    let config = h.fake.config();
    assert_eq!(config.rotation.schedule.source, RotationSource::Library);

    h.press("x");
    assert!(!h.fake.config_text.lock().unwrap().contains("DP-1"));
    let header = h.screen().lines().next().unwrap().to_owned();
    assert!(!header.contains("Override"), "{header}");
}

#[test]
fn editing_the_global_order_keeps_an_overrides_own_value() {
    let fake = Fake {
        config_text: "[rotation.monitor.DP-1]\norder = \"shuffle\"\ninterval = \"5m\"\n"
            .to_owned()
            .into(),
        ..Fake::default()
    };
    let mut h = Harness::with_config(120, 40, fake, config(""));
    h.model.schedules = h.fake.config().rotation;
    let cmds = h.start();
    h.run(cmds);
    h.press("s");
    go_to(&mut h, "Order");
    h.press("l");
    let config = h.fake.config();
    assert_eq!(config.rotation.schedule.order, RotationOrder::Sequential);
    assert_eq!(
        config.rotation.schedule_for("DP-1").order,
        RotationOrder::Shuffle
    );
    assert_eq!(
        h.model.schedules, config.rotation,
        "the panel shows the file"
    );
}

#[test]
fn a_config_that_cannot_be_written_says_why() {
    let fake = Fake {
        config_error: Some("cannot save config.toml: Permission denied".into()),
        ..Fake::default()
    };
    let mut h = panel(fake);
    go_to(&mut h, "Source");
    h.press("l");
    let toast = h.model.toast.clone().unwrap();
    assert_eq!(toast.kind, ToastKind::Error);
    assert!(toast.text.contains("Permission denied"));
}

#[test]
fn a_first_run_offers_pictures_and_saves_it() {
    let mut h = Harness::first_run(120, 40);
    assert_eq!(h.model.mode, Mode::Welcome);
    assert!(h.screen().contains("Use ~/Pictures"));
    h.fake.library = vec![PathBuf::from("/home/me/Pictures/sea.png")];
    let cmds = h.update(Msg::Key(key("enter")));
    assert_eq!(cmds, [Cmd::SaveLibrary(PathBuf::from("/home/me/Pictures"))]);
    h.run(cmds);
    assert_eq!(
        *h.fake.config_text.lock().unwrap(),
        "library = [\"~/Pictures\"]\n"
    );
    assert_eq!(h.model.mode, Mode::Normal);
    assert_eq!(h.model.folders, [PathBuf::from("/home/me/Pictures")]);
    assert!(h.screen().contains("sea.png"));
}

#[test]
fn a_typed_folder_completes_with_tab_and_is_saved() {
    let mut h = Harness::first_run(120, 40);
    h.typed("~/Pi");
    assert_eq!(h.model.welcome.choice, 1, "typing chooses the typed folder");
    h.press("tab");
    assert_eq!(h.model.welcome.input, "~/Pictures");
    assert_eq!(
        h.model.welcome.candidates,
        ["~/Pictures/", "~/Pictures-old/"]
    );
    h.typed("-");
    h.press("tab");
    assert_eq!(h.model.welcome.input, "~/Pictures-old/");
    assert!(h.model.welcome.candidates.is_empty());
    h.press("enter");
    assert_eq!(
        *h.fake.config_text.lock().unwrap(),
        "library = [\"~/Pictures-old\"]\n"
    );
}

#[test]
fn a_completion_for_older_input_is_dropped() {
    let mut h = Harness::first_run(120, 40);
    h.typed("~/W");
    let cmds = h.update(Msg::Key(key("tab")));
    h.typed("x");
    for cmd in cmds {
        if let Some(msg) = h.perform(cmd) {
            h.update(msg);
        }
    }
    assert_eq!(h.model.welcome.input, "~/Wx");
}

#[test]
fn a_folder_that_does_not_exist_is_refused_on_the_welcome_screen() {
    let mut h = Harness::first_run(120, 40);
    h.fake.missing = vec![PathBuf::from("/home/me/Nope")];
    h.typed("~/Nope");
    h.press("enter");
    assert_eq!(h.model.mode, Mode::Welcome);
    assert!(
        h.screen().contains("✗ ~/Nope is not a folder"),
        "{}",
        h.screen()
    );
    h.press("backspace");
    assert!(h.model.welcome.error.is_none());
    h.typed("relative");
    let mut h2 = Harness::first_run(120, 40);
    h2.typed("walls");
    h2.press("enter");
    assert!(h2.screen().contains("Type a full path"));
}

#[test]
fn esc_skips_the_welcome_and_o_brings_it_back() {
    let mut h = Harness::first_run(120, 40);
    h.press("esc");
    assert_eq!(h.model.mode, Mode::Normal);
    let screen = h.screen();
    assert!(screen.contains("No wallpapers yet"), "{screen}");
    assert!(screen.contains("o choose another folder"), "{screen}");
    h.press("o");
    assert_eq!(h.model.mode, Mode::Welcome);
}

#[test]
fn a_narrow_terminal_shows_the_list_and_p_swaps_in_the_preview() {
    let mut h = Harness::started(60, 24);
    let screen = h.screen();
    assert!(screen.contains("aurora.png"), "{screen}");
    assert!(!screen.contains("Preview"), "{screen}");
    let cmds = h.update(Msg::Key(key("p")));
    let job = match cmds.first() {
        Some(Cmd::RenderPreview(job)) => job.clone(),
        other => panic!("expected a preview job, got {other:?}"),
    };
    assert_eq!(job.purpose, Purpose::Preview);
    assert_eq!(job.cells.width, 58, "the preview takes the full width");
    h.run(cmds);
    let screen = h.screen();
    assert!(screen.contains("Preview"), "{screen}");
    assert!(!screen.contains("beach/dunes.jpg"), "{screen}");
    // Moving still works with the list hidden.
    h.press("j");
    assert!(h.screen().contains("dunes.jpg · fill"));
    h.press("p");
    assert!(h.screen().contains("› beach/dunes.jpg"));
}

#[test]
fn p_does_nothing_on_a_wide_terminal() {
    let mut h = Harness::started(120, 40);
    assert!(h.update(Msg::Key(key("p"))).is_empty());
    assert!(!h.model.dirty);
}

#[test]
fn a_tiny_terminal_says_so_and_still_quits() {
    let mut h = Harness::started(30, 8);
    let screen = h.screen();
    assert!(screen.contains("Terminal too small"), "{screen}");
    assert!(screen.contains("30×8, needs 40×10"), "{screen}");
    h.click(1, 1);
    h.press("q");
    assert!(h.model.quit);
}

#[test]
fn growing_out_of_too_small_renders_the_preview() {
    let mut h = Harness::started(30, 8);
    h.dispatch(Msg::Resize {
        width: 120,
        height: 40,
    });
    assert!(h.screen().contains("aurora.png · fill"));
}

#[test]
fn shift_r_reconnects_to_a_daemon_that_was_down() {
    let fake = Fake {
        daemon: Err("vellumd did not start".into()),
        ..Fake::default()
    };
    let mut h = Harness::started_with(120, 40, fake);
    assert!(h.screen().lines().last().unwrap().contains("R reconnect"));
    h.fake.daemon = Ok(daemon_info(vec![monitor("DP-1", 2560, 1440, 120)]));
    let cmds = h.update(Msg::Key(key("R")));
    assert_eq!(cmds, [Cmd::QueryDaemon]);
    assert!(matches!(h.model.daemon, Daemon::Connecting));
    h.run(cmds);
    assert!(h.screen().contains("● DP-1"));
}
