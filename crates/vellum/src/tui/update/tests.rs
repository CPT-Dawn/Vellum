//! Msgs in, Model and Cmds out. No terminal, no Daemon.

use std::path::PathBuf;

use ratatui::layout::Size;

use super::super::harness::*;
use super::super::model::{Daemon, PreviewState, Tile, Toast, ToastKind};
use super::super::msg::{ApplyJob, Cmd, Msg, PreviewJob, Purpose, Screen, Wake};
use super::super::update;
use super::DEBOUNCE;
use crate::render::FitMode;

mod commands;

/// The one preview-pane job among `cmds` (prefetches may follow it).
fn preview_job(cmds: &[Cmd]) -> &PreviewJob {
    match cmds {
        [Cmd::RenderPreview(job), rest @ ..]
            if job.purpose == Purpose::Preview
                && rest.iter().all(
                    |c| matches!(c, Cmd::RenderPreview(j) if j.purpose == Purpose::Prefetch),
                ) =>
        {
            job
        }
        other => panic!("expected one preview job then prefetches, got {other:?}"),
    }
}

/// The debounce timer among `cmds`.
fn wake(cmds: &[Cmd]) -> Wake {
    match cmds {
        [Cmd::After { delay, wake }] => {
            assert_eq!(*delay, DEBOUNCE);
            *wake
        }
        other => panic!("expected one debounce timer, got {other:?}"),
    }
}

/// The paths and purposes of the render jobs among `cmds`.
fn jobs(cmds: &[Cmd]) -> Vec<(PathBuf, Purpose)> {
    cmds.iter()
        .map(|c| match c {
            Cmd::RenderPreview(job) => (job.path.clone(), job.purpose),
            other => panic!("expected only render jobs, got {other:?}"),
        })
        .collect()
}

/// A Library of `n` wallpapers named 000.png, 001.png, ...
fn big_library(n: usize) -> Fake {
    Fake {
        library: (0..n).map(|i| wallpaper(&format!("{i:03}.png"))).collect(),
        ..Fake::default()
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
fn moving_selects_and_asks_for_a_newer_preview_once_it_settles() {
    let mut h = Harness::started(80, 24);
    let before = h.model.preview.generation;
    let cmds = h.update(Msg::Key(key("j")));
    assert_eq!(h.model.selected, 1);
    // The old preview is gone at once; the new one waits for the debounce.
    assert!(matches!(
        &h.model.preview.state,
        PreviewState::Loading { path, .. } if *path == wallpaper("beach/dunes.jpg")
    ));
    let Wake::Preview { generation } = wake(&cmds) else {
        panic!("expected a preview wake");
    };
    assert!(generation > before);
    let cmds = h.update(Msg::Woke(Wake::Preview { generation }));
    assert!(!h.model.dirty, "waking alone draws nothing new");
    let job = preview_job(&cmds);
    assert_eq!(job.path, wallpaper("beach/dunes.jpg"));
    assert_eq!(job.generation, generation);

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
    let old = h.model.preview.generation;
    h.update(Msg::Key(key("j")));
    // The aurora preview finishes after the move: it must not show.
    let late = h.fake.preview_done(old, wallpaper("aurora.png"));
    assert!(h.update(late).is_empty());
    assert!(!h.model.dirty);
    assert!(matches!(
        &h.model.preview.state,
        PreviewState::Loading { path, .. } if *path == wallpaper("beach/dunes.jpg")
    ));
}

#[test]
fn workers_drop_jobs_whose_generation_moved_on() {
    let mut h = Harness::started(80, 24);
    let cmds = h.update(Msg::Key(key("j")));
    let cmds = h.update(Msg::Woke(wake(&cmds)));
    let current = preview_job(&cmds).clone();
    let stale = PreviewJob {
        generation: current.generation - 1,
        ..current.clone()
    };
    h.fake.rendered.lock().unwrap().clear();
    assert!(h.perform(Cmd::RenderPreview(stale)).is_none());
    assert!(
        h.fake.rendered.lock().unwrap().is_empty(),
        "a stale job is dropped before any decoding"
    );
    // Moving again makes the current job stale too, even mid-queue.
    h.update(Msg::Key(key("j")));
    assert!(h.perform(Cmd::RenderPreview(current)).is_none());
}

#[test]
fn fast_moves_render_only_where_the_selection_settles() {
    let mut h = Harness::started_with(80, 24, big_library(50));
    h.fake.rendered.lock().unwrap().clear();
    let wakes: Vec<Wake> = (0..5)
        .map(|_| wake(&h.update(Msg::Key(key("j")))))
        .collect();
    // Every timer fires, but only the newest one renders anything.
    for w in &wakes[..4] {
        assert!(h.update(Msg::Woke(*w)).is_empty());
    }
    let cmds = h.update(Msg::Woke(wakes[4]));
    assert_eq!(preview_job(&cmds).path, wallpaper("005.png"));
    h.run(cmds);
    let rendered = h.fake.rendered.lock().unwrap();
    assert_eq!(rendered[0].path, wallpaper("005.png"));
    assert!(
        rendered
            .iter()
            .all(|j| j.generation == rendered[0].generation)
    );
    assert!(matches!(h.model.preview.state, PreviewState::Ready { .. }));
}

#[test]
fn the_two_neighbours_on_each_side_are_prefetched_after_the_preview() {
    let mut h = Harness::started_with(80, 24, big_library(50));
    h.update(Msg::Key(key("j")));
    let cmds = h.update(Msg::Key(key("j")));
    let cmds = h.update(Msg::Woke(wake(&cmds)));
    assert_eq!(
        jobs(&cmds),
        [
            (wallpaper("002.png"), Purpose::Preview),
            (wallpaper("003.png"), Purpose::Prefetch),
            (wallpaper("001.png"), Purpose::Prefetch),
            (wallpaper("004.png"), Purpose::Prefetch),
            (wallpaper("000.png"), Purpose::Prefetch),
        ]
    );
    // A prefetch only warms the caches; it sends nothing back.
    let Cmd::RenderPreview(prefetch) = cmds[1].clone() else {
        unreachable!()
    };
    assert!(h.perform(Cmd::RenderPreview(prefetch)).is_none());

    // At the top of the Library there is nothing above to prefetch.
    let mut h = Harness::with(80, 24, big_library(50));
    let cmds = h.update(Msg::LibraryScanned(h.fake.library.clone()));
    assert_eq!(
        jobs(&cmds),
        [
            (wallpaper("000.png"), Purpose::Preview),
            (wallpaper("001.png"), Purpose::Prefetch),
            (wallpaper("002.png"), Purpose::Prefetch),
        ]
    );
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
fn a_resize_asks_for_a_preview_that_fits_the_new_pane_once_it_settles() {
    let mut h = Harness::started(80, 24);
    let first = wake(&h.update(Msg::Resize {
        width: 100,
        height: 30,
    }));
    let last = wake(&h.update(Msg::Resize {
        width: 120,
        height: 40,
    }));
    assert!(h.update(Msg::Woke(first)).is_empty());
    let cmds = h.update(Msg::Woke(last));
    assert_eq!(preview_job(&cmds).cells, Size::new(76, 34));
}

#[test]
fn g_toggles_the_grid_which_renders_only_the_tiles_on_screen() {
    let mut h = Harness::started_with(80, 24, big_library(50));
    assert!(!h.model.grid.on);
    let cmds = h.toggle_grid();
    assert!(h.model.grid.on);
    assert!(h.model.dirty);
    // 80x24 fits 3 columns by 2 rows of tiles.
    let names: Vec<_> = (0..6).map(|i| wallpaper(&format!("{i:03}.png"))).collect();
    assert_eq!(
        jobs(&cmds),
        names
            .iter()
            .map(|p| (p.clone(), Purpose::Thumb))
            .collect::<Vec<_>>()
    );
    let Cmd::RenderPreview(job) = &cmds[0] else {
        unreachable!()
    };
    assert_eq!(job.cells, super::super::layout::TILE_IMAGE);
    assert_eq!(job.generation, h.model.grid.generation);
    h.run(cmds);
    assert!(
        names
            .iter()
            .all(|p| matches!(h.model.grid.tiles.get(p), Some(Tile::Ready { .. })))
    );

    // Back to the list: the preview pane renders again.
    let cmds = h.toggle_grid();
    assert!(!h.model.grid.on);
    assert_eq!(preview_job(&cmds).path, wallpaper("000.png"));
}

#[test]
fn moving_in_the_grid_goes_by_tile_and_row_and_scrolls_by_rows() {
    let mut h = Harness::started_with(80, 24, big_library(50));
    let cmds = h.toggle_grid();
    h.run(cmds);
    // Within the page: no rendering at all.
    assert!(h.update(Msg::Key(key("l"))).is_empty());
    assert_eq!(h.model.selected, 1);
    assert!(h.update(Msg::Key(key("j"))).is_empty());
    assert_eq!(h.model.selected, 4);
    assert!(h.update(Msg::Key(key("h"))).is_empty());
    assert_eq!(h.model.selected, 3);
    // Down past the page scrolls one row, debounced; then only the new
    // row renders.
    let cmds = h.update(Msg::Key(key("j")));
    assert_eq!(h.model.selected, 6);
    assert_eq!(h.model.grid.offset_row, 1);
    let cmds = h.update(Msg::Woke(wake(&cmds)));
    assert_eq!(
        jobs(&cmds),
        [6, 7, 8].map(|i| (wallpaper(&format!("{i:03}.png")), Purpose::Thumb))
    );
    // Up from the top row does nothing; h/l do nothing in the list.
    h.run(cmds);
    h.press("k");
    h.press("k");
    assert!(h.update(Msg::Key(key("k"))).is_empty());
    let cmds = h.toggle_grid();
    h.run(cmds);
    assert!(h.update(Msg::Key(key("l"))).is_empty());
    assert!(!h.model.dirty);
}

#[test]
fn down_from_above_a_short_last_row_lands_on_the_last_tile() {
    let mut h = Harness::started_with(80, 24, big_library(5));
    let cmds = h.toggle_grid();
    h.run(cmds);
    h.press("l");
    h.press("l");
    h.press("j");
    assert_eq!(h.model.selected, 4);
    assert!(h.update(Msg::Key(key("j"))).is_empty());
}

#[test]
fn a_stale_thumbnail_is_dropped() {
    let mut h = Harness::started_with(80, 24, big_library(50));
    let cmds = h.toggle_grid();
    let Cmd::RenderPreview(old) = cmds[0].clone() else {
        unreachable!()
    };
    // Scrolling a page down moves the grid generation on.
    for _ in 0..2 {
        h.press("j");
    }
    assert!(h.model.grid.generation > old.generation);
    assert!(h.perform(Cmd::RenderPreview(old.clone())).is_none());
    let late = Msg::ThumbDone {
        generation: old.generation,
        path: old.path.clone(),
        result: Err("late".into()),
    };
    assert!(h.update(late).is_empty());
    assert!(!h.model.dirty);
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
