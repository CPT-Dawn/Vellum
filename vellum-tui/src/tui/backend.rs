use std::{
    collections::{BTreeMap, HashMap},
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

use super::model::{
    Rotation, ScaleMode, TransitionState, apply_rotation, render_to_monitor_canvas,
};

const IPC_QUERY_TIMEOUT: Duration = Duration::from_millis(1800);
const IPC_APPLY_TIMEOUT: Duration = Duration::from_secs(10);
const IPC_QUERY_RETRY_INTERVAL: Duration = Duration::from_millis(120);

#[derive(Debug, Clone)]
struct ApplyGroup {
    outputs: Vec<String>,
    dim: (u32, u32),
    format: PixelFormat,
}

#[derive(Debug, Clone)]
struct ApplyPlan {
    namespace: String,
    groups: Vec<ApplyGroup>,
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
    selected_outputs: Vec<String>,
    scale_mode: ScaleMode,
    rotation: Rotation,
) -> Result<String> {
    let plan = query_apply_plan(&namespace, &selected_outputs).await?;
    apply_wallpaper_request(file_path.clone(), transition, plan, scale_mode, rotation).await?;
    Ok(format!("Applied {}", file_path.display()))
}

async fn query_apply_plan(namespace: &str, selected_outputs: &[String]) -> Result<ApplyPlan> {
    let deadline = Instant::now() + IPC_QUERY_TIMEOUT;

    loop {
        let attempt_namespace = namespace.to_owned();
        let attempt_outputs = selected_outputs.to_vec();

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

            let available = outputs
                .iter()
                .map(|output| (output.name.to_string(), output.clone()))
                .collect::<HashMap<_, _>>();

            let desired_names = if attempt_outputs.is_empty() {
                vec![outputs[0].name.to_string()]
            } else {
                attempt_outputs.clone()
            };

            let missing = desired_names
                .iter()
                .filter(|name| !available.contains_key((*name).as_str()))
                .cloned()
                .collect::<Vec<_>>();

            if !missing.is_empty() {
                let available_names = outputs
                    .iter()
                    .map(|output| output.name.as_ref())
                    .collect::<Vec<_>>()
                    .join(", ");
                bail!(
                    "selected monitor(s) not ready in daemon yet (missing: {}; available: {available_names})",
                    missing.join(", ")
                );
            }

            let mut groups: BTreeMap<(u32, u32, u8), ApplyGroup> = BTreeMap::new();
            for name in desired_names {
                let output = available
                    .get(name.as_str())
                    .ok_or_else(|| anyhow!("missing selected output '{name}'"))?;
                let dim = output.real_dim();
                let key = (dim.0, dim.1, output.pixel_format as u8);
                groups
                    .entry(key)
                    .and_modify(|group| group.outputs.push(name.clone()))
                    .or_insert_with(|| ApplyGroup {
                        outputs: vec![name.clone()],
                        dim,
                        format: output.pixel_format,
                    });
            }

            Ok(ApplyPlan {
                namespace: resolved_namespace,
                groups: groups.into_values().collect(),
            })
        })
        .await;

        match attempt {
            Ok(plan) => return Ok(plan),
            Err(err) => {
                let timed_out = Instant::now() >= deadline;
                let waiting_on_selected = !selected_outputs.is_empty()
                    && err
                        .to_string()
                        .contains("selected monitor(s) not ready in daemon yet");

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
    plan: ApplyPlan,
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

        let mut builder = ImageRequestBuilder::new(build_transition_from_state(&transition))
            .map_err(anyhow::Error::new)
            .context("request mmap failed")?;

        for group in plan.groups {
            let fitted = render_to_monitor_canvas(transformed.clone(), group.dim, scale_mode);
            let (img_bytes, pixel_format) =
                convert_dynamic_image_for_pixel_format(fitted, group.format);
            let img_send = ImgSend {
                path: file_path.to_string_lossy().to_string(),
                dim: group.dim,
                format: pixel_format,
                img: img_bytes.into_boxed_slice(),
            };

            builder.push(
                img_send,
                &plan.namespace,
                scale_mode.backend_resize(),
                "lanczos3",
                &group.outputs,
                None,
            );
        }

        let socket = IpcSocket::client(&plan.namespace)
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
    let transition_type = match super::model::TRANSITION_EFFECTS[state.effect_idx] {
        "fade" => TransitionType::Fade,
        "wipe" => TransitionType::Wipe,
        "grow" => TransitionType::Grow,
        _ => TransitionType::Simple,
    };

    let bezier = match super::model::EASING_PRESETS[state.easing_idx] {
        "linear" => (0.0, 0.0, 1.0, 1.0),
        "ease-in" => (0.42, 0.0, 1.0, 1.0),
        "ease-out" => (0.0, 0.0, 0.58, 1.0),
        _ => (0.42, 0.0, 0.58, 1.0),
    };

    Transition {
        transition_type,
        duration: state.duration_ms as f32 / 1000.0,
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
