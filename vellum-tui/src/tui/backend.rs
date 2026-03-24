use std::{
    num::NonZeroU8,
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use common::ipc::{
    Answer, Coord, ImageRequestBuilder, ImgSend, IpcSocket, PixelFormat, Position, RequestSend,
    Transition, TransitionType,
};
use image::{DynamicImage, ImageReader};
use tokio::{
    process::{Child, Command},
    time,
};
use vellum_core::is_daemon_running;

use super::data::{
    EASING_PRESETS, Rotation, ScaleMode, TRANSITION_EFFECTS, TransitionState, apply_rotation,
    render_to_monitor_canvas,
};

const IPC_QUERY_TIMEOUT: Duration = Duration::from_millis(1800);
const IPC_APPLY_TIMEOUT: Duration = Duration::from_secs(10);
const IPC_QUERY_RETRY_INTERVAL: Duration = Duration::from_millis(120);

pub struct ApplyTarget {
    pub namespace: String,
    pub outputs: Vec<String>,
    pub dim: (u32, u32),
    pub format: PixelFormat,
}

pub fn launch_daemon_subprocess(namespace: &str) -> Result<Child> {
    let exe = std::env::current_exe().context("cannot resolve executable path")?;
    let child = Command::new(exe)
        .arg("--daemon-subprocess")
        .arg("--namespace")
        .arg(namespace)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn daemon subprocess")?;

    Ok(child)
}

pub fn daemon_alive(namespace: &str) -> bool {
    is_daemon_running(namespace).unwrap_or(false)
}

pub async fn perform_apply_request(
    file_path: PathBuf,
    transition: TransitionState,
    namespace: String,
    preferred_output: Option<String>,
    scale_mode: ScaleMode,
    rotation: Rotation,
) -> Result<String> {
    let target = query_apply_target(&namespace, preferred_output).await?;
    apply_wallpaper_request(file_path.clone(), transition, target, scale_mode, rotation).await?;
    Ok(format!("Applied {}", file_path.display()))
}

async fn query_apply_target(
    namespace: &str,
    preferred_output: Option<String>,
) -> Result<ApplyTarget> {
    let deadline = Instant::now() + IPC_QUERY_TIMEOUT;

    loop {
        let attempt_namespace = namespace.to_owned();
        let attempt_preferred = preferred_output.clone();

        let attempt = run_blocking_with_timeout(IPC_QUERY_TIMEOUT, move || {
            let (resolved_namespace, socket) = connect_daemon_socket(&attempt_namespace)?;
            RequestSend::Query
                .send(&socket)
                .map_err(anyhow::Error::new)
                .context("failed to send query request")?;

            let answer = Answer::receive(
                socket
                    .recv()
                    .map_err(anyhow::Error::new)
                    .context("failed to receive daemon query response")?,
            );

            let Answer::Info(outputs) = answer else {
                bail!("unexpected daemon response to query request");
            };

            if outputs.is_empty() {
                bail!("no output information available from daemon");
            }

            let selected = if let Some(name) = attempt_preferred {
                outputs
                    .iter()
                    .find(|output| output.name.as_ref() == name)
                    .ok_or_else(|| {
                        let available = outputs
                            .iter()
                            .map(|o| o.name.as_ref())
                            .collect::<Vec<_>>()
                            .join(", ");
                        anyhow!("selected monitor '{name}' not ready in daemon yet (available: {available})")
                    })?
            } else {
                outputs
                    .first()
                    .ok_or_else(|| anyhow!("no output information available from daemon"))?
            };

            Ok(ApplyTarget {
                namespace: resolved_namespace,
                outputs: vec![selected.name.to_string()],
                dim: selected.real_dim(),
                format: selected.pixel_format,
            })
        })
        .await;

        match attempt {
            Ok(target) => return Ok(target),
            Err(err) => {
                let timed_out = Instant::now() >= deadline;
                let waiting_on_selected = preferred_output.is_some()
                    && err.to_string().contains("selected monitor '")
                    && err.to_string().contains("not ready in daemon yet");
                if timed_out || !waiting_on_selected {
                    return Err(err);
                }
                time::sleep(IPC_QUERY_RETRY_INTERVAL).await;
            }
        }
    }
}

fn connect_daemon_socket(preferred_namespace: &str) -> Result<(String, IpcSocket)> {
    if let Ok(socket) = IpcSocket::client(preferred_namespace) {
        return Ok((preferred_namespace.to_owned(), socket));
    }

    if preferred_namespace.is_empty() {
        bail!("daemon socket unavailable in default namespace");
    }

    if let Ok(socket) = IpcSocket::client("") {
        return Ok((String::new(), socket));
    }

    if let Ok(namespaces) = IpcSocket::all_namespaces() {
        for namespace in namespaces {
            if let Ok(socket) = IpcSocket::client(&namespace) {
                return Ok((namespace, socket));
            }
        }
    }

    bail!(
        "daemon socket not found for namespace '{preferred_namespace}' and no fallback namespace was reachable"
    )
}

async fn apply_wallpaper_request(
    file_path: PathBuf,
    transition: TransitionState,
    target: ApplyTarget,
    scale_mode: ScaleMode,
    rotation: Rotation,
) -> Result<()> {
    run_blocking_with_timeout(IPC_APPLY_TIMEOUT, move || {
        let display_path = file_path.to_string_lossy().into_owned();

        let decoded = ImageReader::open(&file_path)
            .map_err(anyhow::Error::new)
            .with_context(|| format!("cannot open image '{}'", display_path))?
            .decode()
            .map_err(anyhow::Error::new)
            .with_context(|| format!("cannot decode image '{}'", display_path))?;

        let transformed = apply_rotation(decoded, rotation);
        let fitted = render_to_monitor_canvas(transformed, target.dim, scale_mode);
        let (img_bytes, pixel_format) =
            convert_dynamic_image_for_pixel_format(fitted, target.format);

        let img_send = ImgSend {
            path: file_path.to_string_lossy().to_string(),
            dim: target.dim,
            format: pixel_format,
            img: img_bytes.into_boxed_slice(),
        };

        let mut builder = ImageRequestBuilder::new(build_transition_from_state(&transition))
            .map_err(anyhow::Error::new)
            .context("request mmap failed")?;

        builder.push(
            img_send,
            &target.namespace,
            scale_mode.as_resize(),
            "lanczos3",
            &target.outputs,
            None,
        );

        let socket = IpcSocket::client(&target.namespace)
            .map_err(anyhow::Error::new)
            .context("failed to connect to daemon IPC socket")?;

        RequestSend::Img(builder.build())
            .send(&socket)
            .map_err(anyhow::Error::new)
            .context("failed to send image request")?;

        let _ = Answer::receive(
            socket
                .recv()
                .map_err(anyhow::Error::new)
                .context("failed to receive daemon apply response")?,
        );

        Ok(())
    })
    .await
}

fn convert_dynamic_image_for_pixel_format(
    image: DynamicImage,
    format: PixelFormat,
) -> (Vec<u8>, PixelFormat) {
    match format {
        PixelFormat::Bgr => {
            let rgb = image.to_rgb8();
            (rgb.into_raw(), PixelFormat::Bgr)
        }
        PixelFormat::Rgb => {
            let mut rgb = image.to_rgb8().into_raw();
            for chunk in rgb.chunks_exact_mut(3) {
                chunk.swap(0, 2);
            }
            (rgb, PixelFormat::Rgb)
        }
        PixelFormat::Abgr => {
            let rgba = image.to_rgba8();
            (rgba.into_raw(), PixelFormat::Abgr)
        }
        PixelFormat::Argb => {
            let mut rgba = image.to_rgba8().into_raw();
            for chunk in rgba.chunks_exact_mut(4) {
                chunk.swap(0, 2);
            }
            (rgba, PixelFormat::Argb)
        }
    }
}

fn build_transition_from_state(state: &TransitionState) -> Transition {
    let transition_type = match TRANSITION_EFFECTS[state.effect_idx] {
        "fade" => TransitionType::Fade,
        "wipe" => TransitionType::Wipe,
        "grow" => TransitionType::Grow,
        _ => TransitionType::Simple,
    };

    let bezier = match EASING_PRESETS[state.easing_idx] {
        "linear" => (0.0, 0.0, 1.0, 1.0),
        "ease-in" => (0.42, 0.0, 1.0, 1.0),
        "ease-out" => (0.0, 0.0, 0.58, 1.0),
        _ => (0.42, 0.0, 0.58, 1.0),
    };

    Transition {
        transition_type,
        duration: state.duration_ms as f32 / 1000.0,
        // SAFETY: literal 2 is always non-zero.
        step: unsafe { NonZeroU8::new_unchecked(2) },
        fps: state.fps,
        angle: 0.0,
        pos: Position::new(Coord::Percent(0.5), Coord::Percent(0.5)),
        bezier,
        wave: (10.0, 10.0),
        invert_y: false,
    }
}

async fn run_blocking_with_timeout<T, F>(timeout: Duration, f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    let handle = tokio::task::spawn_blocking(f);
    match time::timeout(timeout, handle).await {
        Ok(joined) => joined.map_err(anyhow::Error::new)?,
        Err(_) => bail!("operation timed out after {}ms", timeout.as_millis()),
    }
}
