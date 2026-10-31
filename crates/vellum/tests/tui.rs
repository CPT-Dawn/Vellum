//! The TUI end to end: `vellum` with no arguments in a pseudo-terminal
//! (util-linux `script`), driving a real Daemon on headless Sway.

mod support;

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use image::{Rgb, RgbImage};
use support::*;

/// `vellum` with no arguments in an 80x24 pseudo-terminal that answers the
/// graphics probe like a plain xterm would. Returns the process and its
/// keyboard.
fn open_tui(sway: &Sway) -> (Child, Arc<Mutex<ChildStdin>>) {
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
    (tui, keys)
}

fn press(keys: &Mutex<ChildStdin>, bytes: &[u8]) {
    let mut keys = keys.lock().unwrap();
    keys.write_all(bytes).unwrap();
    keys.flush().unwrap();
}

fn wait_exit(tui: &mut Child) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(status) = tui.try_wait().unwrap() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let _ = tui.kill();
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
