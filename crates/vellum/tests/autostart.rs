//! Auto-start races: two commands starting the Daemon at once must both
//! succeed, and the second must not clobber the first Daemon's log.
//! A fake `vellumd` next to a copy of `vellum` stands in for the real one.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Output};
use std::time::Duration;

/// Takes the display lock like the real Daemon, takes 2 s to bind (slow
/// Monitors), logs to stderr, and exits on its own after a while.
const FAKE_VELLUMD: &str = r#"#!/usr/bin/env python3
import fcntl, os, socket, sys, time
sock = os.path.join(os.environ["XDG_RUNTIME_DIR"], "vellum-" + os.environ["WAYLAND_DISPLAY"] + ".sock")
lock = open(sock[:-len(".sock")] + ".lock", "w")
try:
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
except OSError:
    sys.stderr.write("another vellumd is already running on this display\n")
    sys.exit(1)
sys.stderr.write("fake vellumd: waiting for monitors\n")
sys.stderr.flush()
time.sleep(2)
server = socket.socket(socket.AF_UNIX)
server.bind(sock)
server.listen(8)
sys.stderr.write("fake vellumd: ready\n")
sys.stderr.flush()
server.settimeout(10)
try:
    while True:
        server.accept()[0].close()
except socket.timeout:
    pass
"#;

fn spawn_client(dir: &std::path::Path) -> std::process::Child {
    Command::new(dir.join("bin/vellum"))
        .arg("monitors")
        .env("XDG_RUNTIME_DIR", dir.join("run"))
        .env("WAYLAND_DISPLAY", "wayland-test")
        .env("XDG_STATE_HOME", dir.join("state"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn two_commands_autostarting_at_once_share_one_slow_daemon_and_its_log() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("bin")).unwrap();
    fs::create_dir_all(dir.path().join("run")).unwrap();
    fs::set_permissions(dir.path().join("run"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_vellum"), dir.path().join("bin/vellum")).unwrap();
    let fake = dir.path().join("bin/vellumd");
    fs::write(&fake, FAKE_VELLUMD).unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();

    let first = spawn_client(dir.path());
    std::thread::sleep(Duration::from_millis(500));
    let second = spawn_client(dir.path());
    let first = first.wait_with_output().unwrap();
    let second = second.wait_with_output().unwrap();

    for out in [&first, &second] {
        assert!(
            !stderr(out).contains("did not start"),
            "a command failed to auto-start: {}",
            stderr(out)
        );
    }
    let log = fs::read(dir.path().join("state/vellum/vellumd.log")).unwrap();
    let log = String::from_utf8_lossy(&log);
    assert!(
        log.contains("waiting for monitors") && log.contains("ready"),
        "the running Daemon's log was clobbered: {log:?}"
    );
    assert!(!log.contains('\0'), "the log has a NUL hole: {log:?}");
}
