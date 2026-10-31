//! Every Action, search, Favorites, the palette, help, mouse and timers.

use std::time::Duration;

use vellum_ipc::{RotationSource, Schedule};

use super::super::super::harness::*;
use super::super::super::model::{Mode, PreviewState, Rotation, ToastKind};
use super::super::super::msg::{ApplyJob, Cmd, Msg};
use super::super::super::update;
use crate::render::FitMode;

fn many(n: usize) -> Fake {
    Fake {
        library: (0..n)
            .map(|i| wallpaper(&format!("wall-{i:02}.png")))
            .collect(),
        ..Fake::default()
    }
}

fn names(h: &Harness) -> Vec<String> {
    h.model
        .visible
        .iter()
        .map(|v| h.model.short_name(&h.model.library[v.index]))
        .collect()
}

fn selected_name(h: &Harness) -> String {
    h.model.short_name(h.model.selection().unwrap())
}

fn applies(cmds: &[Cmd]) -> Vec<&ApplyJob> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::Apply(job) => Some(job),
            _ => None,
        })
        .collect()
}

#[test]
fn gg_and_g_jump_to_either_end() {
    let mut h = Harness::started_with(80, 24, many(40));
    h.press("G");
    assert_eq!(h.model.selected, 39);
    // The first g waits for the second.
    let cmds = h.update(Msg::Key(key("g")));
    assert!(cmds.is_empty());
    assert_eq!(h.model.selected, 39);
    h.press("g");
    assert_eq!(h.model.selected, 0);
    h.press("end");
    assert_eq!(h.model.selected, 39);
    h.press("home");
    assert_eq!(h.model.selected, 0);
}

#[test]
fn a_broken_sequence_lets_the_next_key_count() {
    let mut h = Harness::started_with(80, 24, many(40));
    h.press("g");
    h.press("j");
    assert_eq!(h.model.selected, 1);
    assert!(h.model.pending.is_empty());
}

#[test]
fn a_key_bound_alone_and_as_a_sequence_start_fires_after_a_pause() {
    let mut h = Harness::started_with_config(80, 24, "[keymap]\nbottom = \"g\"\n");
    h.press("g");
    assert_eq!(h.model.selected, 0, "still waiting for g g");
    assert_eq!(
        update::wake_at(&h.model),
        Some(super::super::super::keymap::CHORD_TIMEOUT)
    );
    h.advance(400);
    assert_eq!(h.model.selected, 3);
    h.press("g");
    h.press("g");
    assert_eq!(h.model.selected, 0);
}

#[test]
fn ctrl_d_and_ctrl_u_move_half_a_page() {
    let mut h = Harness::started_with(80, 24, many(40));
    // 20 list rows at 80x24.
    h.press("ctrl-d");
    assert_eq!(h.model.selected, 10);
    h.press("pagedown");
    assert_eq!(h.model.selected, 20);
    h.press("ctrl-u");
    assert_eq!(h.model.selected, 10);
}

#[test]
fn a_applies_to_every_monitor() {
    let mut h = Harness::started(80, 24);
    let cmds = h.update(Msg::Key(key("a")));
    assert_eq!(
        applies(&cmds),
        [&ApplyJob {
            path: wallpaper("aurora.png"),
            monitor: None,
            fit: FitMode::Fill,
        }]
    );
    h.run(cmds);
    assert_eq!(
        h.model.toast.as_ref().unwrap().text,
        "Applied to every monitor"
    );
}

#[test]
fn r_picks_another_wallpaper_from_the_list_and_applies_it() {
    let mut h = Harness::started_with(80, 24, many(40));
    for _ in 0..20 {
        let before = h.model.selected;
        let cmds = h.update(Msg::Key(key("r")));
        assert_ne!(h.model.selected, before);
        let jobs = applies(&cmds);
        assert_eq!(jobs.len(), 1);
        assert_eq!(&jobs[0].path, h.model.selection().unwrap());
        assert_eq!(jobs[0].monitor.as_deref(), Some("DP-1"));
        h.run(cmds);
    }
}

#[test]
fn r_draws_only_from_what_the_search_shows() {
    let mut h = Harness::started(80, 24);
    h.press("/");
    h.typed("beach");
    h.press("enter");
    for _ in 0..10 {
        h.press("r");
        assert!(selected_name(&h).starts_with("beach/"));
    }
}

#[test]
fn m_targets_the_next_monitor_and_redraws_the_preview_for_it() {
    let mut h = Harness::started(80, 24);
    let cmds = h.update(Msg::Key(key("m")));
    assert_eq!(h.model.targeted().unwrap().name, "HDMI-A-1");
    match cmds.as_slice() {
        [Cmd::RenderPreview(job)] => assert_eq!(job.monitor.width, 1920),
        other => panic!("{other:?}"),
    }
    h.run(cmds);
    h.press("enter");
    assert_eq!(h.model.toast.as_ref().unwrap().text, "Applied to HDMI-A-1");
    h.press("m");
    assert_eq!(h.model.targeted().unwrap().name, "DP-1");
}

#[test]
fn slash_filters_on_every_keystroke_with_the_matched_characters() {
    let mut h = Harness::started(80, 24);
    h.press("/");
    assert_eq!(h.model.mode, Mode::Search);
    h.typed("d");
    assert_eq!(names(&h), ["beach/dunes.jpg", "beach/tide.webp"]);
    h.typed("u");
    assert_eq!(names(&h), ["beach/dunes.jpg"]);
    assert_eq!(h.model.visible[0].matched, [6, 7]);
    // The best match is selected and previewed.
    assert_eq!(selected_name(&h), "beach/dunes.jpg");
    assert_eq!(
        h.model.preview.state.path(),
        Some(wallpaper("beach/dunes.jpg").as_path())
    );
    h.press("backspace");
    assert_eq!(names(&h).len(), 2);
}

#[test]
fn enter_keeps_the_search_and_esc_clears_it() {
    let mut h = Harness::started(80, 24);
    h.press("/");
    h.typed("tide");
    h.press("enter");
    assert_eq!(h.model.mode, Mode::Normal);
    assert_eq!(names(&h), ["beach/tide.webp"]);
    // Normal keys work again on the filtered list.
    let cmds = h.update(Msg::Key(key("enter")));
    assert_eq!(applies(&cmds)[0].path, wallpaper("beach/tide.webp"));
    // Esc in the list clears the filter and keeps the selection.
    h.press("esc");
    assert!(!h.model.quit);
    assert_eq!(names(&h).len(), 4);
    assert_eq!(selected_name(&h), "beach/tide.webp");
    // With nothing to clear, Esc quits.
    h.press("esc");
    assert!(h.model.quit);
}

#[test]
fn esc_while_typing_clears_the_search_at_once() {
    let mut h = Harness::started(80, 24);
    h.press("/");
    h.typed("neon");
    h.press("esc");
    assert_eq!(h.model.mode, Mode::Normal);
    assert!(h.model.query.is_empty());
    assert_eq!(names(&h).len(), 4);
    assert_eq!(selected_name(&h), "city/neon-rain.jpg");
}

#[test]
fn letters_type_into_the_search_instead_of_running_actions() {
    let mut h = Harness::started(80, 24);
    h.press("/");
    let cmds = h.update(Msg::Key(key("q")));
    assert!(!h.model.quit);
    assert!(applies(&cmds).is_empty());
    assert_eq!(h.model.query, "q");
    h.press("ctrl-u");
    assert_eq!(h.model.query, "");
    // Arrows and ctrl-n/p still move through the matches.
    h.press("down");
    h.press("ctrl-n");
    assert_eq!(h.model.selected, 2);
    h.press("ctrl-p");
    assert_eq!(h.model.selected, 1);
}

#[test]
fn f_toggles_a_favorite_and_saves_it() {
    let mut h = Harness::started(80, 24);
    h.press("j");
    let cmds = h.update(Msg::Key(key("f")));
    assert_eq!(
        cmds,
        [Cmd::SaveFavorite {
            path: wallpaper("beach/dunes.jpg"),
            favorite: true
        }]
    );
    assert!(h.model.is_favorite(&wallpaper("beach/dunes.jpg")));
    h.run(cmds);
    assert_eq!(
        *h.fake.favorites.lock().unwrap(),
        [wallpaper("beach/dunes.jpg")]
    );
    assert_eq!(h.model.toast.as_ref().unwrap().kind, ToastKind::Success);
    h.press("f");
    assert!(h.fake.favorites.lock().unwrap().is_empty());
    assert!(!h.model.is_favorite(&wallpaper("beach/dunes.jpg")));
}

#[test]
fn a_failed_favorite_save_is_undone_and_explained() {
    let fake = Fake {
        favorite_error: Some("cannot save favorites: read-only".into()),
        ..Fake::default()
    };
    let mut h = Harness::started_with(80, 24, fake);
    h.press("f");
    assert!(h.model.favorites.is_empty());
    assert_eq!(h.model.toast.as_ref().unwrap().kind, ToastKind::Error);
}

#[test]
fn toggling_again_while_a_save_is_in_flight_saves_the_latest() {
    let mut h = Harness::started(80, 24);
    let first = h.update(Msg::Key(key("f")));
    let second = h.update(Msg::Key(key("f")));
    // The saves come back in order; after the first, the TUI wants "no".
    h.run(first);
    h.run(second);
    assert!(h.fake.favorites.lock().unwrap().is_empty());
    assert!(h.model.favorites.is_empty());
}

#[test]
fn shift_f_shows_only_favorites_loaded_at_startup() {
    let fake = Fake::default();
    *fake.favorites.lock().unwrap() =
        vec![wallpaper("city/neon-rain.jpg"), wallpaper("aurora.png")];
    let mut h = Harness::started_with(80, 24, fake);
    h.press("j");
    h.press("F");
    assert!(h.model.favorites_only);
    // Library order, and the selection falls to the first Favorite.
    assert_eq!(names(&h), ["aurora.png", "city/neon-rain.jpg"]);
    assert_eq!(selected_name(&h), "aurora.png");
    // Unfavoriting drops it from the filtered list.
    h.press("f");
    assert_eq!(names(&h), ["city/neon-rain.jpg"]);
    h.press("F");
    assert_eq!(names(&h).len(), 4);
}

#[test]
fn question_mark_opens_help_that_filters_and_scrolls() {
    let mut h = Harness::started(80, 12);
    h.press("?");
    assert_eq!(h.model.mode, Mode::Help);
    h.press("down");
    assert_eq!(h.model.prompt.selected, 1);
    h.typed("fav");
    assert_eq!(h.model.prompt.selected, 0);
    h.press("esc");
    assert_eq!(h.model.mode, Mode::Normal);
    assert!(!h.model.quit);
}

#[test]
fn the_palette_runs_the_chosen_action_by_name() {
    let mut h = Harness::started(80, 24);
    h.press(":");
    assert_eq!(h.model.mode, Mode::Palette);
    h.typed("every monitor");
    let cmds = h.update(Msg::Key(key("enter")));
    assert_eq!(h.model.mode, Mode::Normal);
    assert_eq!(applies(&cmds)[0].monitor, None);

    h.press("ctrl-p");
    assert_eq!(h.model.mode, Mode::Palette);
    h.typed("fav");
    h.press("down");
    h.press("enter");
    // "Favorite or unfavorite" first, then "Show only Favorites".
    assert!(h.model.favorites_only);
}

#[test]
fn esc_closes_the_palette_without_running_anything() {
    let mut h = Harness::started(80, 24);
    h.press(":");
    let cmds = h.update(Msg::Key(key("esc")));
    assert!(cmds.is_empty());
    assert_eq!(h.model.mode, Mode::Normal);
}

#[test]
fn ctrl_c_quits_from_any_prompt() {
    for open in ["/", "?", ":"] {
        let mut h = Harness::started(80, 24);
        h.press(open);
        h.press("ctrl-c");
        assert!(h.model.quit, "{open}");
    }
}

#[test]
fn a_remap_changes_what_the_key_does() {
    let mut h =
        Harness::started_with_config(80, 24, "[keymap]\ndown = \"n\"\napply_all = \"enter\"\n");
    h.press("n");
    assert_eq!(h.model.selected, 1);
    h.press("j");
    assert_eq!(h.model.selected, 1, "j no longer moves");
    let cmds = h.update(Msg::Key(key("enter")));
    assert_eq!(applies(&cmds)[0].monitor, None);
}

#[test]
fn clicking_a_row_selects_it_and_a_double_click_applies() {
    let mut h = Harness::started(80, 24);
    // List rows start at y=2 (header, border).
    h.click(5, 4);
    assert_eq!(selected_name(&h), "beach/tide.webp");
    assert!(h.fake.applied.lock().unwrap().is_empty());
    h.model.now += Duration::from_millis(200);
    h.click(5, 4);
    assert_eq!(
        h.fake.applied.lock().unwrap()[0].path,
        wallpaper("beach/tide.webp")
    );
}

#[test]
fn two_slow_clicks_are_not_a_double_click() {
    let mut h = Harness::started(80, 24);
    h.click(5, 3);
    h.model.now += Duration::from_millis(600);
    h.click(5, 3);
    assert!(h.fake.applied.lock().unwrap().is_empty());
}

#[test]
fn clicking_past_the_last_row_does_nothing() {
    let mut h = Harness::started(80, 24);
    let cmds = h.update(Msg::Mouse(ratatui::crossterm::event::MouseEvent {
        kind: ratatui::crossterm::event::MouseEventKind::Down(
            ratatui::crossterm::event::MouseButton::Left,
        ),
        column: 5,
        row: 15,
        modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
    }));
    assert!(cmds.is_empty());
    assert_eq!(h.model.selected, 0);
}

#[test]
fn the_wheel_scrolls_the_list() {
    let mut h = Harness::started(80, 24);
    h.wheel(true, 5, 5);
    h.wheel(true, 5, 5);
    assert_eq!(h.model.selected, 2);
    h.wheel(false, 5, 5);
    assert_eq!(h.model.selected, 1);
}

#[test]
fn footer_hints_are_clickable() {
    let mut h = Harness::started(120, 40);
    let (x, y) = h.find("/ search").unwrap();
    h.click(x + 2, y);
    assert_eq!(h.model.mode, Mode::Search);
    let (x, y) = h.find("esc clear").unwrap();
    h.click(x, y);
    assert_eq!(h.model.mode, Mode::Normal);
    let (x, y) = h.find("? help").unwrap();
    h.click(x, y);
    assert_eq!(h.model.mode, Mode::Help);
}

#[test]
fn clicking_a_monitor_in_the_header_targets_it() {
    let mut h = Harness::started(120, 40);
    let (x, y) = h.find("HDMI-A-1").unwrap();
    h.click(x, y);
    assert_eq!(h.model.targeted().unwrap().name, "HDMI-A-1");
}

#[test]
fn clicking_a_palette_row_runs_it_and_clicking_outside_closes() {
    let mut h = Harness::started(120, 40);
    h.press(":");
    let (x, y) = h.find("Show only Favorites").unwrap();
    h.click(x, y);
    assert!(h.model.favorites_only);
    assert_eq!(h.model.mode, Mode::Normal);

    h.press("?");
    h.click(0, 20);
    assert_eq!(h.model.mode, Mode::Normal);
}

#[test]
fn the_spinner_waits_150ms_then_animates_without_blocking_keys() {
    let mut h = Harness::new(80, 24);
    h.start();
    // Scanning and connecting: wake at 150 ms, not before.
    assert_eq!(update::wake_at(&h.model), Some(update::SPINNER_DELAY));
    assert!(update::spinner(&h.model, Duration::ZERO).is_none());
    h.advance(150);
    assert_eq!(update::spinner(&h.model, Duration::ZERO), Some("⠋"));
    assert!(h.model.dirty);
    assert_eq!(update::wake_at(&h.model), Some(Duration::from_millis(230)));
    h.advance(80);
    assert_eq!(update::spinner(&h.model, Duration::ZERO), Some("⠙"));
    // Keys still work while it spins.
    h.press("?");
    assert_eq!(h.model.mode, Mode::Help);
}

#[test]
fn a_quick_preview_never_shows_a_spinner() {
    let mut h = Harness::started(80, 24);
    let cmds = h.update(Msg::Key(key("j")));
    assert!(matches!(
        h.model.preview.state,
        PreviewState::Loading { .. }
    ));
    h.model.now += Duration::from_millis(100);
    h.run(cmds);
    assert!(matches!(h.model.preview.state, PreviewState::Ready { .. }));
    assert_eq!(update::wake_at(&h.model), None);
}

#[test]
fn a_slow_apply_shows_a_spinner_until_it_finishes() {
    let mut h = Harness::started(80, 24);
    let cmds = h.update(Msg::Key(key("enter")));
    assert_eq!(h.model.applying.len(), 1);
    h.advance(200);
    assert!(h.screen().contains("Applying to DP-1…"));
    h.run(cmds);
    assert!(h.model.applying.is_empty());
    assert!(h.screen().contains("✓ Applied to DP-1"));
}

#[test]
fn the_header_shows_rotation_for_the_targeted_monitor() {
    let mut info = daemon_info(vec![
        monitor("DP-1", 2560, 1440, 120),
        monitor("HDMI-A-1", 1920, 1080, 120),
    ]);
    info.status.rotation_running = true;
    info.status.settings.rotation.schedule.interval_ms = 30 * 60_000;
    info.status
        .settings
        .rotation
        .overrides
        .push(vellum_ipc::Override {
            monitor: "HDMI-A-1".into(),
            schedule: Schedule {
                interval_ms: 90_000,
                source: RotationSource::Favorites,
                ..Schedule::default()
            },
        });
    let fake = Fake {
        daemon: Ok(info),
        ..Fake::default()
    };
    let mut h = Harness::started_with(120, 40, fake);
    assert_eq!(
        h.model.rotation,
        Rotation::On {
            interval_ms: 30 * 60_000,
            source: RotationSource::Library
        }
    );
    assert!(h.screen().contains("⟳ every 30m"));
    h.press("m");
    assert!(h.screen().contains("⟳ every 1m30s · Favorites"));
}

#[test]
fn a_stopped_rotation_reads_off() {
    let h = Harness::started(80, 24);
    assert_eq!(h.model.rotation, Rotation::Off);
}

#[test]
fn a_failed_hook_becomes_an_error_toast() {
    let mut h = Harness::started(80, 24);
    h.dispatch(Msg::HookFailed(
        "the on_change hook `wal` failed on DP-1 (exit status: 1)".into(),
    ));
    let toast = h.model.toast.clone().unwrap();
    assert_eq!(toast.kind, ToastKind::Error);
    assert!(toast.text.contains("`wal` failed"));
}
