//! Msgs flow into [`super::update`]; Cmds flow out to the runtime.
//!
//! A Msg is something that happened: a key, a click, a resize, a timer, or
//! a worker's result. A Cmd is plain data describing work for the runtime:
//! it never runs anything itself, so update tests can assert on it. Each
//! Cmd comes back as exactly one Msg (see [`super::services::perform`]).

use std::path::PathBuf;

use ratatui::crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::Size;
use vellum_ipc::{MonitorInfo, Status};

use super::model::{Meta, PreviewImage};
use crate::render::{Color, FitMode};

#[derive(Debug)]
pub enum Msg {
    Key(KeyEvent),
    Mouse(MouseEvent),
    /// A time the Model asked to be woken at has come (see
    /// `update::wake_at`): a toast to expire, a spinner frame, a key
    /// sequence timing out.
    Tick,
    Resize {
        width: u16,
        height: u16,
    },
    /// The Library scan finished.
    LibraryScanned(Vec<PathBuf>),
    /// The Daemon answered (or could not be reached).
    Daemon(Result<DaemonInfo, String>),
    /// A preview requested with `generation` is ready or failed.
    PreviewDone {
        generation: u64,
        path: PathBuf,
        result: Result<PreviewOutput, String>,
    },
    /// An Apply finished. `monitor` is `None` for every Monitor.
    Applied {
        monitor: Option<String>,
        result: Result<(), String>,
    },
    /// The Favorites file was read.
    FavoritesLoaded(Vec<PathBuf>),
    /// Saving `path` as a Favorite (or not) finished.
    FavoriteSaved {
        path: PathBuf,
        favorite: bool,
        result: Result<(), String>,
    },
    /// The `on_change` hook failed; why.
    HookFailed(String),
}

/// What the Daemon reports.
#[derive(Debug, Clone)]
pub struct DaemonInfo {
    pub monitors: Vec<MonitorInfo>,
    pub status: Status,
}

/// A finished preview.
#[derive(Debug)]
pub struct PreviewOutput {
    pub image: PreviewImage,
    pub meta: Meta,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    /// List every wallpaper under these folders.
    ScanLibrary(Vec<PathBuf>),
    /// Ask the Daemon (starting it if needed) for Monitors and Status.
    QueryDaemon,
    RenderPreview(PreviewJob),
    Apply(ApplyJob),
    /// Read the Favorites file.
    LoadFavorites,
    /// Make `path` a Favorite, or stop it being one.
    SaveFavorite {
        path: PathBuf,
        favorite: bool,
    },
}

/// Draw `path` as `monitor` would show it, to fit in `cells`.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewJob {
    pub generation: u64,
    pub path: PathBuf,
    /// The most cells the preview may cover.
    pub cells: Size,
    pub monitor: Screen,
    pub fit: FitMode,
    pub letterbox: Color,
}

/// The parts of a Monitor a preview depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Screen {
    pub width: u32,
    pub height: u32,
    pub scale_120: u32,
}

impl Screen {
    /// Used for previews until the Daemon reports a real Monitor.
    pub const FALLBACK: Screen = Screen {
        width: 1920,
        height: 1080,
        scale_120: 120,
    };
}

impl From<&MonitorInfo> for Screen {
    fn from(m: &MonitorInfo) -> Screen {
        Screen {
            width: m.physical_width,
            height: m.physical_height,
            scale_120: m.scale_120,
        }
    }
}

/// Put `path` on the Monitor named `monitor`, or on every Monitor.
#[derive(Debug, Clone, PartialEq)]
pub struct ApplyJob {
    pub path: PathBuf,
    pub monitor: Option<String>,
    pub fit: FitMode,
}
