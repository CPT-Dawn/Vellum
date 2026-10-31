//! Running Cmds: [`perform`] turns one Cmd into its one Msg using a
//! [`Services`] implementation. The runtime calls it on worker threads
//! with [`Live`]; tests call it inline with the harness's fake.

use std::path::{Path, PathBuf};

use image::DynamicImage;
use ratatui_image::Resize;
use ratatui_image::picker::Picker;

use super::model::{Meta, PreviewImage};
use super::msg::{ApplyJob, Cmd, DaemonInfo, Msg, PreviewJob, PreviewOutput};
use crate::client;
use crate::config::Config;
use crate::set::{self, Look};

/// The outside world, as the TUI needs it. Each method blocks; the runtime
/// only calls them off the UI thread.
pub trait Services: Send + Sync {
    /// Every wallpaper under `folders`.
    fn scan(&self, folders: &[PathBuf]) -> Vec<PathBuf>;
    /// The Daemon's Monitors and Status, starting it if needed.
    fn daemon(&self) -> Result<DaemonInfo, String>;
    /// Decode, render and encode one preview.
    fn preview(&self, job: &PreviewJob) -> Result<PreviewOutput, String>;
    /// Put a wallpaper on a Monitor.
    fn apply(&self, job: &ApplyJob) -> Result<(), String>;
}

/// Runs `cmd` and reports the result as a Msg.
pub fn perform(services: &dyn Services, cmd: Cmd) -> Msg {
    match cmd {
        Cmd::ScanLibrary(folders) => Msg::LibraryScanned(services.scan(&folders)),
        Cmd::QueryDaemon => Msg::Daemon(services.daemon()),
        Cmd::RenderPreview(job) => Msg::PreviewDone {
            generation: job.generation,
            result: services.preview(&job),
            path: job.path,
        },
        Cmd::Apply(job) => Msg::Applied {
            result: services.apply(&job),
            monitor: job.monitor,
        },
    }
}

/// The real Library, Daemon and terminal graphics.
pub struct Live {
    pub picker: Picker,
    pub config: Config,
}

impl Services for Live {
    fn scan(&self, folders: &[PathBuf]) -> Vec<PathBuf> {
        crate::library::scan(folders)
    }

    fn daemon(&self) -> Result<DaemonInfo, String> {
        let socket = client::daemon()?;
        Ok(DaemonInfo {
            monitors: client::monitors(&socket)?,
            status: client::status(&socket)?,
        })
    }

    fn preview(&self, job: &PreviewJob) -> Result<PreviewOutput, String> {
        let bytes = std::fs::metadata(&job.path)
            .map_err(|err| format!("cannot read {}: {err}", job.path.display()))?
            .len();
        let source = crate::decode::load(&job.path)?;
        let meta = Meta {
            width: source.width(),
            height: source.height(),
            bytes,
        };
        let font = self.picker.font_size();
        let frame = super::preview::render_preview(&source, job, font);
        let cells = super::preview::cells_for(frame.dimensions(), font);
        let protocol = self
            .picker
            .new_protocol(DynamicImage::ImageRgb8(frame), cells, Resize::Fit(None))
            .map_err(|err| format!("cannot encode the preview: {err}"))?;
        Ok(PreviewOutput {
            image: PreviewImage::Protocol(Box::new(protocol)),
            meta,
        })
    }

    fn apply(&self, job: &ApplyJob) -> Result<(), String> {
        let image = crate::decode::load(&job.path)?;
        let socket = client::daemon()?;
        let monitors = set::target(client::monitors(&socket)?, Some(&job.monitor))?;
        let source = absolute(&job.path);
        let look = Look {
            fit: job.fit,
            letterbox: self.config.letterbox,
            transition: self.config.transition,
            restore: false,
        };
        for monitor in &monitors {
            set::present(&socket, &image, monitor, &source, look)?;
        }
        Ok(())
    }
}

fn absolute(path: &Path) -> String {
    std::path::absolute(path)
        .unwrap_or_else(|_| path.to_owned())
        .to_string_lossy()
        .into_owned()
}
