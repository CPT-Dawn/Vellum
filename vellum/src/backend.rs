use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use thiserror::Error;

use common::cache;
use common::ipc::{
    self, Answer, BgInfo, ClearSend, Coord, IpcError, IpcSocket, Position, RequestSend, Transition,
    TransitionType,
};

use crate::app::{DaemonStatus, Monitor, ScalingMode};
use crate::imgproc::{self, ImgBuf, ResizeStrategy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DaemonResourceUsage {
    pub pid: u32,
    pub memory_kib: u64,
    pub total_memory_kib: u64,
}

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("ipc error: {0}")]
    Ipc(#[from] IpcError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Message(String),
}

pub struct Backend {
    namespace: String,
    daemon_child: Option<Child>,
}

impl Backend {
    pub fn new(namespace: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            daemon_child: None,
        }
    }

    pub fn status(&mut self) -> DaemonStatus {
        if let Some(child) = self.daemon_child.as_mut() {
            match child.try_wait() {
                Ok(Some(exit_status)) => {
                    self.daemon_child = None;
                    if exit_status.success() {
                        DaemonStatus::Stopped
                    } else {
                        DaemonStatus::Crashed
                    }
                }
                Ok(None) => DaemonStatus::Running,
                Err(_) => DaemonStatus::Crashed,
            }
        } else if self.ping().unwrap_or(false) {
            DaemonStatus::Running
        } else {
            DaemonStatus::Stopped
        }
    }

    pub fn refresh_monitors(&self) -> Result<Vec<Monitor>, BackendError> {
        Ok(self.query_infos()?.into_iter().map(Monitor::from).collect())
    }

    pub fn resource_snapshot(&self) -> Option<DaemonResourceUsage> {
        let pid = self
            .daemon_child
            .as_ref()
            .map(|child| child.id())
            .or_else(|| find_daemon_pid(&self.namespace))?;

        Some(DaemonResourceUsage {
            pid,
            memory_kib: process_memory_kib(pid)?,
            total_memory_kib: system_total_memory_kib()?,
        })
    }

    pub fn start_daemon(&mut self) -> Result<DaemonStatus, BackendError> {
        if self.ping()? {
            return Ok(DaemonStatus::Running);
        }

        let mut command = Command::new(daemon_program_path());
        command
            .arg("--namespace")
            .arg(&self.namespace)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }

        let child = command.spawn()?;
        self.daemon_child = Some(child);

        for _ in 0..50 {
            if self.ping()? {
                return Ok(DaemonStatus::Running);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        let _ = self.stop_daemon();
        Err(BackendError::Message(
            "daemon did not become ready after spawning".to_string(),
        ))
    }

    pub fn stop_daemon(&mut self) -> Result<DaemonStatus, BackendError> {
        if let Some(child) = self.daemon_child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
            self.daemon_child = None;
            return Ok(DaemonStatus::Stopped);
        }

        if !self.ping()? {
            return Ok(DaemonStatus::Stopped);
        }

        let socket = IpcSocket::client(&self.namespace)?;
        RequestSend::Kill.send(&socket)?;

        for _ in 0..20 {
            if !self.ping()? {
                return Ok(DaemonStatus::Stopped);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        Ok(DaemonStatus::Crashed)
    }

    pub fn apply_wallpaper(
        &self,
        wallpaper: &Path,
        monitor_name: &str,
        scaling: ScalingMode,
    ) -> Result<(), BackendError> {
        let info = self
            .query_infos()?
            .into_iter()
            .find(|info| info.name.as_ref() == monitor_name)
            .ok_or_else(|| BackendError::Message(format!("unknown monitor: {monitor_name}")))?;

        let resize = resize_strategy_for_scaling_mode(scaling);
        let request = build_image_request(&self.namespace, wallpaper, &info, resize)?;

        let socket = IpcSocket::client(&self.namespace)?;
        RequestSend::Img(request).send(&socket)?;

        match Answer::receive(socket.recv()?)? {
            Answer::Ok => Ok(()),
            _ => Err(BackendError::Message(
                "daemon returned an unexpected response".to_string(),
            )),
        }
    }

    pub fn clear_wallpaper(&self, monitor_name: &str) -> Result<(), BackendError> {
        let socket = IpcSocket::client(&self.namespace)?;
        let request = ClearSend {
            color: [0, 0, 0, 255],
            outputs: vec![monitor_name.to_string()].into_boxed_slice(),
        }
        .create_request()
        .map_err(|error| BackendError::Message(error.to_string()))?;

        RequestSend::Clear(request).send(&socket)?;

        match Answer::receive(socket.recv()?)? {
            Answer::Ok => Ok(()),
            _ => Err(BackendError::Message(
                "daemon returned an unexpected response".to_string(),
            )),
        }
    }

    pub fn toggle_pause(&self) -> Result<(), BackendError> {
        let socket = IpcSocket::client(&self.namespace)?;
        RequestSend::Pause.send(&socket)?;

        match Answer::receive(socket.recv()?)? {
            Answer::Ok => Ok(()),
            _ => Err(BackendError::Message(
                "daemon returned an unexpected response".to_string(),
            )),
        }
    }

    fn ping(&self) -> Result<bool, BackendError> {
        let socket = match IpcSocket::client(&self.namespace) {
            Ok(socket) => socket,
            Err(_) => return Ok(false),
        };

        RequestSend::Ping.send(&socket)?;
        match Answer::receive(socket.recv()?)? {
            Answer::Ping(_) => Ok(true),
            _ => Err(BackendError::Message(
                "daemon returned an unexpected response".to_string(),
            )),
        }
    }

    fn query_infos(&self) -> Result<Vec<BgInfo>, BackendError> {
        let socket = IpcSocket::client(&self.namespace)?;
        RequestSend::Query.send(&socket)?;

        match Answer::receive(socket.recv()?)? {
            Answer::Info(infos) => Ok(infos.into_vec()),
            _ => Err(BackendError::Message(
                "daemon returned an unexpected response".to_string(),
            )),
        }
    }
}

fn resize_strategy_for_scaling_mode(mode: ScalingMode) -> ResizeStrategy {
    match mode {
        ScalingMode::Fill => ResizeStrategy::Stretch,
        ScalingMode::Fit => ResizeStrategy::Fit,
        ScalingMode::Crop => ResizeStrategy::Crop,
        ScalingMode::Center => ResizeStrategy::No,
        ScalingMode::Tile => ResizeStrategy::Stretch,
    }
}

fn build_image_request(
    namespace: &str,
    wallpaper: &Path,
    info: &BgInfo,
    resize: ResizeStrategy,
) -> Result<common::mmap::Mmap, BackendError> {
    let img_buf = ImgBuf::new(wallpaper).map_err(BackendError::Message)?;
    let dimensions = info.real_dim();
    let pixel_format = info.pixel_format;
    let resize_name = resize.as_str();
    let filter_name = "Lanczos3";
    let filter = fast_image_resize::FilterType::Lanczos3;
    let path_string = canonical_wallpaper_path(wallpaper)?;
    let output_names = vec![info.name.to_string()];
    let transition = Transition {
        transition_type: TransitionType::None,
        duration: 0.0,
        step: core::num::NonZeroU8::MAX,
        fps: 30,
        angle: 0.0,
        pos: Position::new(Coord::Pixel(0.0), Coord::Pixel(0.0)),
        bezier: (0.0, 0.0, 0.0, 0.0),
        wave: (0.0, 0.0),
        invert_y: false,
    };

    let mut builder = ipc::ImageRequestBuilder::new(transition)
        .map_err(|error| BackendError::Message(error.to_string()))?;
    let cache_path = PathBuf::from(&path_string);

    match img_buf.decode_prepare() {
        imgproc::DecodeBuffer::RasterImage(imgbuf) => {
            let img_raw = imgbuf.decode(pixel_format).map_err(BackendError::Message)?;
            let animation = if imgbuf.is_animated() {
                match cache::load_animation_frames(
                    &cache_path,
                    dimensions,
                    resize_name,
                    pixel_format,
                ) {
                    Ok(Some(animation)) => Some(animation),
                    Ok(None) => Some(ipc::Animation {
                        animation: imgproc::compress_frames(
                            imgbuf.as_frames().map_err(BackendError::Message)?,
                            dimensions,
                            pixel_format,
                            filter,
                            resize,
                            [0, 0, 0, 255],
                        )
                        .map_err(BackendError::Message)?
                        .into_boxed_slice(),
                    }),
                    Err(error) => return Err(BackendError::Message(error.to_string())),
                }
            } else {
                None
            };

            let image = match resize {
                ResizeStrategy::No => imgproc::img_pad(&img_raw, dimensions, [0, 0, 0, 255]),
                ResizeStrategy::Crop => imgproc::img_resize_crop(&img_raw, dimensions, filter)
                    .map_err(BackendError::Message)?,
                ResizeStrategy::Fit => {
                    imgproc::img_resize_fit(&img_raw, dimensions, filter, [0, 0, 0, 255])
                        .map_err(BackendError::Message)?
                }
                ResizeStrategy::Stretch => {
                    imgproc::img_resize_stretch(&img_raw, dimensions, filter)
                        .map_err(BackendError::Message)?
                }
            };

            builder.push(
                ipc::ImgSend {
                    img: image,
                    path: path_string,
                    dim: dimensions,
                    format: pixel_format,
                },
                namespace,
                resize_name,
                filter_name,
                &output_names,
                animation,
            );
        }
        imgproc::DecodeBuffer::VectorImage(imgbuf) => {
            let image = imgbuf
                .decode(pixel_format, dimensions.0, dimensions.1)
                .map_err(BackendError::Message)?;

            let image = match resize {
                ResizeStrategy::No => imgproc::img_pad(&image, dimensions, [0, 0, 0, 255]),
                ResizeStrategy::Crop => imgproc::img_resize_crop(&image, dimensions, filter)
                    .map_err(BackendError::Message)?,
                ResizeStrategy::Fit => {
                    imgproc::img_resize_fit(&image, dimensions, filter, [0, 0, 0, 255])
                        .map_err(BackendError::Message)?
                }
                ResizeStrategy::Stretch => imgproc::img_resize_stretch(&image, dimensions, filter)
                    .map_err(BackendError::Message)?,
            };

            builder.push(
                ipc::ImgSend {
                    img: image,
                    path: path_string,
                    dim: dimensions,
                    format: pixel_format,
                },
                namespace,
                resize_name,
                filter_name,
                &output_names,
                None,
            );
        }
    }

    Ok(builder.build())
}

fn find_daemon_pid(namespace: &str) -> Option<u32> {
    let proc_dir = fs::read_dir("/proc").ok()?;

    for entry in proc_dir.flatten() {
        let Some(pid) = entry.file_name().to_string_lossy().parse::<u32>().ok() else {
            continue;
        };

        let cmdline = fs::read_to_string(entry.path().join("cmdline")).unwrap_or_default();
        if cmdline.contains("vellum-daemon") && cmdline.contains(namespace) {
            return Some(pid);
        }

        let comm = fs::read_to_string(entry.path().join("comm")).unwrap_or_default();
        if comm.trim() == "vellum-daemon" {
            return Some(pid);
        }
    }

    None
}

fn process_memory_kib(pid: u32) -> Option<u64> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;

    status.lines().find_map(|line| {
        line.strip_prefix("VmRSS:")
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse::<u64>().ok())
    })
}

fn system_total_memory_kib() -> Option<u64> {
    let meminfo = fs::read_to_string("/proc/meminfo").ok()?;
    meminfo.lines().find_map(|line| {
        line.strip_prefix("MemTotal:")
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse::<u64>().ok())
    })
}

fn canonical_wallpaper_path(wallpaper: &Path) -> Result<String, BackendError> {
    if wallpaper == Path::new("-") {
        Ok("STDIN".to_string())
    } else {
        Ok(wallpaper
            .canonicalize()
            .unwrap_or_else(|_| wallpaper.to_path_buf())
            .display()
            .to_string())
    }
}

fn daemon_program_path() -> PathBuf {
    if let Ok(mut path) = std::env::current_exe()
        && path.pop()
    {
        let candidate = path.join("vellum-daemon");
        if candidate.exists() {
            return candidate;
        }
    }

    PathBuf::from("vellum-daemon")
}
