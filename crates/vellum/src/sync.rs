//! `vellum render --sync` and `vellum reload`: read the Config and push the
//! settings the Daemon holds. The Daemon spawns `--sync` once at startup
//! and whenever its inotify watch sees the Config directory change.
//!
//! A malformed Config is an error here, and the last good settings are
//! pushed instead, so they stay in effect even in a freshly started Daemon
//! (which otherwise holds the defaults). A valid one is saved as the last
//! good Config, which commands fall back to while the file is broken.

use std::path::Path;

use vellum_ipc::{FitMode as WireFit, Request, Response};

use crate::client::{self, ask, unexpected};
use crate::config::{self, Config};
use crate::render::FitMode;
use crate::restore::INSTANT;
use crate::set::{Look, present_frame};

pub fn run(socket: &Path) -> Result<(), String> {
    let previous = config::last_good();
    let (config, text) = match config::read_current() {
        Ok(current) => current,
        Err(err) => {
            push(socket, &previous)?;
            return Err(err);
        }
    };
    push(socket, &config)?;
    config::save_last_good(text.as_deref())?;
    if config.letterbox != previous.letterbox {
        repaint_letterboxes(socket, &config)?;
    }
    Ok(())
}

fn push(socket: &Path, config: &Config) -> Result<(), String> {
    match ask(socket, &Request::Settings(config.settings()), &[])? {
        Response::Ok => Ok(()),
        Response::Error(err) => Err(err),
        other => Err(unexpected(&other)),
    }
}

/// Re-renders every Monitor showing a wallpaper in Fit mode, so a new
/// letterbox color shows at once.
fn repaint_letterboxes(socket: &Path, config: &Config) -> Result<(), String> {
    let status = client::status(socket)?;
    let look = Look {
        fit: FitMode::Fit,
        letterbox: config.letterbox,
        transition: INSTANT,
        restore: false,
    };
    let mut monitors = None;
    for shown in status.monitors {
        let Some(wallpaper) = shown.wallpaper.filter(|w| w.fit == WireFit::Fit) else {
            continue;
        };
        let monitors = match &monitors {
            Some(monitors) => monitors,
            None => monitors.insert(client::monitors(socket)?),
        };
        let Some(info) = monitors.iter().find(|m| m.name == shown.name) else {
            continue;
        };
        let image = crate::decode::load(wallpaper.path.as_ref())?;
        // Same wallpaper, new bars: not a change for the on_change hook.
        present_frame(socket, &image, info, &wallpaper.path, look)?;
    }
    Ok(())
}
