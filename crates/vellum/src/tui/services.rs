//! Running Cmds: [`perform`] turns one Cmd into its Msg using a
//! [`Services`] implementation. The runtime calls it on worker threads
//! with [`Live`]; tests call it inline with the harness's fake.
//!
//! Preview work checks [`Latest`] before it decodes, before it encodes and
//! before it sends, and gives up as soon as its generation is stale.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use image::DynamicImage;
use ratatui::layout::Size;
use ratatui_image::Resize;
use ratatui_image::picker::Picker;

use super::cache::{DiskCache, Lru};
use super::model::{Meta, Model, PreviewImage};
use super::msg::{
    ApplyJob, Cmd, DaemonInfo, MonitorShape, Msg, PreviewJob, PreviewOutput, Purpose,
};
use super::preview::{cells_for, decode_entry, encode_entry, frame_size, render_preview};
use vellum_ipc::{Request, Response, Settings, Status};

use crate::client;
use crate::config::edit::{self, Edit, Value};
use crate::config::{Config, Named as _};
use crate::render::{Color, FitMode};
use crate::set::{self, Look};

/// The outside world, as the TUI needs it. Each method blocks; the runtime
/// only calls them off the UI thread.
pub trait Services: Send + Sync {
    /// Every wallpaper under `folders`.
    fn scan(&self, folders: &[PathBuf]) -> Vec<PathBuf>;
    /// The Daemon's Monitors and Status, starting it if needed.
    fn daemon(&self) -> Result<DaemonInfo, String>;
    /// Decode, render and encode one preview. Returns `None` as soon as
    /// `stale()` says nobody wants it any more; implementations check it
    /// before decoding and before encoding.
    fn preview(
        &self,
        job: &PreviewJob,
        stale: &dyn Fn() -> bool,
    ) -> Option<Result<PreviewOutput, String>>;
    /// Put a wallpaper on a Monitor.
    fn apply(&self, job: &ApplyJob) -> Result<(), String>;
    /// Every Favorite, missing files included.
    fn favorites(&self) -> Vec<PathBuf>;
    /// Make `path` a Favorite, or stop it being one.
    fn save_favorite(&self, path: &Path, favorite: bool) -> Result<(), String>;
    /// Write `edits` to the Config; the new Config.
    fn save_config(&self, edits: &[Edit]) -> Result<Config, String>;
    /// Hand the Daemon `settings` (if it runs); its Status afterwards.
    fn push_settings(&self, settings: &Settings) -> Result<Status, String>;
    /// Start or stop Rotation; the Daemon's Status afterwards.
    fn rotation(&self, start: bool) -> Result<Status, String>;
    /// The folders a typed path could complete to.
    fn complete(&self, input: &str) -> Vec<String>;
    /// Add `folder` to the Library in the Config; the new Config.
    fn save_library(&self, folder: &Path) -> Result<Config, String>;
}

/// The newest preview and grid generations, published by the UI thread
/// after every update and read by workers.
#[derive(Debug, Default)]
pub struct Latest {
    preview: AtomicU64,
    grid: AtomicU64,
}

impl Latest {
    /// Records `m`'s generations. Call before queueing update's Cmds.
    pub fn publish(&self, m: &Model) {
        self.preview.store(m.preview.generation, Ordering::Release);
        self.grid.store(m.grid.generation, Ordering::Release);
    }

    /// Whether `job` is still wanted.
    pub fn is_current(&self, job: &PreviewJob) -> bool {
        let latest = match job.purpose {
            Purpose::Preview | Purpose::Prefetch => &self.preview,
            Purpose::Thumb => &self.grid,
        };
        latest.load(Ordering::Acquire) == job.generation
    }
}

/// Runs `cmd` and reports the result as a Msg. Stale preview work and
/// prefetches report nothing. [`Cmd::After`] fires at once here; the
/// runtime holds it for its delay instead of calling this.
pub fn perform(services: &dyn Services, cmd: Cmd, latest: &Latest) -> Option<Msg> {
    Some(match cmd {
        Cmd::ScanLibrary(folders) => Msg::LibraryScanned(services.scan(&folders)),
        Cmd::QueryDaemon => Msg::Daemon(services.daemon()),
        Cmd::RenderPreview(job) => {
            let stale = || !latest.is_current(&job);
            let result = services.preview(&job, &stale)?;
            if stale() {
                return None;
            }
            let (generation, path) = (job.generation, job.path);
            match job.purpose {
                Purpose::Preview => Msg::PreviewDone {
                    generation,
                    path,
                    result,
                },
                Purpose::Thumb => Msg::ThumbDone {
                    generation,
                    path,
                    result,
                },
                Purpose::Prefetch => return None,
            }
        }
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
        Cmd::SaveConfig { generation, edits } => Msg::ConfigSaved {
            generation,
            result: services.save_config(&edits),
        },
        Cmd::PushSettings(settings) => Msg::RotationUpdated {
            toggled: false,
            result: services.push_settings(&settings),
        },
        Cmd::RotationControl(start) => Msg::RotationUpdated {
            toggled: true,
            result: services.rotation(start),
        },
        Cmd::Complete(input) => Msg::Completed {
            candidates: services.complete(&input),
            input,
        },
        Cmd::SaveLibrary(folder) => Msg::LibrarySaved(services.save_library(&folder)),
        Cmd::After { wake, .. } => Msg::Woke(wake),
    })
}

/// Encoded previews kept in memory. Thumbnails live in the Model instead.
const MEMORY_PREVIEWS: usize = 12;

/// The real Library, Daemon and terminal graphics.
pub struct Live {
    picker: Picker,
    config: Config,
    memory: Mutex<Lru<MemoryKey, PreviewOutput>>,
    disk: Option<DiskCache>,
    /// Favorites saves read, edit and write the whole file: one at a time.
    favorites_lock: Mutex<()>,
    /// Config writes too.
    config_lock: Mutex<()>,
}

/// What an encoded preview depends on: the file, its mtime, and the cells
/// and look it was rendered for.
#[derive(Debug, PartialEq)]
struct MemoryKey {
    path: PathBuf,
    mtime: SystemTime,
    cells: Size,
    monitor: MonitorShape,
    fit: FitMode,
    letterbox: Color,
}

impl Live {
    /// Live services, with the disk cache in `thumbs` (if known) capped by
    /// the Config.
    pub fn new(picker: Picker, config: Config, thumbs: Option<PathBuf>) -> Live {
        let cap = u64::from(config.preview_cache_mb) << 20;
        Live {
            picker,
            disk: thumbs.map(|dir| DiskCache::new(dir, cap)),
            config,
            memory: Mutex::new(Lru::new(MEMORY_PREVIEWS)),
            favorites_lock: Mutex::new(()),
            config_lock: Mutex::new(()),
        }
    }

    /// The frame for `job` and its source's size, from the disk cache or
    /// freshly decoded and rendered (and then cached). `None` if stale.
    fn frame(
        &self,
        job: &PreviewJob,
        file: &FileId,
        stale: &dyn Fn() -> bool,
    ) -> Option<Result<Rendered, String>> {
        let font = self.picker.font_size();
        let (w, h) = frame_size(job.cells, font, job.monitor);
        let key = format!(
            "{}\0{}\0{}\0{w}x{h}\0{}x{}@{}\0{}\0{:02x?}",
            job.path.display(),
            file.mtime_ns,
            file.bytes,
            job.monitor.width,
            job.monitor.height,
            job.monitor.scale_120,
            job.fit.name(),
            job.letterbox,
        );
        if let Some(hit) = self.disk.as_ref().and_then(|d| decode_entry(&d.get(&key)?)) {
            return Some(Ok(hit));
        }
        if stale() {
            return None;
        }
        let source = match crate::decode::load(&job.path) {
            Ok(source) => source,
            Err(err) => return Some(Err(err)),
        };
        let frame = render_preview(&source, job, font);
        let dims = source.dimensions();
        drop(source);
        if let Some(disk) = &self.disk
            && let Some(entry) = encode_entry(&frame, dims)
        {
            disk.put(&key, &entry);
        }
        Some(Ok((frame, dims)))
    }
}

/// A rendered frame and its source's size in pixels.
type Rendered = (image::RgbImage, (u32, u32));

/// A file's identity for caching.
struct FileId {
    mtime: SystemTime,
    mtime_ns: u128,
    bytes: u64,
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

    fn preview(
        &self,
        job: &PreviewJob,
        stale: &dyn Fn() -> bool,
    ) -> Option<Result<PreviewOutput, String>> {
        if stale() {
            return None;
        }
        let file = match std::fs::metadata(&job.path) {
            Ok(meta) => {
                let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                FileId {
                    mtime,
                    mtime_ns: mtime
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .map_or(0, |d| d.as_nanos()),
                    bytes: meta.len(),
                }
            }
            Err(err) => return Some(Err(format!("cannot read {}: {err}", job.path.display()))),
        };
        let memory_key = (job.purpose != Purpose::Thumb).then(|| MemoryKey {
            path: job.path.clone(),
            mtime: file.mtime,
            cells: job.cells,
            monitor: job.monitor,
            fit: job.fit,
            letterbox: job.letterbox,
        });
        if let Some(key) = &memory_key
            && let Some(hit) = self.memory.lock().ok()?.get(key)
        {
            return Some(Ok(hit.clone()));
        }

        let (frame, (width, height)) = match self.frame(job, &file, stale)? {
            Ok(found) => found,
            Err(err) => return Some(Err(err)),
        };
        if stale() {
            return None;
        }
        let cells = cells_for(frame.dimensions(), self.picker.font_size());
        let protocol =
            match self
                .picker
                .new_protocol(DynamicImage::ImageRgb8(frame), cells, Resize::Fit(None))
            {
                Ok(protocol) => protocol,
                Err(err) => return Some(Err(format!("cannot encode the preview: {err}"))),
            };
        let out = PreviewOutput {
            image: PreviewImage::Protocol(Box::new(protocol)),
            meta: Meta {
                width,
                height,
                bytes: file.bytes,
            },
        };
        if let Some(key) = memory_key
            && let Ok(mut memory) = self.memory.lock()
        {
            memory.put(key, out.clone());
        }
        Some(Ok(out))
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
            origin: vellum_ipc::Origin::Manual,
        };
        for monitor in &monitors {
            // A pick by hand: the Daemon restarts the Monitor's Rotation
            // timer, as with `vellum set`.
            set::present(&socket, &image, monitor, &source, look)?;
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

    fn save_config(&self, edits: &[Edit]) -> Result<Config, String> {
        let _one_at_a_time = self.config_lock.lock().unwrap_or_else(|e| e.into_inner());
        edit::save(edits)
    }

    fn push_settings(&self, settings: &Settings) -> Result<Status, String> {
        // Not running: it reads the Config when it starts.
        let socket = client::socket()?;
        if !client::is_running(&socket) {
            return Err("vellumd is not running; the new settings apply when it starts".into());
        }
        expect_ok(client::ask(
            &socket,
            &Request::Settings(settings.clone()),
            &[],
        )?)?;
        client::status(&socket)
    }

    fn rotation(&self, start: bool) -> Result<Status, String> {
        let socket = client::daemon()?;
        let request = match start {
            true => Request::RotationStart,
            false => Request::RotationStop,
        };
        expect_ok(client::ask(&socket, &request, &[])?)?;
        client::status(&socket)
    }

    fn complete(&self, input: &str) -> Vec<String> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        super::complete::folders(input, home.as_deref())
    }

    fn save_library(&self, folder: &Path) -> Result<Config, String> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let shown = edit::tilde(folder, home.as_deref());
        if !folder.is_dir() {
            return Err(format!("{shown} is not a folder"));
        }
        let _one_at_a_time = self.config_lock.lock().unwrap_or_else(|e| e.into_inner());
        let (current, _) = crate::config::read_current()?;
        // Keep the folders that exist; the chosen one joins them.
        let mut folders: Vec<String> = current
            .library
            .iter()
            .filter(|f| f.is_dir() && f.as_path() != folder)
            .map(|f| edit::tilde(f, home.as_deref()))
            .collect();
        folders.push(shown);
        edit::save(&[Edit::Set {
            table: Vec::new(),
            key: "library".into(),
            value: Value::List(folders),
        }])
    }
}

fn expect_ok(response: Response) -> Result<(), String> {
    match response {
        Response::Ok => Ok(()),
        Response::Error(err) => Err(err),
        other => Err(client::unexpected(&other)),
    }
}

fn absolute(path: &Path) -> String {
    std::path::absolute(path)
        .unwrap_or_else(|_| path.to_owned())
        .to_string_lossy()
        .into_owned()
}
