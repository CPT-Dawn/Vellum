//! The TUI end to end: `vellum` with no arguments in a pseudo-terminal
//! (util-linux `script`), driving a real Daemon on headless Sway.

mod support;

use std::io::{Read, Write};
use std::os::unix::process::CommandExt as _;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use image::{Rgb, RgbImage};
use support::*;

/// `script` running the TUI in its pty. Dropping it (a passing test, a
/// failed assertion, a timeout) kills `script` and everything in the pty:
/// `vellum` runs in its own session there, so it survives `script` alone.
struct Tui(Child);

impl Tui {
    fn id(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        use rustix::process::{Pid, Signal, kill_process_group};
        if matches!(self.0.try_wait(), Ok(Some(_))) {
            // Exited cleanly, and its pid may be reused already.
            return;
        }
        // Each session in the pty leads its own process group.
        for pid in children(self.0.id()) {
            if let Some(pid) = Pid::from_raw(pid as i32) {
                let _ = kill_process_group(pid, Signal::KILL);
            }
        }
        if let Some(group) = Pid::from_raw(self.0.id() as i32) {
            let _ = kill_process_group(group, Signal::KILL);
        }
        let _ = self.0.wait();
    }
}

/// `vellum` with no arguments in an 80x24 pseudo-terminal that answers the
/// graphics probe like a plain xterm would. Returns the process and its
/// keyboard.
fn open_tui(sway: &Sway) -> (Tui, Arc<Mutex<ChildStdin>>) {
    let shell = format!(
        "stty cols 80 rows 24; exec '{}'",
        env!("CARGO_BIN_EXE_vellum")
    );
    let mut tui = sway
        .command("script")
        .args(["-qfec", &shell, "/dev/null"])
        .env("TERM", "xterm-256color")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .expect("failed to run script; is util-linux installed?");
    let keys = Arc::new(Mutex::new(tui.stdin.take().unwrap()));
    // Read the screen (so the TUI never blocks on a full pipe) and answer
    // the probe: cell size, primary device attributes, status report.
    let mut screen = tui.stdout.take().unwrap();
    let terminal = Arc::clone(&keys);
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = screen.read(&mut buf) {
            if n == 0 {
                break;
            }
            let chunk = &buf[..n];
            if chunk.windows(4).any(|w| w == b"\x1b[5n") {
                let mut t = terminal.lock().unwrap();
                let _ = t.write_all(b"\x1b[6;20;10t\x1b[?62;22c\x1b[0n");
                let _ = t.flush();
            }
            if std::env::var_os("VELLUM_TUI_DUMP").is_some() {
                eprintln!("{:?}", String::from_utf8_lossy(chunk));
            }
        }
    });
    (Tui(tui), keys)
}

fn press(keys: &Mutex<ChildStdin>, bytes: &[u8]) {
    let mut keys = keys.lock().unwrap();
    keys.write_all(bytes).unwrap();
    keys.flush().unwrap();
}

/// Waits for the TUI to exit; `None` after 5 s (the guard then kills it).
fn wait_exit(tui: &mut Tui) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(status) = tui.0.try_wait().unwrap() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    None
}

#[test]
fn enter_puts_the_selected_wallpaper_on_the_monitor_and_q_quits() {
    let mut sway = Sway::start(&[Monitor {
        width: 320,
        height: 240,
    }]);
    sway.start_daemon();
    let library = sway.dir().join("walls");
    std::fs::create_dir_all(&library).unwrap();
    RgbImage::from_pixel(320, 240, Rgb(RED))
        .save(library.join("red.png"))
        .unwrap();
    sway.write_config(&format!(
        "library = [\"{}\"]\n[transition]\ntype = \"none\"\n",
        library.display()
    ));

    let (mut tui, keys) = open_tui(&sway);
    // Enter only applies once the Library and the Daemon have answered, so
    // keep pressing it until the wallpaper shows up.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        press(&keys, b"\r");
        if sway.screenshot("HEADLESS-1").get_pixel(160, 120).0 == RED {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the TUI never applied the wallpaper; daemon log:\n{}",
            sway.daemon_log()
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    let status = sway.vellum_stdout(["status"]);
    assert!(status.contains("red.png"), "{status}");

    // The preview went into the disk cache, under XDG_CACHE_HOME.
    let thumbs = sway.dir().join("cache/vellum/thumbs");
    let cached = || {
        std::fs::read_dir(&thumbs).is_ok_and(|dir| {
            dir.filter_map(Result::ok)
                .any(|e| !e.file_name().to_string_lossy().starts_with('.'))
        })
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cached() {
        assert!(Instant::now() < deadline, "no preview in {thumbs:?}");
        std::thread::sleep(Duration::from_millis(50));
    }

    press(&keys, b"q");
    let exit = wait_exit(&mut tui).expect("q did not quit the TUI");
    assert!(exit.success(), "{exit:?}");
}

/// Polls `what` for up to 5 s.
fn eventually(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_first_run_saves_the_library_and_the_rotation_panel_drives_the_daemon() {
    let mut sway = Sway::start(&[Monitor {
        width: 320,
        height: 240,
    }]);
    sway.start_daemon();
    // No Config and no ~/Pictures/Wallpapers, but a ~/Pictures.
    let pictures = sway.dir().join("Pictures");
    std::fs::create_dir_all(&pictures).unwrap();
    RgbImage::from_pixel(32, 24, Rgb(RED))
        .save(pictures.join("red.png"))
        .unwrap();
    let config = sway.config_path();

    let (mut tui, keys) = open_tui(&sway);
    // The welcome screen offers ~/Pictures first; Enter takes it. Keys
    // typed during the graphics probe can be lost, so press until saved.
    let deadline = Instant::now() + Duration::from_secs(15);
    while !config.exists() {
        press(&keys, b"\r");
        assert!(Instant::now() < deadline, "the welcome screen never saved");
        std::thread::sleep(Duration::from_millis(200));
    }
    let text = std::fs::read_to_string(&config).unwrap();
    assert_eq!(text, "library = [\"~/Pictures\"]\n");

    // s opens the Rotation panel; Enter on its first line starts Rotation.
    press(&keys, b"s");
    std::thread::sleep(Duration::from_millis(100));
    press(&keys, b"\r");
    eventually("Rotation to start", || {
        sway.vellum_stdout(["status"]).contains("running")
    });
    // Down to the interval (past the scope line), one step up: 1h, saved
    // to the Config and held by the Daemon.
    press(&keys, b"jjl");
    eventually("the interval in the Config", || {
        std::fs::read_to_string(&config).is_ok_and(|t| t.contains("interval = \"1h\""))
    });
    eventually("the Daemon to hold the interval", || {
        sway.vellum_stdout(["status"]).contains("every 1h")
    });
    let text = std::fs::read_to_string(&config).unwrap();
    assert!(text.starts_with("library = [\"~/Pictures\"]\n"), "{text}");

    // q closes the panel, then quits.
    press(&keys, b"q");
    std::thread::sleep(Duration::from_millis(100));
    press(&keys, b"q");
    let exit = wait_exit(&mut tui).expect("q did not quit the TUI");
    assert!(exit.success(), "{exit:?}");
}

#[test]
fn without_a_terminal_vellum_says_so() {
    let sway = Sway::start(&[Monitor {
        width: 320,
        height: 240,
    }]);
    let out = sway
        .command(env!("CARGO_BIN_EXE_vellum"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("needs a terminal"), "{err}");
}

/// The direct children of `pid`.
fn children(pid: u32) -> Vec<u32> {
    std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|p| p.parse().ok())
        .collect()
}

/// Whether `pid` exists and is not a zombie.
fn running(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        let state = stat.rsplit(')').next().unwrap_or("").trim_start();
        !state.starts_with('Z')
    })
}

#[test]
fn a_test_that_gives_up_on_the_tui_leaves_no_process_behind() {
    let sway = Sway::start(&[Monitor {
        width: 320,
        height: 240,
    }]);
    let (tui, _keys) = open_tui(&sway);
    let script = tui.id();
    let mut inside = Vec::new();
    eventually("vellum to start inside the pty", || {
        inside = children(script);
        !inside.is_empty()
    });
    // A failing assertion unwinds through here.
    drop(tui);
    eventually("every TUI process to exit", || {
        !running(script) && !inside.iter().any(|&pid| running(pid))
    });
}
