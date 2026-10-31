//! Msgs in, Model and Cmds out. No terminal, no Daemon.

use std::path::PathBuf;

use ratatui::layout::Size;

use super::super::harness::*;
use super::super::model::{Daemon, PreviewState, Toast, ToastKind};
use super::super::msg::{ApplyJob, Cmd, Msg, PreviewJob, Screen};
use super::super::update;
use crate::render::FitMode;

mod commands;

fn preview_job(cmds: &[Cmd]) -> &PreviewJob {
    match cmds {
        [Cmd::RenderPreview(job)] => job,
        other => panic!("expected one RenderPreview, got {other:?}"),
    }
}

#[test]
fn starting_scans_the_library_and_asks_the_daemon_at_once() {
    let mut h = Harness::new(80, 24);
    assert_eq!(
        h.start(),
        [
            Cmd::ScanLibrary(vec![wallpaper("")]),
            Cmd::QueryDaemon,
            Cmd::LoadFavorites
        ]
    );
    assert!(h.model.scanning);
}

#[test]
fn the_list_appears_before_any_preview_finishes() {
    let mut h = Harness::new(120, 40);
    let library = h.fake.library.clone();
    let cmds = h.update(Msg::LibraryScanned(library.clone()));
    assert!(!h.model.scanning);
    assert_eq!(h.model.library, library);
    assert_eq!(h.model.selection(), Some(&library[0]));
    assert!(matches!(
        h.model.preview.state,
        PreviewState::Loading { .. }
    ));
    let job = preview_job(&cmds);
    assert_eq!(job.path, library[0]);
    // Until the Daemon answers, previews assume a 1080p Monitor.
    assert_eq!(job.monitor, Screen::FALLBACK);
    assert_eq!(job.fit, FitMode::Fill);
    // The preview pane at 120x40: 78 columns minus borders, 38 rows minus
    // borders and the metadata strip.
    assert_eq!(job.cells, Size::new(76, 34));
}

#[test]
fn the_preview_is_drawn_for_the_targeted_monitor() {
    let mut h = Harness::new(80, 24);
    h.update(Msg::LibraryScanned(h.fake.library.clone()));
    let info = h.fake.daemon.clone().unwrap();
    let cmds = h.update(Msg::Daemon(Ok(info)));
    let job = preview_job(&cmds);
    assert_eq!(
        job.monitor,
        Screen {
            width: 2560,
            height: 1440,
            scale_120: 120
        }
    );
    assert_eq!(h.model.targeted().unwrap().name, "DP-1");
}

#[test]
fn moving_selects_and_asks_for_a_newer_preview() {
    let mut h = Harness::started(80, 24);
    let before = h.model.preview.generation;
    let cmds = h.update(Msg::Key(key("j")));
    assert_eq!(h.model.selected, 1);
    let job = preview_job(&cmds);
    assert_eq!(job.path, wallpaper("beach/dunes.jpg"));
    assert!(job.generation > before);

    h.update(Msg::Key(key("down")));
    h.update(Msg::Key(key("k")));
    h.update(Msg::Key(key("up")));
    assert_eq!(h.model.selected, 0);
}

#[test]
fn moving_past_either_end_does_nothing() {
    let mut h = Harness::started(80, 24);
    let cmds = h.update(Msg::Key(key("k")));
    assert_eq!(h.model.selected, 0);
    assert!(cmds.is_empty());
    assert!(!h.model.dirty);
    for _ in 0..10 {
        h.press("j");
    }
    assert_eq!(h.model.selected, 3);
}

#[test]
fn a_finished_preview_is_shown_with_its_metadata() {
    let h = Harness::started(80, 24);
    match &h.model.preview.state {
        PreviewState::Ready { path, meta, .. } => {
            assert_eq!(path, &wallpaper("aurora.png"));
            assert_eq!((meta.width, meta.height), (3840, 2160));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_stale_preview_is_dropped_without_a_redraw() {
    let mut h = Harness::started(80, 24);
    let cmds = h.update(Msg::Key(key("j")));
    let newer = preview_job(&cmds).clone();
    let stale = PreviewJob {
        generation: newer.generation - 1,
        path: wallpaper("aurora.png"),
        ..newer.clone()
    };
    let msg = super::super::services::perform(&h.fake, Cmd::RenderPreview(stale));
    assert!(h.update(msg).is_empty());
    assert!(!h.model.dirty);
    assert!(matches!(
        &h.model.preview.state,
        PreviewState::Loading { path, .. } if *path == newer.path
    ));
}

#[test]
fn a_corrupt_wallpaper_shows_an_error_not_a_crash() {
    let fake = Fake {
        broken: vec![wallpaper("aurora.png")],
        ..Fake::default()
    };
    let h = Harness::started_with(80, 24, fake);
    assert!(matches!(
        &h.model.preview.state,
        PreviewState::Failed { error, .. } if error.contains("invalid PNG")
    ));
}

#[test]
fn enter_applies_to_the_targeted_monitor_and_confirms() {
    let mut h = Harness::started(80, 24);
    h.press("j");
    let cmds = h.update(Msg::Key(key("enter")));
    assert_eq!(
        cmds,
        [Cmd::Apply(ApplyJob {
            path: wallpaper("beach/dunes.jpg"),
            monitor: Some("DP-1".into()),
            fit: FitMode::Fill,
        })]
    );
    h.run(cmds);
    assert_eq!(h.fake.applied.lock().unwrap().len(), 1);
    assert_eq!(
        h.model.toast,
        Some(Toast {
            kind: ToastKind::Success,
            text: "Applied to DP-1".into()
        })
    );
}

#[test]
fn a_failed_apply_shows_why() {
    let fake = Fake {
        apply_error: Some("DP-1: frame size mismatch".into()),
        ..Fake::default()
    };
    let mut h = Harness::started_with(80, 24, fake);
    h.press("enter");
    let toast = h.model.toast.clone().unwrap();
    assert_eq!(toast.kind, ToastKind::Error);
    assert!(toast.text.contains("frame size mismatch"), "{toast:?}");
}

#[test]
fn enter_without_a_daemon_explains_instead_of_applying() {
    let fake = Fake {
        daemon: Err("vellumd did not start".into()),
        ..Fake::default()
    };
    let mut h = Harness::started_with(80, 24, fake);
    assert!(matches!(h.model.daemon, Daemon::Down(_)));
    let cmds = h.update(Msg::Key(key("enter")));
    assert!(cmds.is_empty());
    assert_eq!(h.model.toast.as_ref().unwrap().kind, ToastKind::Error);
}

#[test]
fn a_toast_stays_through_keys_and_expires_on_its_own() {
    let mut h = Harness::started(80, 24);
    h.press("enter");
    assert!(h.model.toast.is_some());
    h.press("j");
    assert!(h.model.toast.is_some());
    // The runtime is asked to wake exactly when it expires.
    assert_eq!(update::wake_at(&h.model), Some(update::TOAST_OK));
    h.advance(2_900);
    assert!(h.model.toast.is_some());
    h.advance(100);
    assert!(h.model.toast.is_none());
    assert!(h.model.dirty);
    assert_eq!(update::wake_at(&h.model), None, "idle: no more wakeups");
}

#[test]
fn errors_stay_up_longer_than_confirmations() {
    let fake = Fake {
        apply_error: Some("DP-1: frame size mismatch".into()),
        ..Fake::default()
    };
    let mut h = Harness::started_with(80, 24, fake);
    h.press("enter");
    h.advance(update::TOAST_OK.as_millis() as u64);
    assert!(h.model.toast.is_some());
    h.advance(3_000);
    assert!(h.model.toast.is_none());
}

#[test]
fn quitting_with_q_esc_or_ctrl_c() {
    for k in ["q", "esc", "ctrl-c"] {
        let mut h = Harness::started(80, 24);
        assert!(h.update(Msg::Key(key(k))).is_empty());
        assert!(h.model.quit, "{k}");
    }
}

#[test]
fn an_unbound_key_changes_nothing_and_skips_the_redraw() {
    let mut h = Harness::started(80, 24);
    assert!(h.update(Msg::Key(key("z"))).is_empty());
    assert!(!h.model.dirty);
}

#[test]
fn a_resize_asks_for_a_preview_that_fits_the_new_pane() {
    let mut h = Harness::started(80, 24);
    let cmds = h.update(Msg::Resize {
        width: 120,
        height: 40,
    });
    assert_eq!(preview_job(&cmds).cells, Size::new(76, 34));
}

#[test]
fn an_empty_library_asks_for_no_preview() {
    let fake = Fake {
        library: Vec::<PathBuf>::new(),
        ..Fake::default()
    };
    let h = Harness::started_with(80, 24, fake);
    assert!(h.model.library.is_empty());
    assert!(matches!(h.model.preview.state, PreviewState::Empty));
}
