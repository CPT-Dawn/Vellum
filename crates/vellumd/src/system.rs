//! The Daemon's ties to the rest of the system: the single-instance lock,
//! systemd readiness, its own memory use, and the `vellum` renderer it
//! spawns.

use std::os::fd::OwnedFd;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use rustix::fs::{FlockOperation, Mode, OFlags, flock, open};

/// Takes the per-display lock next to the socket, held for the Daemon's
/// whole life. The kernel drops it when the Daemon exits, however it exits,
/// so a stale socket is always safe to replace.
pub fn lock_display(socket: &Path) -> Result<OwnedFd, String> {
    let path = socket.with_extension("lock");
    let fd = open(
        &path,
        OFlags::CREATE | OFlags::RDWR | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|err| format!("cannot open {}: {err}", path.display()))?;
    match flock(&fd, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(fd),
        Err(rustix::io::Errno::WOULDBLOCK) => Err(format!(
            "another vellumd is already running on this display ({}); \
             stop it with `vellum kill` first",
            socket.display()
        )),
        Err(err) => Err(format!("cannot lock {}: {err}", path.display())),
    }
}

/// Tells systemd the Daemon is ready, if it runs as a `Type=notify` unit.
pub fn notify_ready() {
    let Some(target) = std::env::var_os("NOTIFY_SOCKET") else {
        return;
    };
    let result = UnixDatagram::unbound().and_then(|socket| {
        let bytes = target.as_encoded_bytes();
        match bytes.strip_prefix(b"@") {
            Some(name) => {
                use std::os::linux::net::SocketAddrExt;
                let addr = std::os::unix::net::SocketAddr::from_abstract_name(name)?;
                socket.send_to_addr(b"READY=1\n", &addr)
            }
            None => socket.send_to(b"READY=1\n", &target),
        }
    });
    if let Err(err) = result {
        eprintln!("vellumd: cannot notify systemd: {err}");
    }
}

/// The Daemon's resident memory in bytes.
pub fn rss_bytes() -> u64 {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let pages: u64 = statm
        .split_whitespace()
        .nth(1)
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    pages * rustix::param::page_size() as u64
}

/// A running `vellum render` and a pidfd that turns readable when it exits.
pub struct Renderer {
    pub child: Child,
    pub pidfd: OwnedFd,
    pub job: Job,
}

/// What a `vellum render` was spawned to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    /// `--restore --monitor <name>`: put State's wallpaper back.
    Restore(String),
    /// `--sync`: read the Config and push its settings.
    Sync,
}

impl std::fmt::Display for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Job::Restore(monitor) => write!(f, "restoring {monitor}"),
            Job::Sync => f.write_str("reading the Config"),
        }
    }
}

/// Spawns `vellum render` for `job`. Its stderr is the Daemon's, so render
/// and Config errors land in the Daemon's log.
pub fn spawn_render(job: Job) -> Result<Renderer, String> {
    let mut command = Command::new(vellum_bin());
    match &job {
        Job::Restore(monitor) => command.args(["render", "--restore", "--monitor", monitor]),
        Job::Sync => command.args(["render", "--sync"]),
    };
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|err| format!("cannot start `vellum render`: {err}"))?;
    let pid = rustix::process::Pid::from_child(&child);
    let pidfd = rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty())
        .map_err(|err| format!("cannot watch `vellum render`: {err}"))?;
    Ok(Renderer { child, pidfd, job })
}

/// `vellum` next to this `vellumd`, or else the one on PATH.
fn vellum_bin() -> PathBuf {
    std::env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name("vellum"))
        .filter(|bin| bin.exists())
        .unwrap_or_else(|| PathBuf::from("vellum"))
}
