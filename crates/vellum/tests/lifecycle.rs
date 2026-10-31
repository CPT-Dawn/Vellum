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
    sway.wait_until("the wallpapers to go away", || {
        center_is(&sway, "HEADLESS-1", BLACK) && center_is(&sway, "HEADLESS-2", BLACK)
    });

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
        let (mut stream, _) = listener.accept().unwrap();
        let mut header = [0u8; vellum_ipc::HEADER_LEN];
        stream.read_exact(&mut header).unwrap();
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

    sway.start_daemon_with_env(&[("NOTIFY_SOCKET", notify.as_os_str())]);

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
