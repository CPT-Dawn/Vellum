//! The Daemon's life end to end: State restore, auto-start, single instance,
//! version mismatch, systemd readiness, compositor loss, status and kill.

mod support;

use std::io::{Read, Write};
use std::os::unix::net::{UnixDatagram, UnixListener};
use std::time::Duration;

use image::{ImageFormat, Rgb, RgbImage};
use support::*;

const SMALL: Monitor = Monitor {
    width: 320,
    height: 240,
};
const TINY: Monitor = Monitor {
    width: 200,
    height: 150,
};
const BLACK: [u8; 3] = [0, 0, 0];

fn solid(sway: &Sway, color: [u8; 3], width: u32, height: u32) -> String {
    let name = format!(
        "solid-{:02x}{:02x}{:02x}-{width}x{height}.png",
        color[0], color[1], color[2]
    );
    let path = sway.dir().join(name);
    RgbImage::from_fn(width, height, |_, _| Rgb(color))
        .save_with_format(&path, ImageFormat::Png)
        .unwrap();
    path.to_string_lossy().into_owned()
}

fn center_is(sway: &Sway, monitor: &str, color: [u8; 3]) -> bool {
    let shot = sway.screenshot(monitor);
    let (w, h) = shot.dimensions();
    shot.get_pixel(w / 2, h / 2).0 == color
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn wallpapers_and_fit_modes_come_back_after_a_daemon_restart() {
    let mut sway = Sway::start(&[SMALL, TINY]);
    sway.start_daemon();
    // A small image centred leaves black around it; filled, it covers all.
    let red = solid(&sway, RED, 40, 30);
    let green = solid(&sway, GREEN, 40, 30);
    sway.vellum_ok(["set", &red, "-m", "HEADLESS-1", "--fit", "center"]);
    sway.vellum_ok(["set", &green, "-m", "HEADLESS-2"]);
    sway.wait_until("both wallpapers", || {
        center_is(&sway, "HEADLESS-1", RED) && center_is(&sway, "HEADLESS-2", GREEN)
    });

    sway.vellum_ok(["kill"]);
    assert!(sway.wait_daemon_exit().is_some_and(|s| s.success()));
    // [DEBUG-c1r5]
    {
        let start = std::time::Instant::now();
        let mut seen = Vec::new();
        loop {
            let a = sway.screenshot("HEADLESS-1");
            let b = sway.screenshot("HEADLESS-2");
            let pa = (a.get_pixel(160, 120).0, a.get_pixel(5, 5).0);
            let pb = (b.get_pixel(100, 75).0, b.get_pixel(2, 2).0);
            let entry = format!("{:?} H1 {:?} H2 {:?}", start.elapsed(), pa, pb);
            if seen.last().map(|(_, e): &(String, String)| e.split_once(" H1 ").unwrap().1.to_string()) != Some(entry.split_once(" H1 ").unwrap().1.to_string()) {
                seen.push((String::new(), entry));
            }
            if pa.0 == BLACK && pb.0 == BLACK { break; }
            if start.elapsed() > std::time::Duration::from_secs(5) {
                let out = sway.command("swaymsg").env("SWAYSOCK", std::fs::read_dir(sway.dir()).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.file_name().unwrap().to_string_lossy().starts_with("sway-ipc.")).unwrap()).args(["-t", "get_tree", "-r"]).output().unwrap();
                panic!("[DEBUG-c1r5] still not black:\n{}\n--- tree ---\n{}\n--- sway log ---\n{}\n--- vellumd log ---\n{}",
                    seen.iter().map(|(_, e)| e.as_str()).collect::<Vec<_>>().join("\n"),
                    String::from_utf8_lossy(&out.stdout).chars().take(3000).collect::<String>(),
                    std::fs::read_to_string(sway.dir().join("sway.log")).unwrap_or_default(),
                    sway.daemon_log());
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        eprintln!("[DEBUG-c1r5] went black: {:?}", seen.iter().map(|(_, e)| e.as_str()).collect::<Vec<_>>());
    }

    sway.start_daemon();
    sway.wait_until("the wallpapers to come back", || {
        center_is(&sway, "HEADLESS-1", RED) && center_is(&sway, "HEADLESS-2", GREEN)
    });
    let one = sway.screenshot("HEADLESS-1");
    assert_pixel(&one, 160, 120, RED, 0);
    assert_pixel(&one, 5, 5, BLACK, 0); // still centred, not filled
    let two = sway.screenshot("HEADLESS-2");
    assert_pixel(&two, 2, 2, GREEN, 0);
    assert!(sway.daemon_alive(), "vellumd died:\n{}", sway.daemon_log());
}

#[test]
fn a_monitor_plugged_in_later_gets_a_wallpaper() {
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon();
    sway.vellum_ok(["set", &solid(&sway, BLUE, 8, 8)]);
    sway.wait_until("HEADLESS-1 turns blue", || {
        center_is(&sway, "HEADLESS-1", BLUE)
    });

    sway.swaymsg("create_output");
    sway.swaymsg("output HEADLESS-2 mode 200x150 position 10000 0");

    sway.wait_until("HEADLESS-2 turns blue", || {
        center_is(&sway, "HEADLESS-2", BLUE)
    });
    let shot = sway.screenshot("HEADLESS-2");
    assert_eq!(shot.dimensions(), (200, 150));
    assert_pixel(&shot, 198, 148, BLUE, 0);
}

#[test]
fn a_restore_never_overwrites_a_newer_set() {
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon();
    sway.vellum_ok(["set", &solid(&sway, RED, 8, 8)]);
    sway.vellum_ok(["kill"]);
    sway.wait_daemon_exit();

    // `set` auto-starts the Daemon, which restores red at the same time.
    sway.vellum_ok(["set", &solid(&sway, GREEN, 8, 8)]);
    sway.wait_until("green", || center_is(&sway, "HEADLESS-1", GREEN));
    std::thread::sleep(Duration::from_millis(500));
    assert!(center_is(&sway, "HEADLESS-1", GREEN), "a restore won");
}

#[test]
fn any_command_auto_starts_the_daemon() {
    let sway = Sway::start(&[SMALL]);
    assert!(!sway.daemon_answers());

    sway.vellum_ok(["set", &solid(&sway, GREEN, 8, 8)]);

    sway.wait_until("green", || center_is(&sway, "HEADLESS-1", GREEN));
    assert!(sway.daemon_answers());
    let monitors = sway.vellum_stdout(["monitors"]);
    assert!(monitors.contains("HEADLESS-1"), "{monitors}");
}

#[test]
fn kill_stops_the_daemon_and_never_starts_one() {
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon();

    sway.vellum_ok(["kill"]);
    assert!(
        sway.wait_daemon_exit().is_some_and(|s| s.success()),
        "vellumd did not exit cleanly:\n{}",
        sway.daemon_log()
    );
    assert!(!sway.daemon_answers());

    let out = sway.vellum(["kill"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stderr(&out).contains("not running"), "{}", stderr(&out));
    std::thread::sleep(Duration::from_millis(300));
    assert!(!sway.daemon_answers(), "kill started a Daemon");
}

#[test]
fn a_second_daemon_on_the_same_display_refuses_to_start() {
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon();

    let out = sway.vellumd();

    assert!(!out.status.success());
    assert!(stderr(&out).contains("already running"), "{}", stderr(&out));
    assert!(sway.daemon_alive(), "vellumd died:\n{}", sway.daemon_log());
    assert!(sway.daemon_answers());
}

#[test]
fn a_daemon_from_another_release_gets_a_clear_error() {
    let sway = Sway::start(&[SMALL]);
    // A fake Daemon that speaks a future protocol version.
    let listener = UnixListener::bind(sway.socket()).unwrap();
    let future = vellum_ipc::PROTOCOL_VERSION + 1;
    let fake = std::thread::spawn(move || {
        // Skip probes that connect and send nothing; answer the request.
        let mut stream = loop {
            let (mut stream, _) = listener.accept().unwrap();
            let mut header = [0u8; vellum_ipc::HEADER_LEN];
            if stream.read_exact(&mut header).is_ok() {
                break stream;
            }
        };
        let mut reply = Vec::from(vellum_ipc::MAGIC);
        reply.extend_from_slice(&future.to_le_bytes());
        reply.extend_from_slice(&0x8000u16.to_le_bytes());
        reply.extend_from_slice(&0u32.to_le_bytes());
        stream.write_all(&reply).unwrap();
    });

    let out = sway.vellum(["monitors"]);
    fake.join().unwrap();

    assert!(!out.status.success());
    let text = stderr(&out);
    assert!(text.contains(&format!("v{future}")), "{text}");
    assert!(
        text.contains(&format!("v{}", vellum_ipc::PROTOCOL_VERSION)),
        "{text}"
    );
    assert!(text.contains("restart"), "{text}");
}

#[test]
fn a_client_stopped_and_continued_mid_request_still_gets_its_answer() {
    // A socket with a read timeout returns EINTR after a stop and continue
    // (job control, a cgroup freeze, a loaded CI box), handler or not.
    let sway = Sway::start(&[SMALL]);
    let listener = UnixListener::bind(sway.socket()).unwrap();
    let (got_request, request_seen) = std::sync::mpsc::channel();
    let (may_answer, answer) = std::sync::mpsc::channel::<()>();
    let future = vellum_ipc::PROTOCOL_VERSION + 1;
    let fake = std::thread::spawn(move || {
        let mut stream = loop {
            let (mut stream, _) = listener.accept().unwrap();
            let mut header = [0u8; vellum_ipc::HEADER_LEN];
            if stream.read_exact(&mut header).is_ok() {
                break stream;
            }
        };
        got_request.send(()).unwrap();
        answer.recv().unwrap();
        // Any well-formed answer will do; a version mismatch is the simplest.
        let mut reply = Vec::from(vellum_ipc::MAGIC);
        reply.extend_from_slice(&future.to_le_bytes());
        reply.extend_from_slice(&0x8000u16.to_le_bytes());
        reply.extend_from_slice(&0u32.to_le_bytes());
        // A client that gave up has closed its end; its stderr says why.
        let _ = stream.write_all(&reply);
    });

    let child = sway
        .command(env!("CARGO_BIN_EXE_vellum"))
        .arg("monitors")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    request_seen.recv().unwrap();
    // Let vellum block in recvmsg, then stop and continue it there.
    std::thread::sleep(Duration::from_millis(200));
    let pid = rustix::process::Pid::from_child(&child);
    rustix::process::kill_process(pid, rustix::process::Signal::STOP).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    rustix::process::kill_process(pid, rustix::process::Signal::CONT).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    may_answer.send(()).unwrap();
    let out = child.wait_with_output().unwrap();
    fake.join().unwrap();

    let text = stderr(&out);
    assert!(!text.contains("Interrupted"), "{text}");
    assert!(text.contains(&format!("v{future}")), "{text}");
}

#[test]
fn the_daemon_answers_an_older_client_in_its_own_version_and_survives() {
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon();
    let mut frame = vellum_ipc::Request::Monitors.encode();
    frame[4..6].copy_from_slice(&(vellum_ipc::PROTOCOL_VERSION - 1).to_le_bytes());

    let mut stream = std::os::unix::net::UnixStream::connect(sway.socket()).unwrap();
    stream.write_all(&frame).unwrap();
    let mut header = [0u8; vellum_ipc::HEADER_LEN];
    stream.read_exact(&mut header).unwrap();

    assert_eq!(&header[0..4], b"VLUM");
    assert_eq!(
        u16::from_le_bytes([header[4], header[5]]),
        vellum_ipc::PROTOCOL_VERSION
    );
    assert!(sway.daemon_alive(), "vellumd died:\n{}", sway.daemon_log());
    sway.vellum_ok(["monitors"]);
}

#[test]
fn the_daemon_tells_systemd_it_is_ready() {
    let mut sway = Sway::start(&[SMALL]);
    let notify = sway.dir().join("notify.sock");
    let systemd = UnixDatagram::bind(&notify).unwrap();
    systemd
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();

    sway.start_daemon_with(&[("NOTIFY_SOCKET", notify.to_str().unwrap())]);

    let mut buf = [0u8; 256];
    let n = systemd.recv(&mut buf).expect("no sd_notify message");
    let message = String::from_utf8_lossy(&buf[..n]);
    assert!(message.lines().any(|l| l == "READY=1"), "{message}");
    // Readiness means the socket answers.
    sway.vellum_ok(["monitors"]);
}

#[test]
fn the_daemon_exits_cleanly_when_the_compositor_goes_away() {
    let mut sway = Sway::start(&[SMALL]);
    sway.start_daemon();

    sway.kill_compositor();

    let status = sway.wait_daemon_exit();
    assert!(
        status.is_some_and(|s| s.success()),
        "vellumd did not exit cleanly ({status:?}):\n{}",
        sway.daemon_log()
    );
    assert!(!sway.socket().exists(), "vellumd left its socket behind");
}

#[test]
fn status_shows_monitors_wallpapers_and_memory() {
    let mut sway = Sway::start(&[SMALL, TINY]);
    sway.start_daemon();
    let red = solid(&sway, RED, 8, 8);
    sway.vellum_ok(["set", &red, "-m", "HEADLESS-1", "--fit", "stretch"]);

    let out = sway.vellum_stdout(["status"]);

    let line = |name: &str| {
        out.lines()
            .find(|l| l.starts_with(name))
            .unwrap_or_else(|| panic!("no {name} in:\n{out}"))
            .to_owned()
    };
    let one = line("HEADLESS-1");
    assert!(one.contains(&red), "{out}");
    assert!(one.contains("stretch"), "{out}");
    assert!(line("HEADLESS-2").contains('-'), "{out}");
    assert!(out.contains("vellumd"), "{out}");
    assert!(out.contains("MB") || out.contains("KB"), "{out}");
}
