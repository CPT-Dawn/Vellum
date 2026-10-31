//! Talking to the Daemon: one request per connection, with errors phrased
//! for the person at the terminal.

use std::fs;
use std::os::fd::BorrowedFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use vellum_ipc::{MonitorInfo, PROTOCOL_VERSION, Request, Response, Status};

/// How long an auto-started Daemon may take to answer. It waits up to 3 s
/// for slow Monitors itself.
const START_TIMEOUT: Duration = Duration::from_secs(8);

/// The Daemon's socket for this Wayland display.
pub fn socket() -> Result<PathBuf, String> {
    vellum_ipc::socket_path_from_env().map_err(|err| err.to_string())
}

/// The Daemon's socket, starting `vellumd` first if nothing answers there.
pub fn daemon() -> Result<PathBuf, String> {
    let socket = socket()?;
    if UnixStream::connect(&socket).is_ok() {
        return Ok(socket);
    }
    start_daemon(&socket)?;
    Ok(socket)
}

/// Whether a Daemon answers at `socket`.
pub fn is_running(socket: &Path) -> bool {
    UnixStream::connect(socket).is_ok()
}

/// Starts `vellumd` in the background and waits until it answers. Its log
/// goes to `$XDG_STATE_HOME/vellum/vellumd.log`.
fn start_daemon(socket: &Path) -> Result<(), String> {
    let log = log_path();
    let stderr = log
        .as_deref()
        .and_then(|path| {
            fs::create_dir_all(path.parent()?).ok()?;
            fs::File::create(path).ok()
        })
        .map_or_else(Stdio::null, Stdio::from);
    let mut child = Command::new(vellumd_bin())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        // Its own process group, so Ctrl-C on this command spares it.
        .process_group(0)
        .spawn()
        .map_err(|err| format!("cannot start vellumd: {err}"))?;

    let deadline = Instant::now() + START_TIMEOUT;
    let mut exited_at = None;
    while Instant::now() < deadline {
        if is_running(socket) {
            return Ok(());
        }
        if exited_at.is_none() && matches!(child.try_wait(), Ok(Some(_))) {
            // Another command may have started a Daemon at the same time;
            // give that one a moment before giving up.
            exited_at = Some(Instant::now());
        }
        if exited_at.is_some_and(|t| t.elapsed() > Duration::from_secs(1)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let why = log
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .map_or_else(String::new, |text| format!(":\n{text}"));
    Err(format!("vellumd did not start{why}"))
}

fn log_path() -> Option<PathBuf> {
    state_dir().map(|dir| dir.join("vellumd.log"))
}

/// `$XDG_STATE_HOME/vellum`, falling back to `~/.local/state/vellum`.
pub fn state_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|dir| Path::new(dir).is_absolute())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/state")))?;
    Some(base.join("vellum"))
}

/// `vellumd` next to this `vellum`, or else the one on PATH.
fn vellumd_bin() -> PathBuf {
    std::env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name("vellumd"))
        .filter(|bin| bin.exists())
        .unwrap_or_else(|| PathBuf::from("vellumd"))
}

/// Sends one request to the Daemon, turning "no Daemon" and version
/// mismatches into helpful hints.
pub fn ask(socket: &Path, request: &Request, fds: &[BorrowedFd<'_>]) -> Result<Response, String> {
    vellum_ipc::request(socket, request, fds).map_err(|err| describe(socket, err))
}

fn describe(socket: &Path, err: vellum_ipc::Error) -> String {
    match err {
        vellum_ipc::Error::Io(io) if is_not_running(&io) => format!(
            "vellumd is not running (no Daemon at {}); start it with `vellumd &`",
            socket.display()
        ),
        vellum_ipc::Error::VersionMismatch { ours, theirs } => format!(
            "vellumd speaks protocol v{theirs} but this vellum ({}) speaks v{ours}; \
             restart the Daemon so both come from the same release: \
             `vellum kill`, then run your command again",
            env!("CARGO_PKG_VERSION")
        ),
        other => format!("talking to vellumd failed: {other}"),
    }
}

/// Every Monitor the Daemon can present to.
pub fn monitors(socket: &Path) -> Result<Vec<MonitorInfo>, String> {
    match ask(socket, &Request::Monitors, &[])? {
        Response::Monitors(monitors) => Ok(monitors),
        other => Err(unexpected(&other)),
    }
}

/// What the Daemon shows and remembers.
pub fn status(socket: &Path) -> Result<Status, String> {
    match ask(socket, &Request::Status, &[])? {
        Response::Status(status) => Ok(status),
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

/// `vellum kill`: stops the Daemon and waits until it is gone. A Daemon
/// from another release cannot read the request, so it gets SIGTERM.
pub fn kill() -> Result<(), String> {
    let socket = socket()?;
    let Ok(stream) = UnixStream::connect(&socket) else {
        eprintln!("vellum: vellumd is not running");
        return Ok(());
    };
    let pid = rustix::net::sockopt::socket_peercred(&stream)
        .ok()
        .map(|cred| cred.pid);
    drop(stream);
    match vellum_ipc::request(&socket, &Request::Kill, &[]) {
        Ok(Response::Ok) => {}
        Ok(Response::Error(err)) => return Err(err),
        Ok(other) => return Err(unexpected(&other)),
        Err(vellum_ipc::Error::VersionMismatch { theirs, .. }) if theirs != PROTOCOL_VERSION => {
            let pid = pid.ok_or("cannot find vellumd's process to stop it")?;
            rustix::process::kill_process(pid, rustix::process::Signal::TERM)
                .map_err(|err| format!("cannot stop vellumd: {err}"))?;
        }
        Err(err) => return Err(describe(&socket, err)),
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while is_running(&socket) {
        if Instant::now() > deadline {
            return Err("vellumd did not stop".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}
