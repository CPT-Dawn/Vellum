//! The Model: everything the TUI shows, and nothing it does not.
//!
//! The Model is plain data. [`super::update`] is the only code that changes
//! it, and [`super::view`] only reads it.

use std::path::{Path, PathBuf};

use ratatui::layout::{Rect, Size};
use vellum_ipc::{MonitorInfo, Status};

use super::theme::Theme;
use crate::config::Config;
use crate::render::{Color, FitMode};

/// The whole TUI state.
#[derive(Debug)]
pub struct Model {
    /// Which keys mean what right now. #11 adds Search, Help and Palette;
    /// #13 adds Rotation and Welcome.
    pub mode: Mode,
    /// The Library folders, from the Config.
    pub folders: Vec<PathBuf>,
    /// Every wallpaper in the Library, sorted by path. Empty while
    /// `scanning`.
    pub library: Vec<PathBuf>,
    /// True until the first Library scan reports back.
    pub scanning: bool,
    /// Index into `library` of the highlighted wallpaper.
    pub selected: usize,
    /// Index of the first wallpaper the list shows. Update keeps the
    /// selection in view.
    pub list_offset: usize,
    /// What the Daemon told us, or why it could not.
    pub daemon: Daemon,
    /// Index into the Daemon's Monitors of the one Enter applies to.
    pub target: usize,
    /// Rotation, as shown in the header.
    pub rotation: Rotation,
    /// The Fit mode the preview and Enter use.
    pub fit: FitMode,
    /// The letterbox color the preview and Enter use.
    pub letterbox: Color,
    pub preview: Preview,
    /// The latest confirmation or error, shown in the footer.
    pub toast: Option<Toast>,
    /// The terminal's size, so update can ask for a preview that fills its
    /// pane.
    pub area: Rect,
    /// `$HOME`, to print folders as `~/...`.
    pub home: Option<PathBuf>,
    pub theme: Theme,
    /// Set once the user asked to leave.
    pub quit: bool,
    /// Whether the last Msg changed anything on screen. The runtime only
    /// redraws when it is set. `update` sets it for every Msg and clears it
    /// for the ones it ignores.
    pub dirty: bool,
}

impl Model {
    /// A Model for a terminal of `area`, before the Library or the Daemon
    /// has answered.
    pub fn new(config: &Config, theme: Theme, area: Rect, home: Option<PathBuf>) -> Model {
        Model {
            mode: Mode::Normal,
            folders: config.library.clone(),
            library: Vec::new(),
            scanning: true,
            selected: 0,
            list_offset: 0,
            daemon: Daemon::Connecting,
            target: 0,
            rotation: Rotation::Off,
            fit: config.fit,
            letterbox: config.letterbox,
            preview: Preview::default(),
            toast: None,
            area,
            home,
            theme,
            quit: false,
            dirty: true,
        }
    }

    /// The highlighted wallpaper, if the Library has any.
    pub fn selection(&self) -> Option<&PathBuf> {
        self.library.get(self.selected)
    }

    /// The Monitors the Daemon reported, or none.
    pub fn monitors(&self) -> &[MonitorInfo] {
        match &self.daemon {
            Daemon::Up { monitors, .. } => monitors,
            _ => &[],
        }
    }

    /// The Monitor Enter applies to and the preview is drawn for.
    pub fn targeted(&self) -> Option<&MonitorInfo> {
        self.monitors().get(self.target)
    }

    /// `path` with `$HOME` shortened to `~`.
    pub fn tilde(&self, path: &Path) -> String {
        match self.home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
            Some(rest) if rest.as_os_str().is_empty() => "~".into(),
            Some(rest) => format!("~/{}", rest.display()),
            None => path.display().to_string(),
        }
    }
}

/// Which keymap is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Browsing the list.
    Normal,
}

/// The Daemon as the TUI knows it.
#[derive(Debug)]
pub enum Daemon {
    /// The first query has not answered yet.
    Connecting,
    Up {
        monitors: Vec<MonitorInfo>,
    },
    /// The Daemon could not be reached or started; the reason.
    Down(String),
}

/// Rotation status for the header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rotation {
    Off,
    /// Running, changing every `interval_secs`.
    #[expect(dead_code, reason = "Status reports Rotation once #8 lands")]
    On {
        interval_secs: u64,
    },
}

impl Rotation {
    /// What `vellum status` says about Rotation. `Status` carries no
    /// Rotation yet, so this is always Off until #8 adds it.
    pub fn from_status(_status: &Status) -> Rotation {
        Rotation::Off
    }
}

/// The preview pane.
#[derive(Debug, Default)]
pub struct Preview {
    /// Bumped by every new preview request. Results from older requests
    /// are dropped.
    pub generation: u64,
    pub state: PreviewState,
}

#[derive(Debug, Default)]
pub enum PreviewState {
    /// Nothing selected.
    #[default]
    Empty,
    Loading {
        path: PathBuf,
    },
    Ready {
        path: PathBuf,
        image: PreviewImage,
        meta: Meta,
    },
    Failed {
        path: PathBuf,
        error: String,
    },
}

/// Facts about a wallpaper file, for the metadata strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

/// A preview ready to draw.
pub enum PreviewImage {
    /// Encoded for the terminal's graphics protocol.
    Protocol(Box<ratatui_image::protocol::Protocol>),
    /// A stand-in of `size` cells showing `label`, for tests and snapshots.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "only the test harness draws placeholders")
    )]
    Placeholder { size: Size, label: String },
}

impl PreviewState {
    /// The wallpaper this preview is of.
    pub fn path(&self) -> Option<&Path> {
        match self {
            PreviewState::Empty => None,
            PreviewState::Loading { path }
            | PreviewState::Ready { path, .. }
            | PreviewState::Failed { path, .. } => Some(path),
        }
    }
}

impl PreviewImage {
    /// The cells the image covers.
    pub fn size(&self) -> Size {
        match self {
            PreviewImage::Protocol(protocol) => protocol.size(),
            PreviewImage::Placeholder { size, .. } => *size,
        }
    }
}

impl std::fmt::Debug for PreviewImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PreviewImage::Protocol(p) => write!(f, "Protocol({:?})", p.size()),
            PreviewImage::Placeholder { size, label } => {
                write!(f, "Placeholder({size:?}, {label})")
            }
        }
    }
}

/// A short message in the footer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub kind: ToastKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Error,
}
