//! End-to-end harness: the real `vellumd` against a headless Sway.
//!
//! Each [`Sway`] owns a private temp dir used as `XDG_RUNTIME_DIR`, `HOME`
//! and every other XDG dir, so tests never touch the user's session or files.
//! Drive Vellum only through [`Sway::vellum`] and observe it only through
//! [`Sway::screenshot`] and what the CLI prints.
//!
//! Requires `sway` and `grim` on PATH.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use image::{Rgb, RgbImage};
use tempfile::TempDir;

const TIMEOUT: Duration = Duration::from_secs(10);

/// A monitor to create on the headless backend.
#[derive(Clone, Copy)]
pub struct Monitor {
    pub width: u32,
    pub height: u32,
}

/// A headless Sway session in its own sandbox.
pub struct Sway {
    dir: TempDir,
    child: Child,
    display: String,
    daemon: Option<Child>,
}

impl Sway {
    /// Starts Sway with one headless monitor per entry, named `HEADLESS-1`,
    /// `HEADLESS-2`, ... in order.
    pub fn start(monitors: &[Monitor]) -> Sway {
        let dir = tempfile::Builder::new()
            .prefix("vellum-e2e-")
            .tempdir()
            .unwrap();
        let mut config = String::from("default_border none\n");
        for (i, m) in monitors.iter().enumerate() {
            config += &format!(
                "output HEADLESS-{} mode {}x{} position {} 0\n",
                i + 1,
                m.width,
                m.height,
                i * 10_000
            );
        }
        fs::write(dir.path().join("sway.conf"), config).unwrap();
        let log = fs::File::create(dir.path().join("sway.log")).unwrap();

        let child = Command::new("sway")
            .arg("--config")
            .arg(dir.path().join("sway.conf"))
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("XDG_RUNTIME_DIR", dir.path())
            .env("HOME", dir.path())
            .env("WLR_BACKENDS", "headless")
            .env("WLR_RENDERER", "pixman")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            .env("WLR_HEADLESS_OUTPUTS", monitors.len().to_string())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .expect("failed to spawn sway; is it installed?");

        let mut sway = Sway {
            dir,
            child,
            display: String::new(),
            daemon: None,
        };
        sway.display = sway.wait_for_display();
        sway
    }

    fn wait_for_display(&mut self) -> String {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let found = fs::read_dir(self.dir.path())
                .unwrap()
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .find(|name| name.starts_with("wayland-") && !name.ends_with(".lock"));
            if let Some(display) = found {
                // The socket appears before outputs are fully up.
                if self.swaymsg_ok() {
                    return display;
                }
            }
            if let Ok(Some(status)) = self.child.try_wait() {
                panic!("sway exited early ({status}):\n{}", self.sway_log());
            }
            assert!(
                Instant::now() < deadline,
                "sway did not start:\n{}",
                self.sway_log()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn swaymsg_ok(&self) -> bool {
        Command::new("swaymsg")
            .args(["-t", "get_outputs"])
            .env("XDG_RUNTIME_DIR", self.dir.path())
            .env("SWAYSOCK", self.sway_socket().unwrap_or_default())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    fn sway_socket(&self) -> Option<PathBuf> {
        fs::read_dir(self.dir.path())
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("sway-ipc."))
            })
    }

    fn sway_log(&self) -> String {
        fs::read_to_string(self.dir.path().join("sway.log")).unwrap_or_default()
    }

    /// The sandbox directory. Put test wallpapers here.
    pub fn dir(&self) -> &Path {
        self.dir.path()
    }

    /// A command with the sandbox environment: this Sway's display and
    /// private XDG dirs.
    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut cmd = Command::new(program);
        let d = self.dir.path();
        cmd.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("XDG_RUNTIME_DIR", d)
            .env("WAYLAND_DISPLAY", &self.display)
            .env("HOME", d)
            .env("XDG_CONFIG_HOME", d.join("config"))
            .env("XDG_STATE_HOME", d.join("state"))
            .env("XDG_CACHE_HOME", d.join("cache"))
            .env("XDG_DATA_HOME", d.join("data"));
        cmd
    }

    /// The Config file `vellum` reads in this sandbox.
    pub fn config_path(&self) -> PathBuf {
        self.dir.path().join("config/vellum/config.toml")
    }

    /// Writes the Config file in one step (temp file plus rename, as most
    /// editors save), creating its directory.
    pub fn write_config(&self, text: &str) {
        let path = self.config_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, text).unwrap();
        fs::rename(&tmp, &path).unwrap();
    }

    /// Starts `vellumd` and waits until its socket answers.
    pub fn start_daemon(&mut self) -> u32 {
        self.start_daemon_with(&[])
    }

    /// Like [`Sway::start_daemon`], with extra environment variables.
    pub fn start_daemon_with(&mut self, env: &[(&str, &str)]) -> u32 {
        let log = fs::File::create(self.dir.path().join("vellumd.log")).unwrap();
        let child = self
            .command(vellumd_bin())
            .envs(env.iter().copied())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .expect("failed to spawn vellumd");
        let pid = child.id();
        self.daemon = Some(child);
        let socket = self.socket();
        let deadline = Instant::now() + TIMEOUT;
        while std::os::unix::net::UnixStream::connect(&socket).is_err() {
            if let Some(Ok(Some(status))) = self.daemon.as_mut().map(|d| d.try_wait()) {
                panic!("vellumd exited early ({status}):\n{}", self.daemon_log());
            }
            assert!(Instant::now() < deadline, "vellumd never opened its socket");
            std::thread::sleep(Duration::from_millis(20));
        }
        pid
    }

    /// Whether the Daemon started by [`Sway::start_daemon`] is still running.
    pub fn daemon_alive(&mut self) -> bool {
        self.daemon
            .as_mut()
            .is_some_and(|d| matches!(d.try_wait(), Ok(None)))
    }

    /// Waits for the Daemon started by [`Sway::start_daemon`] to exit and
    /// returns its status, or `None` if it is still running after a few
    /// seconds.
    pub fn wait_daemon_exit(&mut self) -> Option<std::process::ExitStatus> {
        let daemon = self.daemon.as_mut().expect("no Daemon was started");
        let deadline = Instant::now() + TIMEOUT;
        while Instant::now() < deadline {
            if let Ok(Some(status)) = daemon.try_wait() {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }

    /// The Daemon's socket for this display.
    pub fn socket(&self) -> PathBuf {
        vellum_ipc::socket_path(self.dir.path(), &self.display)
    }

    /// Whether any Daemon answers on this display's socket.
    pub fn daemon_answers(&self) -> bool {
        std::os::unix::net::UnixStream::connect(self.socket()).is_ok()
    }

    /// Runs another `vellumd` in the foreground and returns its output.
    pub fn vellumd(&self) -> Output {
        self.command(vellumd_bin())
            .output()
            .expect("failed to run vellumd")
    }

    /// Kills the compositor, as a crashed or logged-out session would.
    pub fn kill_compositor(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// The Daemon's resident set size in KiB, from `/proc`.
    pub fn daemon_rss_kib(&self) -> u64 {
        let pid = self.daemon.as_ref().expect("no daemon").id();
        let status = fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
        status
            .lines()
            .find_map(|l| l.strip_prefix("VmRSS:"))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
            .expect("VmRSS in /proc status")
    }

    /// A field of the Daemon's `/proc/PID/smaps_rollup` (`"Rss"`, `"Pss"`,
    /// `"Private_Dirty"`, ...) in KiB. Unlike VmRSS it is exact, not a cached
    /// per-thread counter.
    pub fn daemon_smaps_kib(&self, field: &str) -> u64 {
        let pid = self.daemon.as_ref().expect("no daemon").id();
        let rollup = fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).unwrap();
        rollup
            .lines()
            .find_map(|l| l.strip_prefix(field)?.strip_prefix(':'))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
            .unwrap_or_else(|| panic!("{field} in smaps_rollup"))
    }

    /// Context switches of every Daemon thread so far. Each wakeup costs at
    /// least one, so an idle Daemon's count stands still.
    pub fn daemon_context_switches(&self) -> u64 {
        let pid = self.daemon.as_ref().expect("no daemon").id();
        fs::read_dir(format!("/proc/{pid}/task"))
            .unwrap()
            .filter_map(|task| fs::read_to_string(task.ok()?.path().join("status")).ok())
            .flat_map(|status| {
                status
                    .lines()
                    .filter(|l| l.contains("ctxt_switches:"))
                    .filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
                    .collect::<Vec<_>>()
            })
            .sum()
    }

    /// Whether the Daemon currently maps any memfd (pixel memory).
    pub fn daemon_maps_memfd(&self) -> bool {
        let pid = self.daemon.as_ref().expect("no daemon").id();
        fs::read_to_string(format!("/proc/{pid}/maps"))
            .unwrap()
            .contains("memfd:")
    }

    pub fn daemon_log(&self) -> String {
        fs::read_to_string(self.dir.path().join("vellumd.log")).unwrap_or_default()
    }

    /// Runs `vellum` with `args` and returns its output.
    pub fn vellum<I, S>(&self, args: I) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.command(env!("CARGO_BIN_EXE_vellum"))
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("failed to run vellum")
    }

    /// Runs `vellum` with `args` and panics with its stderr if it fails.
    pub fn vellum_ok<I, S>(&self, args: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let out = self.vellum(args);
        assert!(
            out.status.success(),
            "vellum failed ({}):\n{}\n--- vellumd log ---\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr),
            self.daemon_log()
        );
    }

    /// Runs a Sway IPC command (e.g. `output HEADLESS-1 scale 1.5`,
    /// `create_output`, `output HEADLESS-2 unplug`) and panics if it fails.
    pub fn swaymsg(&self, command: &str) {
        let out = self
            .command("swaymsg")
            .env(
                "SWAYSOCK",
                self.sway_socket().expect("sway has an IPC socket"),
            )
            .arg(command)
            .output()
            .expect("failed to run swaymsg");
        assert!(
            out.status.success(),
            "swaymsg {command:?} failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Runs `vellum` with `args`, panics if it fails, and returns its stdout.
    pub fn vellum_stdout<I, S>(&self, args: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let out = self.vellum(args);
        assert!(
            out.status.success(),
            "vellum failed ({}):\n{}\n--- vellumd log ---\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr),
            self.daemon_log()
        );
        String::from_utf8(out.stdout).expect("vellum prints UTF-8")
    }

    /// Polls until `ready` holds, panicking with `what` after a few seconds.
    pub fn wait_until(&self, what: &str, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    /// Captures `monitor` with grim.
    pub fn screenshot(&self, monitor: &str) -> RgbImage {
        let out = self
            .command("grim")
            .args(["-o", monitor, "-t", "png", "-"])
            .output()
            .expect("failed to run grim; is it installed?");
        assert!(
            out.status.success(),
            "grim failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        image::load_from_memory(&out.stdout).unwrap().to_rgb8()
    }

    /// Captures `monitor` until `ready` accepts the image, so a test does not
    /// race the compositor's next repaint. Returns the last capture either way.
    pub fn screenshot_until(&self, monitor: &str, ready: impl Fn(&RgbImage) -> bool) -> RgbImage {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let shot = self.screenshot(monitor);
            if ready(&shot) || Instant::now() > deadline {
                return shot;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

impl Drop for Sway {
    fn drop(&mut self) {
        if let Some(mut daemon) = self.daemon.take() {
            let _ = daemon.kill();
            let _ = daemon.wait();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn vellumd_bin() -> PathBuf {
    let bin = Path::new(env!("CARGO_BIN_EXE_vellum")).with_file_name("vellumd");
    assert!(
        bin.exists(),
        "{} is missing; run the e2e tests with `cargo test --workspace`",
        bin.display()
    );
    bin
}

/// Asserts that every pixel channel is within `tolerance` of `expected`.
#[track_caller]
pub fn assert_pixel(img: &RgbImage, x: u32, y: u32, expected: [u8; 3], tolerance: u8) {
    let Rgb(actual) = *img.get_pixel(x, y);
    let close = actual
        .iter()
        .zip(expected)
        .all(|(a, e)| a.abs_diff(e) <= tolerance);
    assert!(
        close,
        "pixel ({x}, {y}) is {actual:?}, expected {expected:?} (±{tolerance})"
    );
}

pub const RED: [u8; 3] = [255, 0, 0];
pub const GREEN: [u8; 3] = [0, 255, 0];
pub const BLUE: [u8; 3] = [0, 0, 255];
pub const WHITE: [u8; 3] = [255, 255, 255];
