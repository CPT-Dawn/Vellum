//! The Model: everything the TUI shows, and nothing it does not.
//!
//! The Model is plain data. [`super::update`] is the only code that changes
//! it, and [`super::view`] only reads it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ratatui::layout::{Rect, Size};
use vellum_ipc::{MonitorInfo, RotationSettings, RotationSource, Status};

use super::keymap::{self, Key, Keymap};
use super::layout::{self, Areas};
use super::search::{Index, Match};
use super::theme::Theme;
use crate::config::Config;
use crate::config::edit::Edit;
use crate::render::{Color, FitMode};

/// The whole TUI state.
#[derive(Debug)]
pub struct Model {
    /// Which keys mean what right now.
    pub mode: Mode,
    /// Every key binding, Config remaps applied.
    pub keymap: Keymap,
    /// The Library folders, from the Config.
    pub folders: Vec<PathBuf>,
    /// Every wallpaper in the Library, sorted by path. Empty while
    /// `scanning`.
    pub library: Vec<PathBuf>,
    /// The Library's names, for fuzzy search.
    pub index: Index,
    /// The wallpapers the list shows, in order: the Library narrowed by
    /// `query` and `favorites_only`. Each is an index into `library` plus
    /// the characters the query matched.
    pub visible: Vec<Match>,
    /// The search query. Empty: no search.
    pub query: String,
    /// The user's Favorites as the TUI wants them (a save may still be in
    /// flight).
    pub favorites: Vec<PathBuf>,
    /// Show only Favorites.
    pub favorites_only: bool,
    /// True until the first Library scan reports back.
    pub scanning: bool,
    /// Index into `visible` of the highlighted wallpaper.
    pub selected: usize,
    /// Index into `visible` of the first row the list shows. Update keeps
    /// the selection in view.
    pub list_offset: usize,
    /// The help overlay's or the palette's query and cursor.
    pub prompt: Prompt,
    /// Keys typed so far of a longer binding (`g` of `g g`), and when the
    /// first one was pressed.
    pub pending: Vec<Key>,
    pub pending_since: Duration,
    /// What the Daemon told us, or why it could not.
    pub daemon: Daemon,
    /// Index into the Daemon's Monitors of the one Enter applies to.
    pub target: usize,
    /// The Daemon's last Status, for the Rotation in the header.
    pub status: Option<Status>,
    /// Rotation on the targeted Monitor, as shown in the header.
    pub rotation: Rotation,
    /// The Rotation schedule as the Config has it (what the panel edits),
    /// with edits not yet saved already applied.
    pub schedules: RotationSettings,
    /// The Rotation panel's cursor and scope.
    pub panel: Panel,
    /// Config edits waiting for the user to stop changing things.
    pub config_edits: Vec<Edit>,
    /// Bumped by every Config edit; a save of an older one does not
    /// overwrite what the panel shows.
    pub config_generation: u64,
    /// The welcome screen's choice and typed folder.
    pub welcome: Welcome,
    /// On a narrow terminal: the preview is shown instead of the list.
    pub preview_only: bool,
    /// The Fit mode the preview and Enter use.
    pub fit: FitMode,
    /// The letterbox color the preview and Enter use.
    pub letterbox: Color,
    pub preview: Preview,
    /// Applies in flight, oldest first, for the spinner.
    pub applying: Vec<Busy>,
    /// The thumbnail grid, shown instead of the list and preview when on.
    pub grid: Grid,
    /// The latest confirmation or error, shown in the footer.
    pub toast: Option<Toast>,
    /// When `toast` goes away.
    pub toast_until: Duration,
    /// The last click on a list row: (index into `visible`, when), to spot
    /// a double-click.
    pub last_click: Option<(usize, Duration)>,
    /// Picks `r`'s wallpaper. Seeded in tests.
    pub rng: fastrand::Rng,
    /// Time since the TUI started. The runtime sets it before every Msg
    /// and tests set it by hand, so update never reads a clock.
    pub now: Duration,
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
            keymap: Keymap::new(&config.keymap),
            folders: config.library.clone(),
            library: Vec::new(),
            index: Index::default(),
            visible: Vec::new(),
            query: String::new(),
            favorites: Vec::new(),
            favorites_only: false,
            scanning: true,
            selected: 0,
            list_offset: 0,
            prompt: Prompt::default(),
            pending: Vec::new(),
            pending_since: Duration::ZERO,
            daemon: Daemon::Connecting,
            target: 0,
            status: None,
            rotation: Rotation::Off,
            schedules: config.rotation.clone(),
            panel: Panel::default(),
            config_edits: Vec::new(),
            config_generation: 0,
            welcome: Welcome::default(),
            preview_only: false,
            fit: config.fit,
            letterbox: config.letterbox,
            preview: Preview::default(),
            applying: Vec::new(),
            grid: Grid::default(),
            toast: None,
            toast_until: Duration::ZERO,
            last_click: None,
            rng: fastrand::Rng::new(),
            now: Duration::ZERO,
            area,
            home,
            theme,
            quit: false,
            dirty: true,
        }
    }

    /// Opens the welcome screen, for a first run with no Library folder.
    pub fn first_run(&mut self) {
        self.mode = Mode::Welcome;
    }

    /// Where everything goes on screen now.
    pub fn areas(&self) -> Areas {
        layout::areas(self.area, self.preview_only)
    }

    /// What the footer depends on besides the mode.
    pub fn footer_context(&self) -> keymap::Context {
        keymap::Context {
            filtered: self.filtered(),
            grid: self.grid.on,
            narrow: layout::is_narrow(self.area),
            preview_only: self.preview_only,
            daemon_down: matches!(self.daemon, Daemon::Down(_)),
            overriding: self.panel.monitor.is_some(),
        }
    }

    /// The highlighted wallpaper, if the list shows any.
    pub fn selection(&self) -> Option<&PathBuf> {
        self.visible
            .get(self.selected)
            .and_then(|m| self.library.get(m.index))
    }

    pub fn is_favorite(&self, path: &Path) -> bool {
        self.favorites.iter().any(|f| f == path)
    }

    /// Whether the list is narrowed by a search or the Favorites filter.
    pub fn filtered(&self) -> bool {
        !self.query.is_empty() || self.favorites_only
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

    /// `path` relative to the Library folder holding it:
    /// `beach/dunes.jpg`.
    pub fn short_name(&self, path: &Path) -> String {
        self.folders
            .iter()
            .find_map(|folder| path.strip_prefix(folder).ok())
            .unwrap_or(path)
            .display()
            .to_string()
    }
}

/// Which keymap is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Browsing the list.
    Normal,
    /// Typing a search; the list filters on every key.
    Search,
    /// The `?` overlay.
    Help,
    /// The command palette.
    Palette,
    /// The Rotation panel (`s`).
    Rotation,
    /// The welcome screen: choose a Library folder.
    Welcome,
}

/// The Rotation panel's state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Panel {
    /// The highlighted setting, an index into `update::panel_fields`.
    pub row: usize,
    /// `None` edits the global Rotation; a name edits that Monitor's
    /// Override.
    pub monitor: Option<String>,
}

/// One line of the Rotation panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// Running or stopped.
    Running,
    /// Every Monitor, or one Monitor's Override.
    Scope,
    Interval,
    Source,
    Order,
}

/// The welcome screen's state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Welcome {
    /// 0: `~/Pictures`; 1: the typed folder.
    pub choice: usize,
    /// The folder typed so far.
    pub input: String,
    /// Folders the last Tab could complete to, when there were several.
    pub candidates: Vec<String>,
    /// Why the last choice was refused.
    pub error: Option<String>,
    /// A save is in flight.
    pub saving: bool,
}

/// The query and cursor of the help overlay or the palette.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prompt {
    pub query: String,
    /// The highlighted palette row, or the first help row shown.
    pub selected: usize,
}

/// Slow work in flight. It gets a spinner once it has taken
/// [`super::update::SPINNER_DELAY`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Busy {
    pub label: String,
    pub since: Duration,
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
    /// Running on the targeted Monitor, changing every `interval_ms`.
    On {
        interval_ms: u64,
        source: RotationSource,
        /// The targeted Monitor follows its own Override.
        overridden: bool,
    },
}

impl Rotation {
    /// Rotation as `status` reports it for `monitor`: its Override, else
    /// the global schedule.
    pub fn from_status(status: &Status, monitor: Option<&str>) -> Rotation {
        if !status.rotation_running {
            return Rotation::Off;
        }
        let rotation = &status.settings.rotation;
        let schedule = monitor.map_or(&rotation.schedule, |m| rotation.schedule_for(m));
        Rotation::On {
            interval_ms: schedule.interval_ms,
            source: schedule.source,
            overridden: monitor.is_some_and(|m| rotation.overrides.iter().any(|o| o.monitor == m)),
        }
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
        /// When it was asked for, for the spinner.
        since: Duration,
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

/// The thumbnail grid. Only the tiles on screen are rendered.
#[derive(Debug, Default)]
pub struct Grid {
    /// Whether the grid replaces the list and preview. `g` toggles it; it
    /// starts on where the terminal speaks the kitty graphics protocol.
    pub on: bool,
    /// The first row of tiles on screen.
    pub offset_row: usize,
    /// Bumped whenever the tiles on screen change. Thumbnails from older
    /// requests are dropped.
    pub generation: u64,
    /// What the tiles were rendered for. A change clears `tiles`.
    pub spec: Option<TileSpec>,
    /// Thumbnails for the tiles on screen and about a page either side.
    pub tiles: HashMap<PathBuf, Tile>,
}

/// Everything a thumbnail depends on besides its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileSpec {
    pub cells: Size,
    pub monitor: super::msg::Screen,
    pub fit: FitMode,
    pub letterbox: Color,
}

/// One tile's thumbnail.
#[derive(Debug)]
pub enum Tile {
    Loading,
    Ready { image: PreviewImage, meta: Meta },
    Failed,
}

/// Facts about a wallpaper file, for the metadata strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

/// A preview ready to draw. Cloning shares the encoded image.
#[derive(Clone)]
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
            PreviewState::Loading { path, .. }
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

/// A short message in the footer. It goes away on its own.
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
