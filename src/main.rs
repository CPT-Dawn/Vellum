mod app;
mod backend;
mod persistence;
mod ui;
mod wallpapers;

use std::{
    env, io,
    path::{Path, PathBuf},
    time::Duration,
};

use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use directories::UserDirs;
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::{mpsc, watch};

use crate::{
    app::{App, AppAction, PlaylistControl},
    backend::{
        awww::{ApplyRequest, AwwwClient, TransitionSettings},
        monitors::{self, MonitorInfo},
    },
    persistence::{load_state, save_state, state_file_path},
    wallpapers::discover_wallpapers,
};

/// Application-wide result type.
type AppResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Program entrypoint.
#[tokio::main]
async fn main() -> AppResult<()> {
    let wallpapers = discover_initial_wallpapers()?;
    let monitors = discover_initial_monitors().await;
    let state_path = state_file_path()?;
    let stored_state = load_state(&state_path).unwrap_or_default();
    let mut app = App::new(wallpapers, monitors, stored_state);

    let awww = AwwwClient::default();
    if let Err(err) = awww.start_daemon().await {
        app.set_status(format!("awww-daemon startup warning: {err}"));
    }

    let mut terminal = init_terminal()?;
    let result = run_app(&mut terminal, app, &awww, state_path).await;
    restore_terminal(&mut terminal)?;
    result
}

/// Enables raw terminal mode and configures the alternate screen.
fn init_terminal() -> AppResult<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Restores terminal state so the shell works correctly after exit.
fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> AppResult<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Runs the main event/render loop until the user requests exit.
async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mut app: App,
    awww: &AwwwClient,
    state_path: PathBuf,
) -> AppResult<()> {
    const TICK_RATE: Duration = Duration::from_millis(120);

    let (playlist_tick_tx, mut playlist_tick_rx) = mpsc::unbounded_channel::<()>();
    let (playlist_cfg_tx, playlist_cfg_rx) = watch::channel(app.playlist_control());
    let _playlist_worker = tokio::spawn(playlist_worker(playlist_cfg_rx, playlist_tick_tx));

    loop {
        while playlist_tick_rx.try_recv().is_ok() {
            execute_action(&mut app, awww, AppAction::AutoCycleNext, &state_path).await;
            let _ = playlist_cfg_tx.send(app.playlist_control());
        }

        terminal.draw(|frame| ui::render(frame, &app))?;

        if event::poll(TICK_RATE)? {
            if let Event::Key(key) = event::read()? {
                let actions = app.on_key(key);
                for action in actions {
                    execute_action(&mut app, awww, action, &state_path).await;
                    let _ = playlist_cfg_tx.send(app.playlist_control());
                }
            }
        } else {
            app.on_tick();
        }

        if app.should_quit {
            break;
        }

        tokio::task::yield_now().await;
    }

    Ok(())
}

/// Tokio worker that emits periodic ticks when playlist auto-cycle is enabled.
async fn playlist_worker(
    mut cfg_rx: watch::Receiver<PlaylistControl>,
    tick_tx: mpsc::UnboundedSender<()>,
) {
    loop {
        let cfg = *cfg_rx.borrow();
        if !cfg.enabled {
            if cfg_rx.changed().await.is_err() {
                return;
            }
            continue;
        }

        tokio::select! {
            _ = tokio::time::sleep(cfg.interval) => {
                if tick_tx.send(()).is_err() {
                    return;
                }
            }
            changed = cfg_rx.changed() => {
                if changed.is_err() {
                    return;
                }
            }
        }
    }
}

/// Executes one app action and updates app status with success/failure details.
async fn execute_action(app: &mut App, awww: &AwwwClient, action: AppAction, state_path: &PathBuf) {
    match action {
        AppAction::TryOnSelected => {
            let Some(path) = app.selected_wallpaper_path() else {
                app.set_status("no wallpaper selected");
                return;
            };

            match apply_for_selection(app, awww, &path).await {
                Ok(()) => {
                    app.mark_preview_active();
                    app.set_status(format!("live preview: {}", path.display()));
                }
                Err(err) => app.set_status(format!("preview failed: {err}")),
            }
        }
        AppAction::ConfirmSelected => {
            let Some(path) = app.selected_wallpaper_path() else {
                app.set_status("no wallpaper selected");
                return;
            };

            match apply_for_selection(app, awww, &path).await {
                Ok(()) => {
                    app.mark_confirmed();
                    app.set_status(format!("confirmed: {}", path.display()));
                }
                Err(err) => app.set_status(format!("confirm failed: {err}")),
            }
        }
        AppAction::CancelPreview => {
            if let Some(path) = app.confirmed_wallpaper.clone() {
                match apply_for_selection(app, awww, &path).await {
                    Ok(()) => {
                        app.clear_preview();
                        app.set_status(format!("reverted preview to: {}", path.display()));
                    }
                    Err(err) => app.set_status(format!("revert failed: {err}")),
                }
            } else {
                let output = app.selected_monitor_name();
                match awww.clear_wallpaper(output).await {
                    Ok(()) => {
                        app.clear_preview();
                        app.set_status("preview canceled: cleared wallpaper".to_owned());
                    }
                    Err(err) => app.set_status(format!("cancel failed: {err}")),
                }
            }
        }
        AppAction::SaveQuickProfile => {
            app.save_quick_profile();
            if let Err(err) = save_state(state_path, &app.stored_state) {
                app.set_status(format!("profile save failed: {err}"));
            }
        }
        AppAction::LoadQuickProfile => {
            app.load_quick_profile();
            let Some(path) = app.selected_wallpaper_path() else {
                return;
            };

            match apply_for_selection(app, awww, &path).await {
                Ok(()) => {
                    app.mark_preview_active();
                    app.set_status(format!("profile applied: {}", path.display()));
                }
                Err(err) => app.set_status(format!("profile apply failed: {err}")),
            }
        }
        AppAction::TogglePlaylist => {
            app.toggle_playlist();
            if let Err(err) = save_state(state_path, &app.stored_state) {
                app.set_status(format!("playlist state save failed: {err}"));
            }
        }
        AppAction::AutoCycleNext => {
            if !app.has_active_playlist_entries() {
                app.set_status("auto-cycle skipped: playlist empty");
                return;
            }

            let Some(path) = app.next_playlist_path() else {
                app.set_status("auto-cycle skipped: no path");
                return;
            };

            app.select_wallpaper_by_path(path.clone());
            match apply_for_selection(app, awww, &path).await {
                Ok(()) => {
                    app.mark_confirmed();
                    app.set_status(format!("playlist apply: {}", path.display()));
                }
                Err(err) => app.set_status(format!("playlist apply failed: {err}")),
            }
        }
    }
}

/// Applies wallpaper using current transition and selected output settings.
async fn apply_for_selection(app: &App, awww: &AwwwClient, path: &Path) -> AppResult<()> {
    let outputs = app
        .selected_monitor_name()
        .map(str::to_owned)
        .into_iter()
        .collect::<Vec<_>>();

    let transition = TransitionSettings {
        kind: app.transition_kind,
        step: app.transition_step,
        fps: app.transition_fps,
    };

    let request = ApplyRequest {
        image_path: path,
        outputs: &outputs,
        transition,
    };

    awww.apply_wallpaper(&request).await?;
    Ok(())
}

/// Discovers monitor metadata with graceful fallback to an empty list.
async fn discover_initial_monitors() -> Vec<MonitorInfo> {
    monitors::query_monitors().await.unwrap_or_default()
}

/// Discovers wallpapers from configured root and returns an owned list.
fn discover_initial_wallpapers() -> AppResult<Vec<wallpapers::WallpaperItem>> {
    let root = resolve_wallpaper_root();
    let items = discover_wallpapers(&root)?;
    Ok(items)
}

/// Resolves wallpaper root from environment variable, then user picture directory.
#[must_use]
fn resolve_wallpaper_root() -> PathBuf {
    if let Some(custom) = env::var_os("AWWW_TUI_WALLPAPER_DIR") {
        return PathBuf::from(custom);
    }

    if let Some(user_dirs) = UserDirs::new() {
        if let Some(pictures) = user_dirs.picture_dir() {
            return pictures.to_path_buf();
        }
    }

    PathBuf::from(".")
}
