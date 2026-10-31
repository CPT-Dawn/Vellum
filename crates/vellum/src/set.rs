//! `vellum set <file>`: decode, render for each targeted Monitor, present.
//! `vellum render --restore` shares [`present`].

use std::os::fd::AsFd;
use std::path::Path;

use image::RgbaImage;
use vellum_ipc::{MonitorInfo, PixelFormat, Present, Request, Response, Transition};

use crate::client::{self, ask, unexpected};
use crate::frame::Frame;
use crate::render::{DEFAULT_LETTERBOX, FitMode, Target, render};

/// Sets `file` on the Monitor named `monitor`, or on every Monitor, playing
/// `transition` on each.
pub fn run(
    file: &Path,
    monitor: Option<&str>,
    fit: FitMode,
    transition: Transition,
) -> Result<(), String> {
    let image = crate::decode::load(file)?;

    let socket = client::daemon()?;
    let monitors = target(client::monitors(&socket)?, monitor)?;

    let source = std::path::absolute(file).unwrap_or_else(|_| file.to_owned());
    let source = source.to_string_lossy();
    for monitor in &monitors {
        present(&socket, &image, monitor, fit, &source, transition, false)?;
    }
    Ok(())
}

/// Renders `image` for `monitor` and presents it, recording `source` and
/// `fit` in State. A `restore` frame is dropped by the Daemon if the Monitor
/// already shows an up-to-date one.
pub fn present(
    socket: &Path,
    image: &RgbaImage,
    monitor: &MonitorInfo,
    fit: FitMode,
    source: &str,
    transition: Transition,
    restore: bool,
) -> Result<(), String> {
    let (width, height) = (monitor.physical_width, monitor.physical_height);
    let mut frame = Frame::new(width, height)
        .map_err(|err| format!("cannot allocate a frame for {}: {err}", monitor.name))?;
    let target = Target::from_scale_120(width, height, monitor.scale_120);
    render(image, target, fit, DEFAULT_LETTERBOX, frame.pixels_mut());
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
        transition,
        fit: fit.into(),
        restore,
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
