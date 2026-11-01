//! The Daemon's ties to the rest of the system: the single-instance lock,
//! systemd readiness, its own memory use, the Rotation timerfd, and the
//! `vellum` renderer it spawns.

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
    /// `--rotate --ticket <n> --monitor <name>...`: a Rotation tick for
    /// these Monitors, holding the ticket its frames carry back (see
    /// [`crate::rotation::ManualApplies`]).
    Rotate(u64, Vec<String>),
}

impl std::fmt::Display for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Job::Restore(monitor) => write!(f, "restoring {monitor}"),
            Job::Sync => f.write_str("reading the Config"),
            Job::Rotate(_, monitors) => write!(f, "rotating {}", monitors.join(", ")),
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
        Job::Rotate(ticket, monitors) => {
            command.args(["render", "--rotate", "--ticket", &ticket.to_string()]);
            for monitor in monitors {
                command.args(["--monitor", monitor]);
            }
            &mut command
        }
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

/// A one-shot `CLOCK_REALTIME` timerfd armed at a wall-clock deadline. It
/// is readable from the deadline on, and silent while disarmed.
pub struct Timer {
    fd: OwnedFd,
    /// The deadline it is armed at, in Unix milliseconds.
    armed: Option<u64>,
}

impl Timer {
    pub fn new() -> Result<Timer, String> {
        use rustix::time::{TimerfdClockId, TimerfdFlags, timerfd_create};
        let fd = timerfd_create(
            TimerfdClockId::Realtime,
            TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK,
        )
        .map_err(|err| format!("cannot create the Rotation timer: {err}"))?;
        Ok(Timer { fd, armed: None })
    }

    pub fn fd(&self) -> std::os::fd::BorrowedFd<'_> {
        use std::os::fd::AsFd;
        self.fd.as_fd()
    }

    /// Arms the timer at `deadline` (Unix ms), or disarms it with `None`.
    pub fn set(&mut self, deadline: Option<u64>) {
        use rustix::time::{Itimerspec, TimerfdTimerFlags, Timespec, timerfd_settime};
        if self.armed == deadline {
            return;
        }
        let zero = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let value = match deadline {
            // An all-zero value would disarm instead; keep at least 1 ns.
            Some(ms) => Timespec {
                tv_sec: (ms / 1000) as i64,
                tv_nsec: ((ms % 1000) * 1_000_000).max(1) as i64,
            },
            None => zero,
        };
        let spec = Itimerspec {
            it_interval: zero,
            it_value: value,
        };
        match timerfd_settime(&self.fd, TimerfdTimerFlags::ABSTIME, &spec) {
            Ok(_) => self.armed = deadline,
            Err(err) => eprintln!("vellumd: cannot arm the Rotation timer: {err}"),
        }
    }

    /// Clears a fired timer. It stays disarmed until the next [`Timer::set`].
    pub fn acknowledge(&mut self) {
        let mut buf = [0u8; 8];
        let _ = rustix::io::read(&self.fd, &mut buf);
        self.armed = None;
    }
}
