//! `vellum set <file>`: decode, render for each targeted Monitor, present.
//! `vellum render --restore` and `--sync` share [`present`].

use std::os::fd::AsFd;
use std::path::Path;

use image::RgbaImage;
use vellum_ipc::{MonitorInfo, PixelFormat, Present, Request, Response, Transition};

use crate::client::{self, ask, unexpected};
use crate::config::Config;
use crate::frame::Frame;
use crate::render::{Color, FitMode, Target, render};

/// How a frame is rendered and presented.
#[derive(Debug, Clone, Copy)]
pub struct Look {
    pub fit: FitMode,
    pub letterbox: Color,
    pub transition: Transition,
    /// A restore frame is dropped by the Daemon if the Monitor already
    /// shows an up-to-date one.
    pub restore: bool,
}

/// Sets `file` (a path, or a name in the Library) on the Monitor named
/// `monitor`, or on every Monitor. `fit` and `transition` already have the
/// command's flags applied over the Config.
pub fn run(
    file: &Path,
    monitor: Option<&str>,
    config: &Config,
    fit: FitMode,
    transition: Transition,
) -> Result<(), String> {
    let file = crate::library::resolve(file, &config.library)?;
    let image = crate::decode::load(&file)?;

    let socket = client::daemon()?;
    let monitors = target(client::monitors(&socket)?, monitor)?;

    let source = std::path::absolute(&file).unwrap_or(file);
    let source = source.to_string_lossy();
    let look = Look {
        fit,
        letterbox: config.letterbox,
        transition,
        restore: false,
    };
    for monitor in &monitors {
        present(&socket, &image, monitor, &source, look)?;
    }
    Ok(())
}

/// Renders `image` for `monitor` and presents it, recording `source` and
/// the Fit mode in State.
///
/// This is the one place a new wallpaper reaches a Monitor, so it is where
/// the Config's `on_change` hook fires: after the Daemon accepts the frame,
/// unless `look.restore` is set (a restore brings back a wallpaper that
/// was already applied, so the hook ran back then).
pub fn present(
    socket: &Path,
    image: &RgbaImage,
    monitor: &MonitorInfo,
    source: &str,
    look: Look,
) -> Result<(), String> {
    present_frame(socket, image, monitor, source, look)?;
    if !look.restore {
        crate::hook::fire(source, &monitor.name);
    }
    Ok(())
}

/// [`present`] without the `on_change` hook, for re-presenting the
/// wallpaper a Monitor already shows (a letterbox repaint).
pub fn present_frame(
    socket: &Path,
    image: &RgbaImage,
    monitor: &MonitorInfo,
    source: &str,
    look: Look,
) -> Result<(), String> {
    let (width, height) = (monitor.physical_width, monitor.physical_height);
    let mut frame = Frame::new(width, height)
        .map_err(|err| format!("cannot allocate a frame for {}: {err}", monitor.name))?;
    let target = Target::from_scale_120(width, height, monitor.scale_120);
    render(image, target, look.fit, look.letterbox, frame.pixels_mut());
    let stride = frame.stride;
    let memfd = frame
        .finish()
        .map_err(|err| format!("cannot seal the frame: {err}"))?;
    let present = Request::Present(Present {
        monitor: monitor.name.clone(),
        width,
        height,
        stride,
        format: PixelFormat::Xrgb8888,
        source: source.to_owned(),
        transition: look.transition,
        fit: look.fit.into(),
        restore: look.restore,
    });
    match ask(socket, &present, &[memfd.as_fd()])? {
        Response::Ok => Ok(()),
        Response::Error(err) => Err(format!("{}: {err}", monitor.name)),
        other => Err(unexpected(&other)),
    }
}

/// The Monitors a command applies to: the one named, or all of them.
pub fn target(monitors: Vec<MonitorInfo>, name: Option<&str>) -> Result<Vec<MonitorInfo>, String> {
    if monitors.is_empty() {
        return Err("vellumd sees no monitors".into());
    }
    let Some(name) = name else {
        return Ok(monitors);
    };
    if monitors.iter().any(|m| m.name == name) {
        return Ok(monitors.into_iter().filter(|m| m.name == name).collect());
    }
    let known: Vec<&str> = monitors.iter().map(|m| m.name.as_str()).collect();
    Err(format!(
        "no monitor named {name}; connected monitors: {}",
        known.join(", ")
    ))
}
