use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use vellum_ipc::{Request, RequestEnvelope, Response, ResponseEnvelope, ScaleMode};

#[test]
fn daemon_ipc_flow_and_clear_cycle() {
    let test_dir = new_test_dir("vellum-ipc-flow");
    let socket_path = test_dir.join("daemon.sock");
    let state_path = test_dir.join("state.json");
    let image_path = test_dir.join("pixel.png");
    write_test_png(&image_path);

    let mut daemon = spawn_daemon(&socket_path, &state_path);
    wait_for_socket(&socket_path);

    let ping = send_request(&socket_path, Request::Ping);
    assert!(matches!(ping, Response::Pong));

    let set = send_request(
        &socket_path,
        Request::SetWallpaper {
            path: image_path.display().to_string(),
            monitor: None,
            mode: ScaleMode::Fill,
        },
    );
    assert!(matches!(set, Response::Ok));

    let assignments = send_request(&socket_path, Request::GetAssignments);
    match assignments {
        Response::Assignments { entries } => {
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].mode, ScaleMode::Fill);
        }
        other => panic!("expected assignments response, got {other:?}"),
    }

    let clear = send_request(&socket_path, Request::ClearAssignments);
    assert!(matches!(clear, Response::Ok));

    let assignments = send_request(&socket_path, Request::GetAssignments);
    match assignments {
        Response::Assignments { entries } => {
            assert!(entries.is_empty());
        }
        other => panic!("expected assignments response, got {other:?}"),
    }

    let kill = send_request(&socket_path, Request::KillDaemon);
    assert!(matches!(kill, Response::Ok));
    wait_for_exit(&mut daemon);

    std::fs::remove_dir_all(test_dir).expect("test directory should be removed");
}

#[test]
fn daemon_persists_assignments_across_restart() {
    let test_dir = new_test_dir("vellum-ipc-persist");
    let socket_path = test_dir.join("daemon.sock");
    let state_path = test_dir.join("state.json");
    let image_path = test_dir.join("pixel.png");
    write_test_png(&image_path);

    {
        let mut daemon = spawn_daemon(&socket_path, &state_path);
        wait_for_socket(&socket_path);

        let set = send_request(
            &socket_path,
            Request::SetWallpaper {
                path: image_path.display().to_string(),
                monitor: None,
                mode: ScaleMode::Crop,
            },
        );
        assert!(matches!(set, Response::Ok));

        let kill = send_request(&socket_path, Request::KillDaemon);
        assert!(matches!(kill, Response::Ok));
        wait_for_exit(&mut daemon);
    }

    {
        let mut daemon = spawn_daemon(&socket_path, &state_path);
        wait_for_socket(&socket_path);

        let assignments = send_request(&socket_path, Request::GetAssignments);
        match assignments {
            Response::Assignments { entries } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].mode, ScaleMode::Crop);
            }
            other => panic!("expected assignments response, got {other:?}"),
        }

        let kill = send_request(&socket_path, Request::KillDaemon);
        assert!(matches!(kill, Response::Ok));
        wait_for_exit(&mut daemon);
    }

    std::fs::remove_dir_all(test_dir).expect("test directory should be removed");
}

fn spawn_daemon(socket_path: &Path, state_path: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_vellumd"))
        .arg("--socket")
        .arg(socket_path)
        .arg("--state-file")
        .arg(state_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn daemon")
}

fn wait_for_socket(socket_path: &Path) {
    for _ in 0..50 {
        if socket_path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }

    panic!("daemon socket did not appear at {}", socket_path.display());
}

fn wait_for_exit(child: &mut Child) {
    for _ in 0..100 {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }

    let _ = child.kill();
    let _ = child.wait();
    panic!("daemon did not exit within timeout");
}

fn send_request(socket_path: &Path, request: Request) -> Response {
    let mut stream = UnixStream::connect(socket_path).expect("failed to connect to daemon socket");

    let payload =
        serde_json::to_string(&RequestEnvelope::new(request)).expect("failed to serialize request");
    stream
        .write_all(payload.as_bytes())
        .expect("failed to write request payload");
    stream
        .write_all(b"\n")
        .expect("failed to write request newline");
    stream.flush().expect("failed to flush request");

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .expect("failed to read response line");

    let envelope: ResponseEnvelope =
        serde_json::from_str(line.trim()).expect("failed to deserialize response envelope");
    envelope
        .validate_version()
        .expect("response version should be valid");
    envelope.response
}

fn new_test_dir(prefix: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("{prefix}-{nonce}"));
    std::fs::create_dir_all(&path).expect("test dir should be created");
    path
}

fn write_test_png(path: &Path) {
    let image = image::RgbImage::from_pixel(1, 1, image::Rgb([42, 84, 126]));
    image
        .save(path)
        .expect("png fixture should be generated and written");
}
