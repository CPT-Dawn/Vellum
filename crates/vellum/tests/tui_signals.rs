//! Signals that end the TUI must still give the terminal back: mouse
//! capture off, alternate screen left, cursor shown. No compositor needed.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process};

/// Runs the TUI in a pty (util-linux `script`) in a sandbox, sends it
/// `signal` once it is up, and returns everything the terminal saw.
fn terminal_after(signal: Signal) -> String {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let shell = format!(
        "stty cols 80 rows 24; echo PID=$$; exec '{}'",
        env!("CARGO_BIN_EXE_vellum")
    );
    let mut script = Command::new("script")
        .args(["-qfec", &shell, "/dev/null"])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("TERM", "xterm-256color")
        .env("HOME", d)
        .env("XDG_RUNTIME_DIR", d)
        .env("XDG_CONFIG_HOME", d.join("config"))
        .env("XDG_STATE_HOME", d.join("state"))
        .env("XDG_CACHE_HOME", d.join("cache"))
        .env("XDG_DATA_HOME", d.join("data"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to run script; is util-linux installed?");
    let mut out = script.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = out.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut seen = Vec::new();
    let mut pid = None;
    let mut sent = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => seen.extend(chunk),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(_) => {}
        }
        let text = String::from_utf8_lossy(&seen).into_owned();
        if pid.is_none() {
            pid = text
                .split("PID=")
                .nth(1)
                .and_then(|s| s.split_whitespace().next()?.parse::<i32>().ok());
        }
        // The alternate screen is entered once the TUI is up.
        if !sent && text.contains("\x1b[?1049h") {
            std::thread::sleep(Duration::from_millis(500));
            kill_process(Pid::from_raw(pid.expect("no pid")).unwrap(), signal).unwrap();
            sent = true;
        }
    }
    if let Some(pid) = pid.and_then(Pid::from_raw) {
        let _ = kill_process(pid, Signal::KILL);
    }
    let _ = script.kill();
    let _ = script.wait();
    while let Ok(chunk) = rx.try_recv() {
        seen.extend(chunk);
    }
    assert!(sent, "the TUI never came up");
    String::from_utf8_lossy(&seen).into_owned()
}

fn assert_restored(text: &str) {
    let after = text.rsplit("\x1b[?1049h").next().unwrap();
    for (what, seq) in [
        ("mouse capture off", "\x1b[?1000l"),
        ("alternate screen left", "\x1b[?1049l"),
        ("cursor shown", "\x1b[?25h"),
    ] {
        assert!(after.contains(seq), "{what} ({seq:?}) missing in {after:?}");
    }
}

#[test]
fn sigterm_gives_the_terminal_back() {
    assert_restored(&terminal_after(Signal::TERM));
}

#[test]
fn sighup_gives_the_terminal_back() {
    assert_restored(&terminal_after(Signal::HUP));
}
