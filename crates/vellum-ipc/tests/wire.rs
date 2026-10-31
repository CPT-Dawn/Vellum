//! The encode/decode boundary of the Vellum wire protocol.

use std::io::{Read, Seek, Write};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;

use vellum_ipc::{
    Direction, Easing, Error, FitMode, HEADER_LEN, MonitorInfo, MonitorStatus, Override,
    PROTOCOL_VERSION, PixelFormat, Present, Request, Response, RotationOrder, RotationSettings,
    RotationSource, Schedule, Settings, Status, Transition, TransitionKind, Wallpaper, recv, send,
};

fn settings() -> Settings {
    Settings {
        transition: Transition {
            kind: TransitionKind::Wipe,
            direction: Direction::Up,
            duration_ms: 250,
            easing: Easing::Linear,
        },
        rotation: RotationSettings {
            schedule: Schedule {
                interval_ms: 90_000,
                source: RotationSource::Favorites,
                order: RotationOrder::Sequential,
            },
            sync: true,
            overrides: vec![Override {
                monitor: "DP-2".into(),
                schedule: Schedule {
                    interval_ms: 3_600_000,
                    source: RotationSource::Library,
                    order: RotationOrder::Shuffle,
                },
            }],
        },
        letterbox: [0x12, 0x34, 0x56],
    }
}

fn present() -> Present {
    Present {
        monitor: "HEADLESS-1".into(),
        width: 1920,
        height: 1080,
        stride: 1920 * 4,
        format: PixelFormat::Xrgb8888,
        source: "/home/me/Pictures/Wallpapers/dunes.jpg".into(),
        transition: Transition::default(),
        fit: FitMode::Center,
        restore: true,
    }
}

#[test]
fn the_default_transition_is_a_600ms_ease_out_fade() {
    let t = Transition::default();
    assert_eq!(t.kind, TransitionKind::Fade);
    assert_eq!(t.duration_ms, 600);
    assert_eq!(t.easing, Easing::EaseOut);
}

#[test]
fn easing_curves_start_at_zero_end_at_one_and_bend_as_named() {
    for easing in [
        Easing::Linear,
        Easing::EaseIn,
        Easing::EaseOut,
        Easing::EaseInOut,
    ] {
        assert_eq!(easing.apply(0.0), 0.0, "{easing:?}");
        assert_eq!(easing.apply(1.0), 1.0, "{easing:?}");
        assert_eq!(easing.apply(7.0), 1.0, "{easing:?} clamps");
    }
    assert_eq!(Easing::Linear.apply(0.5), 0.5);
    assert_eq!(Easing::EaseIn.apply(0.5), 0.125);
    assert_eq!(Easing::EaseOut.apply(0.5), 0.875);
    assert_eq!(Easing::EaseInOut.apply(0.5), 0.5);
    assert_eq!(Easing::EaseInOut.apply(0.25), 0.0625);
}

#[test]
fn requests_survive_a_round_trip() {
    let mut requests = vec![
        Request::Monitors,
        Request::Present(present()),
        Request::Status,
        Request::Kill,
        Request::Settings(settings()),
        Request::Settings(Settings::default()),
        Request::RotationStart,
        Request::RotationStop,
        Request::RotationReset("DP-1".into()),
    ];
    for kind in [
        TransitionKind::None,
        TransitionKind::Fade,
        TransitionKind::Wipe,
        TransitionKind::Slide,
    ] {
        for (direction, easing) in [
            (Direction::Left, Easing::Linear),
            (Direction::Right, Easing::EaseIn),
            (Direction::Up, Easing::EaseOut),
            (Direction::Down, Easing::EaseInOut),
        ] {
            let mut p = present();
            p.transition = Transition {
                kind,
                direction,
                duration_ms: 1234,
                easing,
            };
            requests.push(Request::Present(p));
        }
    }
    for request in requests {
        let frame = request.encode();
        assert_eq!(Request::decode(&frame).unwrap(), request);
    }
}

#[test]
fn responses_survive_a_round_trip() {
    let monitors = vec![
        MonitorInfo {
            name: "DP-1".into(),
            logical_width: 2560,
            logical_height: 1440,
            physical_width: 2560,
            physical_height: 1440,
            scale_120: 120,
        },
        MonitorInfo {
            name: "eDP-1".into(),
            logical_width: 1280,
            logical_height: 800,
            physical_width: 2560,
            physical_height: 1600,
            scale_120: 240,
        },
    ];
    for response in [
        Response::Ok,
        Response::Error("monitor DP-9 is not connected".into()),
        Response::Monitors(monitors),
        Response::Status(Status {
            version: "2.0.0".into(),
            pid: 4242,
            rss_bytes: 3 << 20,
            settings: settings(),
            rotation_running: true,
            monitors: vec![
                MonitorStatus {
                    name: "DP-1".into(),
                    wallpaper: Some(Wallpaper {
                        path: "/w/dunes.jpg".into(),
                        fit: FitMode::Fit,
                    }),
                    next_change_ms: Some(61_000),
                },
                MonitorStatus {
                    name: "eDP-1".into(),
                    wallpaper: None,
                    next_change_ms: None,
                },
            ],
        }),
    ] {
        let frame = response.encode();
        assert_eq!(Response::decode(&frame).unwrap(), response);
    }
}

#[test]
fn frames_start_with_magic_and_version_and_are_length_prefixed() {
    let frame = Request::Monitors.encode();
    assert_eq!(&frame[0..4], b"VLUM");
    assert_eq!(u16::from_le_bytes([frame[4], frame[5]]), PROTOCOL_VERSION);
    let len = u32::from_le_bytes(frame[8..12].try_into().unwrap()) as usize;
    assert_eq!(frame.len(), HEADER_LEN + len);
}

#[test]
fn a_frame_from_another_protocol_version_names_both_versions() {
    let mut frame = Request::Monitors.encode();
    frame[4..6].copy_from_slice(&(PROTOCOL_VERSION + 1).to_le_bytes());
    let err = Request::decode(&frame).unwrap_err();
    assert!(matches!(
        err,
        Error::VersionMismatch { ours, theirs } if ours == PROTOCOL_VERSION && theirs == PROTOCOL_VERSION + 1
    ));
    let text = err.to_string();
    assert!(text.contains(&format!("v{PROTOCOL_VERSION}")), "{text}");
    assert!(
        text.contains(&format!("v{}", PROTOCOL_VERSION + 1)),
        "{text}"
    );
}

#[test]
fn garbage_is_rejected_not_trusted() {
    assert!(matches!(
        Request::decode(b"HTTP/1.1 200 OK\r\n"),
        Err(Error::BadMagic)
    ));
    let frame = Request::Present(present()).encode();
    assert!(matches!(
        Request::decode(&frame[..frame.len() - 3]),
        Err(Error::Truncated)
    ));
    let mut unknown = Request::Monitors.encode();
    unknown[6..8].copy_from_slice(&0xBEEFu16.to_le_bytes());
    assert!(matches!(
        Request::decode(&unknown),
        Err(Error::UnknownKind(0xBEEF))
    ));
}

#[test]
fn a_present_request_carries_its_frame_as_a_file_descriptor() {
    let (client, daemon) = UnixStream::pair().unwrap();
    let mut pixels = tempfile_with(b"\x10\x20\x30\xff");

    send(&client, &Request::Present(present()), &[pixels.as_fd()]).unwrap();
    let (request, fds): (Request, _) = recv(&daemon).unwrap();

    assert_eq!(request, Request::Present(present()));
    assert_eq!(fds.len(), 1);
    let mut received = std::fs::File::from(fds.into_iter().next().unwrap());
    let mut bytes = Vec::new();
    received.rewind().unwrap();
    received.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"\x10\x20\x30\xff");
    pixels.rewind().unwrap();
}

#[test]
fn a_peer_hanging_up_mid_frame_is_an_error() {
    let (client, daemon) = UnixStream::pair().unwrap();
    let frame = Request::Present(present()).encode();
    (&client).write_all(&frame[..HEADER_LEN + 2]).unwrap();
    drop(client);
    assert!(matches!(recv::<Request>(&daemon), Err(Error::Closed)));
}

fn tempfile_with(bytes: &[u8]) -> std::fs::File {
    let fd = rustix::fs::memfd_create("test", rustix::fs::MemfdFlags::CLOEXEC).unwrap();
    let mut file = std::fs::File::from(fd);
    file.write_all(bytes).unwrap();
    file
}

#[test]
fn fit_modes_have_stable_names() {
    for fit in [
        FitMode::Fill,
        FitMode::Fit,
        FitMode::Center,
        FitMode::Stretch,
    ] {
        assert_eq!(FitMode::from_name(fit.name()), Some(fit));
    }
    assert_eq!(FitMode::Stretch.name(), "stretch");
    assert_eq!(FitMode::from_name("tile"), None);
}

#[test]
fn rotation_defaults_to_a_30_minute_library_shuffle_without_sync() {
    let r = RotationSettings::default();
    assert_eq!(r.schedule.interval_ms, 30 * 60 * 1000);
    assert_eq!(r.schedule.source, RotationSource::Library);
    assert_eq!(r.schedule.order, RotationOrder::Shuffle);
    assert!(!r.sync);
    assert!(r.overrides.is_empty());
}

#[test]
fn an_override_replaces_the_global_rotation_for_its_monitor_only() {
    let r = settings().rotation;
    assert_eq!(r.schedule_for("DP-2").interval_ms, 3_600_000);
    assert_eq!(r.schedule_for("DP-1").interval_ms, 90_000);
    // `sync` groups the Monitors that follow the global Rotation.
    assert!(r.in_sync_group("DP-1"));
    assert!(!r.in_sync_group("DP-2"));
    assert!(!RotationSettings::default().in_sync_group("DP-1"));
}
