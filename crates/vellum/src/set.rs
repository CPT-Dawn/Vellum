//! `vellum set <file>`: decode, render for each Monitor, present.

use std::os::fd::{AsFd, BorrowedFd};
use std::path::Path;

use vellum_ipc::{PixelFormat, Present, Request, Response};

use crate::frame::Frame;
use crate::render::render_fill;

pub fn run(file: &Path) -> Result<(), String> {
    let image = image::ImageReader::open(file)
        .map_err(|err| format!("cannot open {}: {err}", file.display()))?
        .with_guessed_format()
        .map_err(|err| format!("cannot read {}: {err}", file.display()))?
        .decode()
        .map_err(|err| format!("cannot decode {}: {err}", file.display()))?
        .into_rgba8();

    let socket = vellum_ipc::socket_path_from_env().map_err(|err| err.to_string())?;
    let monitors = match ask(&socket, &Request::Monitors, &[])? {
        Response::Monitors(monitors) => monitors,
        other => return Err(unexpected(&other)),
    };
    if monitors.is_empty() {
        return Err("vellumd sees no monitors".into());
    }

    let source = std::path::absolute(file).unwrap_or_else(|_| file.to_owned());
    for monitor in monitors {
        let (width, height) = (monitor.physical_width, monitor.physical_height);
        let mut frame = Frame::new(width, height)
            .map_err(|err| format!("cannot allocate a frame for {}: {err}", monitor.name))?;
        render_fill(&image, width, height, frame.pixels_mut());
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
            source: source.to_string_lossy().into_owned(),
        });
        match ask(&socket, &present, &[memfd.as_fd()])? {
            Response::Ok => {}
            Response::Error(err) => return Err(format!("{}: {err}", monitor.name)),
            other => return Err(unexpected(&other)),
        }
    }
    Ok(())
}

/// Sends one request to the Daemon, turning "no Daemon" into a helpful hint.
fn ask(socket: &Path, request: &Request, fds: &[BorrowedFd<'_>]) -> Result<Response, String> {
    vellum_ipc::request(socket, request, fds).map_err(|err| match err {
        vellum_ipc::Error::Io(io) if is_not_running(&io) => format!(
            "vellumd is not running (no Daemon at {}); start it with `vellumd &`",
            socket.display()
        ),
        other => format!("talking to vellumd failed: {other}"),
    })
}

fn is_not_running(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
    )
}

fn unexpected(response: &Response) -> String {
    format!("vellumd sent an unexpected reply: {response:?}")
}
