//! `vellum render --sync` and `vellum reload`: read the Config and push the
//! settings the Daemon holds. The Daemon spawns `--sync` once at startup
//! and whenever its inotify watch sees the Config directory change.
//!
//! A malformed Config is an error here and nothing is pushed, so the last
//! good settings stay in effect. A valid one is saved as the last good
//! Config, which commands fall back to while the file is broken.

use std::path::Path;

use vellum_ipc::{FitMode as WireFit, Request, Response};

use crate::client::{self, ask, unexpected};
use crate::config::{self, Config};
use crate::render::FitMode;
use crate::restore::INSTANT;
use crate::set::{Look, present};

pub fn run(socket: &Path) -> Result<(), String> {
    let (config, text) = config::read_current()?;
    let previous = config::last_good();
    match ask(socket, &Request::Settings(config.settings()), &[])? {
        Response::Ok => {}
        Response::Error(err) => return Err(err),
        other => return Err(unexpected(&other)),
    }
    config::save_last_good(text.as_deref())?;
    if config.letterbox != previous.letterbox {
        repaint_letterboxes(socket, &config)?;
    }
    Ok(())
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
        present(socket, &image, info, &wallpaper.path, look)?;
    }
    Ok(())
}
