//! Msgs flow into [`super::update`]; Cmds flow out to the runtime.
//!
//! A Msg is something that happened: a key, a resize, or a worker's result.
//! A Cmd is plain data describing work for the runtime: it never runs
//! anything itself, so update tests can assert on it. Each Cmd comes back
//! as exactly one Msg (see [`super::services::perform`]).

use std::path::PathBuf;
use std::time::Duration;

use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Size;
use vellum_ipc::{MonitorInfo, Status};

use super::model::{Meta, PreviewImage};
use crate::render::{Color, FitMode};

#[derive(Debug)]
pub enum Msg {
    Key(KeyEvent),
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
    /// A thumbnail requested with grid `generation` is ready or failed.
    ThumbDone {
        generation: u64,
        path: PathBuf,
        result: Result<PreviewOutput, String>,
    },
    /// A [`Cmd::After`] delay passed.
    Woke(Wake),
    /// An Apply finished.
    Applied {
        monitor: String,
        result: Result<(), String>,
    },
}

/// What the Daemon reports.
#[derive(Debug, Clone)]
pub struct DaemonInfo {
    pub monitors: Vec<MonitorInfo>,
    pub status: Status,
}

/// A finished preview.
#[derive(Debug, Clone)]
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
    /// Render a preview, a prefetch or a thumbnail (see [`Purpose`]).
    RenderPreview(PreviewJob),
    Apply(ApplyJob),
    /// Send back [`Msg::Woke`] once `delay` has passed. A newer `After`
    /// with the same kind of [`Wake`] replaces a pending one, so this is a
    /// debounce.
    After {
        delay: Duration,
        wake: Wake,
    },
}

/// What a [`Cmd::After`] wakes update up for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// Render the preview of preview `generation`, if still current.
    Preview { generation: u64 },
    /// Render the tiles of grid `generation`, if still current.
    Grid { generation: u64 },
}

/// Why a preview is rendered. Workers drop a job once its generation is no
/// longer the latest of its kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// The preview pane. Answered with [`Msg::PreviewDone`].
    Preview,
    /// A neighbour of the selection, rendered at low priority into the
    /// caches. Answered with nothing.
    Prefetch,
    /// A grid tile. Answered with [`Msg::ThumbDone`]; its generation is the
    /// grid's.
    Thumb,
}

/// Draw `path` as `monitor` would show it, to fit in `cells`.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewJob {
    pub generation: u64,
    pub purpose: Purpose,
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

/// Put `path` on the Monitor named `monitor`.
#[derive(Debug, Clone, PartialEq)]
pub struct ApplyJob {
    pub path: PathBuf,
    pub monitor: String,
    pub fit: FitMode,
}
