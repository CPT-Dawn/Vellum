//! Talking to the Daemon: one request per connection, with errors phrased
//! for the person at the terminal.

use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};

use vellum_ipc::{MonitorInfo, Request, Response};

/// The Daemon's socket for this Wayland display.
pub fn socket() -> Result<PathBuf, String> {
    vellum_ipc::socket_path_from_env().map_err(|err| err.to_string())
}

/// Sends one request to the Daemon, turning "no Daemon" into a helpful hint.
pub fn ask(socket: &Path, request: &Request, fds: &[BorrowedFd<'_>]) -> Result<Response, String> {
    vellum_ipc::request(socket, request, fds).map_err(|err| match err {
        vellum_ipc::Error::Io(io) if is_not_running(&io) => format!(
            "vellumd is not running (no Daemon at {}); start it with `vellumd &`",
            socket.display()
        ),
        other => format!("talking to vellumd failed: {other}"),
    })
}

/// Every Monitor the Daemon can present to.
pub fn monitors(socket: &Path) -> Result<Vec<MonitorInfo>, String> {
    match ask(socket, &Request::Monitors, &[])? {
        Response::Monitors(monitors) => Ok(monitors),
        other => Err(unexpected(&other)),
    }
}

fn is_not_running(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
    )
}

pub fn unexpected(response: &Response) -> String {
    format!("vellumd sent an unexpected reply: {response:?}")
}
