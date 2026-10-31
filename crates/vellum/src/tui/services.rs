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
    /// Every Favorite, missing files included.
    fn favorites(&self) -> Vec<PathBuf>;
    /// Make `path` a Favorite, or stop it being one.
    fn save_favorite(&self, path: &Path, favorite: bool) -> Result<(), String>;
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
        Cmd::LoadFavorites => Msg::FavoritesLoaded(services.favorites()),
        Cmd::SaveFavorite { path, favorite } => Msg::FavoriteSaved {
            result: services.save_favorite(&path, favorite),
            path,
            favorite,
        },
    }
}

/// The real Library, Daemon and terminal graphics.
pub struct Live {
    pub picker: Picker,
    pub config: Config,
    pub favorites_lock: std::sync::Mutex<()>,
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
        let monitors = set::target(client::monitors(&socket)?, job.monitor.as_deref())?;
        let source = absolute(&job.path);
        let look = Look {
            fit: job.fit,
            letterbox: self.config.letterbox,
            transition: self.config.transition,
            restore: false,
        };
        for monitor in &monitors {
            set::present(&socket, &image, monitor, &source, look)?;
            // A pick by hand stays up a full Rotation interval, as with
            // `vellum set`.
            crate::rotation::reset_timer(&socket, &monitor.name)?;
        }
        Ok(())
    }

    fn favorites(&self) -> Vec<PathBuf> {
        crate::favorites::load()
    }

    fn save_favorite(&self, path: &Path, favorite: bool) -> Result<(), String> {
        // Workers may run two saves at once; each reads, edits and writes
        // the whole file.
        let _one_at_a_time = self
            .favorites_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::favorites::set(path, favorite)
    }
}

fn absolute(path: &Path) -> String {
    std::path::absolute(path)
        .unwrap_or_else(|_| path.to_owned())
        .to_string_lossy()
        .into_owned()
}
