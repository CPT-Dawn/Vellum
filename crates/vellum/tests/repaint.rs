//! A letterbox Repaint is slow (`--sync` decodes the wallpaper first), so it
//! can reach the Daemon after the Monitor moved on. The Daemon must drop it
//! rather than put the old wallpaper back.

mod support;

use image::RgbImage;
use rustix::fs::{MemfdFlags, SealFlags};
use support::*;
use vellum_ipc::{
    Direction, Easing, FitMode, Origin, PixelFormat, Present, Request, Response, Transition,
    TransitionKind,
};

const SIZE: Monitor = Monitor {
    width: 320,
    height: 240,
};

fn solid(sway: &Sway, name: &str, color: [u8; 3]) -> String {
    let path = sway.dir().join(name);
    RgbImage::from_pixel(64, 16, image::Rgb(color))
        .save(&path)
        .unwrap();
    path.to_string_lossy().into_owned()
}

/// Sends a frame by hand, as a `--sync` that decoded `source` long ago.
fn present(sway: &Sway, source: &str, origin: Origin) -> Response {
    let (w, h) = (SIZE.width, SIZE.height);
    let fd =
        rustix::fs::memfd_create("frame", MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING).unwrap();
    rustix::fs::ftruncate(&fd, u64::from(w * h * 4)).unwrap();
    rustix::fs::fcntl_add_seals(&fd, SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL)
        .unwrap();
    let request = Request::Present(Present {
        monitor: "HEADLESS-1".into(),
        width: w,
        height: h,
        stride: w * 4,
        format: PixelFormat::Xrgb8888,
        source: source.into(),
        transition: Transition {
            kind: TransitionKind::None,
            direction: Direction::Right,
            duration_ms: 0,
            easing: Easing::Linear,
        },
        fit: FitMode::Fit,
        origin,
    });
    vellum_ipc::request(&sway.socket(), &request, &[std::os::fd::AsFd::as_fd(&fd)]).unwrap()
}

#[test]
fn a_late_repaint_of_the_old_wallpaper_is_dropped() {
    let mut sway = Sway::start(&[SIZE]);
    sway.start_daemon();
    let a = solid(&sway, "a.png", RED);
    let b = solid(&sway, "b.png", GREEN);
    sway.vellum_ok(["set", &a, "--fit", "fit"]);
    sway.vellum_ok(["set", &b, "--fit", "fit"]);

    // The Repaint was rendered from `a` before `b` was applied.
    assert_eq!(present(&sway, &a, Origin::Repaint), Response::Dropped);

    let status = sway.vellum_stdout(["status"]);
    assert!(status.contains(&b) && !status.contains(&a), "{status}");
    let shot = sway.screenshot_until("HEADLESS-1", |s| s.get_pixel(160, 120).0 == GREEN);
    assert_pixel(&shot, 160, 120, GREEN, 0);
}

#[test]
fn a_repaint_of_the_wallpaper_on_screen_still_shows() {
    let mut sway = Sway::start(&[SIZE]);
    sway.start_daemon();
    let a = solid(&sway, "a.png", RED);
    sway.vellum_ok(["set", &a, "--fit", "fit"]);

    assert_eq!(present(&sway, &a, Origin::Repaint), Response::Ok);
}
