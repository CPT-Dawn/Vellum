//! The encode/decode boundary of the Vellum wire protocol.

use std::io::{Read, Seek, Write};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;

use vellum_ipc::{
    Error, HEADER_LEN, MonitorInfo, PROTOCOL_VERSION, PixelFormat, Present, Request, Response,
    recv, send,
};

fn present() -> Present {
    Present {
        monitor: "HEADLESS-1".into(),
        width: 1920,
        height: 1080,
        stride: 1920 * 4,
        format: PixelFormat::Xrgb8888,
        source: "/home/me/Pictures/Wallpapers/dunes.jpg".into(),
    }
}

#[test]
fn requests_survive_a_round_trip() {
    for request in [Request::Monitors, Request::Present(present())] {
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
