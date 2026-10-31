//! Test harness: the real update and view, a fake Daemon, a fake Library
//! and a placeholder image protocol. No terminal, no threads.
//!
//! ```ignore
//! let mut h = Harness::started(80, 24);   // init Cmds already run
//! h.press("j");                           // update + run its Cmds
//! insta::assert_snapshot!(h.screen());
//! let cmds = h.update(Msg::Key(key("enter"))); // update only
//! ```

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use vellum_ipc::{MonitorInfo, MonitorStatus, Settings, Status};

use crate::config::edit::{self, Edit, Value};

use super::model::{Meta, Model, PreviewImage};
use ratatui::layout::Size;

use super::msg::{ApplyJob, Cmd, DaemonInfo, Msg, PreviewJob, PreviewOutput, Purpose, Screen};
use super::services::{Latest, Services, perform};
use super::theme::Theme;
use super::{update, view};
use crate::config::Config;
use crate::config::Named as _;
use crate::render::FitMode;

pub const HOME: &str = "/home/me";

/// A wallpaper path in the default Library folder.
pub fn wallpaper(rel: &str) -> PathBuf {
    Path::new(HOME).join("Pictures/Wallpapers").join(rel)
}

pub fn monitor(name: &str, width: u32, height: u32, scale_120: u32) -> MonitorInfo {
    MonitorInfo {
        name: name.into(),
        logical_width: width * 120 / scale_120,
        logical_height: height * 120 / scale_120,
        physical_width: width,
        physical_height: height,
        scale_120,
    }
}

/// The Daemon, Library and files a test sees.
pub struct Fake {
    pub library: Vec<PathBuf>,
    pub daemon: Result<DaemonInfo, String>,
    /// Files whose preview fails to decode.
    pub broken: Vec<PathBuf>,
    /// If set, every Apply fails with this.
    pub apply_error: Option<String>,
    /// Every Apply the runtime would have sent.
    pub applied: Mutex<Vec<ApplyJob>>,
    /// The Favorites file.
    pub favorites: Mutex<Vec<PathBuf>>,
    /// If set, saving a Favorite fails with this.
    pub favorite_error: Option<String>,
    /// Every preview job that was rendered, not dropped as stale.
    pub rendered: Mutex<Vec<PreviewJob>>,
    /// The Config file's text; Config edits really apply to it.
    pub config_text: Mutex<String>,
    /// Every batch of Config edits saved.
    pub config_saves: Mutex<Vec<Vec<Edit>>>,
    /// If set, every Config write fails with this.
    pub config_error: Option<String>,
    /// The Daemon's Rotation: running, and the settings it holds.
    pub rotation_running: Mutex<bool>,
    pub settings: Mutex<Settings>,
    /// Folders Tab completion knows about, as typed (`~/Pictures/`).
    pub folders: Vec<String>,
    /// Folders the welcome screen may not choose (they do not exist).
    pub missing: Vec<PathBuf>,
}

impl Default for Fake {
    fn default() -> Fake {
        Fake {
            library: vec![
                wallpaper("aurora.png"),
                wallpaper("beach/dunes.jpg"),
                wallpaper("beach/tide.webp"),
                wallpaper("city/neon-rain.jpg"),
            ],
            daemon: Ok(daemon_info(vec![
                monitor("DP-1", 2560, 1440, 120),
                monitor("HDMI-A-1", 1920, 1080, 120),
            ])),
            broken: Vec::new(),
            apply_error: None,
            applied: Mutex::new(Vec::new()),
            favorites: Mutex::new(Vec::new()),
            favorite_error: None,
            rendered: Mutex::new(Vec::new()),
            config_text: Mutex::new(String::new()),
            config_saves: Mutex::new(Vec::new()),
            config_error: None,
            rotation_running: Mutex::new(false),
            settings: Mutex::new(Settings::default()),
            folders: vec![
                "~/Pictures/".into(),
                "~/Pictures-old/".into(),
                "~/Wallpapers/".into(),
            ],
            missing: Vec::new(),
        }
    }
}

impl Fake {
    /// The Daemon's Status now.
    fn status(&self) -> Result<Status, String> {
        let mut status = self.daemon.clone()?.status;
        status.rotation_running = *self.rotation_running.lock().unwrap();
        status.settings = self.settings.lock().unwrap().clone();
        Ok(status)
    }

    /// The Config the fake file holds.
    pub fn config(&self) -> crate::config::Config {
        config(&self.config_text.lock().unwrap())
    }

    /// The [`Msg::PreviewDone`] a worker that took preview `generation`
    /// would send for `path` if it never checked the generation.
    pub fn preview_done(&self, generation: u64, path: PathBuf) -> Msg {
        let job = PreviewJob {
            generation,
            purpose: Purpose::Preview,
            path: path.clone(),
            cells: Size::new(40, 10),
            monitor: Screen::FALLBACK,
            fit: FitMode::Fill,
            letterbox: [0; 3],
        };
        Msg::PreviewDone {
            generation,
            path,
            result: self.preview(&job, &|| false).expect("never stale"),
        }
    }
}

pub fn daemon_info(monitors: Vec<MonitorInfo>) -> DaemonInfo {
    DaemonInfo {
        status: Status {
            version: "2.0.0".into(),
            pid: 4242,
            rss_bytes: 2 << 20,
            monitors: monitors
                .iter()
                .map(|m| MonitorStatus {
                    name: m.name.clone(),
                    wallpaper: None,
                    next_change_ms: None,
                })
                .collect(),
            settings: Settings::default(),
            rotation_running: false,
        },
        monitors,
    }
}

/// The font size the placeholder protocol pretends the terminal has.
const FONT: (u16, u16) = (10, 20);

impl Services for Fake {
    fn scan(&self, _folders: &[PathBuf]) -> Vec<PathBuf> {
        self.library.clone()
    }

    fn daemon(&self) -> Result<DaemonInfo, String> {
        self.daemon.clone()
    }

    fn preview(
        &self,
        job: &PreviewJob,
        stale: &dyn Fn() -> bool,
    ) -> Option<Result<PreviewOutput, String>> {
        if stale() {
            return None;
        }
        self.rendered.lock().unwrap().push(job.clone());
        if self.broken.contains(&job.path) {
            return Some(Err("cannot decode: invalid PNG signature".into()));
        }
        let (w, h) = super::preview::frame_size(job.cells, FONT.into(), job.monitor);
        let size = super::preview::cells_for((w, h), FONT.into());
        let name = job.path.file_name().unwrap().to_string_lossy();
        Some(Ok(PreviewOutput {
            image: PreviewImage::Placeholder {
                size,
                label: format!("{name} · {}", job.fit.name()),
            },
            meta: Meta {
                width: 3840,
                height: 2160,
                bytes: 2_516_582,
            },
        }))
    }

    fn apply(&self, job: &ApplyJob) -> Result<(), String> {
        self.applied.lock().unwrap().push(job.clone());
        match &self.apply_error {
            Some(err) => Err(err.clone()),
            None => Ok(()),
        }
    }

    fn favorites(&self) -> Vec<PathBuf> {
        self.favorites.lock().unwrap().clone()
    }

    fn save_favorite(&self, path: &Path, favorite: bool) -> Result<(), String> {
        if let Some(err) = &self.favorite_error {
            return Err(err.clone());
        }
        let mut favorites = self.favorites.lock().unwrap();
        favorites.retain(|f| f != path);
        if favorite {
            favorites.push(path.to_owned());
        }
        Ok(())
    }

    fn save_config(&self, edits: &[Edit]) -> Result<crate::config::Config, String> {
        if let Some(err) = &self.config_error {
            return Err(err.clone());
        }
        self.config_saves.lock().unwrap().push(edits.to_vec());
        let mut text = self.config_text.lock().unwrap();
        *text = edit::apply(&text, edits)?;
        Ok(config(&text))
    }

    fn push_settings(&self, settings: &Settings) -> Result<Status, String> {
        *self.settings.lock().unwrap() = settings.clone();
        self.status()
    }

    fn rotation(&self, start: bool) -> Result<Status, String> {
        self.daemon.clone()?;
        *self.rotation_running.lock().unwrap() = start;
        self.status()
    }

    fn complete(&self, input: &str) -> Vec<String> {
        let input = if input.is_empty() { "~/" } else { input };
        self.folders
            .iter()
            .filter(|f| f.starts_with(input))
            .cloned()
            .collect()
    }

    fn save_library(&self, folder: &Path) -> Result<crate::config::Config, String> {
        let shown = edit::tilde(folder, Some(Path::new(HOME)));
        if self.missing.iter().any(|m| m == folder) {
            return Err(format!("{shown} is not a folder"));
        }
        self.save_config(&[Edit::Set {
            table: Vec::new(),
            key: "library".into(),
            value: Value::List(vec![shown]),
        }])
    }
}

pub struct Harness {
    pub model: Model,
    pub fake: Fake,
    /// The generations workers see, published after every update like the
    /// runtime does.
    pub latest: Latest,
}

impl Harness {
    /// A fresh Model in a `width`×`height` terminal with the default
    /// Config and Midnight Bloom. Nothing has run yet.
    pub fn new(width: u16, height: u16) -> Harness {
        Harness::with(width, height, Fake::default())
    }

    pub fn with(width: u16, height: u16, fake: Fake) -> Harness {
        Harness::with_config(width, height, fake, config(""))
    }

    pub fn with_config(width: u16, height: u16, fake: Fake, config: Config) -> Harness {
        let mut model = Model::new(
            &config,
            Theme::midnight_bloom(),
            Rect::new(0, 0, width, height),
            Some(PathBuf::from(HOME)),
        );
        model.rng = fastrand::Rng::with_seed(7);
        Harness {
            model,
            fake,
            latest: Latest::default(),
        }
    }

    /// Opened on a first run (no Library folder): the welcome screen.
    pub fn first_run(width: u16, height: u16) -> Harness {
        let fake = Fake {
            library: Vec::new(),
            ..Fake::default()
        };
        let mut h = Harness::with(width, height, fake);
        h.model.first_run();
        let cmds = h.start();
        h.run(cmds);
        h
    }

    /// Started with a Config parsed from `toml`.
    pub fn started_with_config(width: u16, height: u16, toml: &str) -> Harness {
        let mut h = Harness::with_config(width, height, Fake::default(), config(toml));
        let cmds = h.start();
        h.run(cmds);
        h
    }

    /// Moves the clock on by `ms` and delivers the Tick the runtime would.
    pub fn advance(&mut self, ms: u64) {
        self.model.now += std::time::Duration::from_millis(ms);
        self.dispatch(Msg::Tick);
    }

    /// Presses `g` and lets it time out (it also starts `g g`), so the grid
    /// toggles. The Cmds come back unrun.
    pub fn toggle_grid(&mut self) -> Vec<Cmd> {
        let mut cmds = self.update(Msg::Key(key("g")));
        self.model.now += super::keymap::CHORD_TIMEOUT;
        cmds.extend(self.update(Msg::Tick));
        cmds
    }

    /// Presses each character of `text` in turn.
    pub fn typed(&mut self, text: &str) {
        for c in text.chars() {
            self.press(&c.to_string());
        }
    }

    /// A left click at column `x`, row `y`.
    pub fn click(&mut self, x: u16, y: u16) {
        self.mouse(MouseEventKind::Down(MouseButton::Left), x, y);
    }

    /// One wheel notch at column `x`, row `y`.
    pub fn wheel(&mut self, down: bool, x: u16, y: u16) {
        let kind = match down {
            true => MouseEventKind::ScrollDown,
            false => MouseEventKind::ScrollUp,
        };
        self.mouse(kind, x, y);
    }

    fn mouse(&mut self, kind: MouseEventKind, column: u16, row: u16) {
        self.dispatch(Msg::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }));
    }

    /// Where `text` first appears on screen: (column, row).
    pub fn find(&self, text: &str) -> Option<(u16, u16)> {
        let buffer = self.buffer();
        (0..buffer.area.height).find_map(|y| {
            let row: String = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            row.find(text)
                .map(|byte| (u16::try_from(row[..byte].chars().count()).unwrap(), y))
        })
    }

    /// The init Cmds, not run.
    pub fn start(&mut self) -> Vec<Cmd> {
        update::init(&self.model)
    }

    /// Started, with every Cmd run to completion.
    pub fn started(width: u16, height: u16) -> Harness {
        Harness::started_with(width, height, Fake::default())
    }

    pub fn started_with(width: u16, height: u16, fake: Fake) -> Harness {
        let mut h = Harness::with(width, height, fake);
        let cmds = h.start();
        h.run(cmds);
        h
    }

    /// Update only: the Cmds come back unrun.
    pub fn update(&mut self, msg: Msg) -> Vec<Cmd> {
        let cmds = update::update(&mut self.model, msg);
        self.latest.publish(&self.model);
        cmds
    }

    /// Update, then run every resulting Cmd (and theirs) against the fake.
    pub fn dispatch(&mut self, msg: Msg) {
        let cmds = self.update(msg);
        self.run(cmds);
    }

    /// Runs `cmds` (and the Cmds their Msgs cause) to completion, in
    /// order. Debounce timers fire at once.
    pub fn run(&mut self, cmds: Vec<Cmd>) {
        let mut queue = std::collections::VecDeque::from(cmds);
        while let Some(cmd) = queue.pop_front() {
            if let Some(msg) = self.perform(cmd) {
                queue.extend(self.update(msg));
            }
        }
    }

    /// What a worker would send back for `cmd`, given the generations now.
    pub fn perform(&self, cmd: Cmd) -> Option<Msg> {
        perform(&self.fake, cmd, &self.latest)
    }

    pub fn press(&mut self, keys: &str) {
        self.dispatch(Msg::Key(key(keys)));
    }

    /// The screen as the view draws it.
    pub fn buffer(&self) -> Buffer {
        let area = self.model.area;
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal.draw(|f| view::view(&self.model, f)).unwrap();
        terminal.backend().buffer().clone()
    }

    /// The screen as text, for snapshots.
    pub fn screen(&self) -> String {
        let buffer = self.buffer();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            let mut line = String::new();
            for x in 0..buffer.area.width {
                line.push_str(buffer[(x, y)].symbol());
            }
            out += line.trim_end();
            out.push('\n');
        }
        out
    }
}

/// A Config parsed from `toml`, with `~` at [`HOME`].
pub fn config(toml: &str) -> Config {
    Config::parse(toml, Some(Path::new(HOME))).unwrap()
}

/// A key from a name: "j", "G", "enter", "esc", "down", "ctrl-c". Like a
/// terminal, uppercase letters and symbols come with Shift.
pub fn key(name: &str) -> KeyEvent {
    let (mut mods, name) = match name.strip_prefix("ctrl-") {
        Some(rest) => (KeyModifiers::CONTROL, rest),
        None => (KeyModifiers::NONE, name),
    };
    let code = match name {
        "enter" => KeyCode::Enter,
        "esc" => KeyCode::Esc,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "delete" => KeyCode::Delete,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pagedown" => KeyCode::PageDown,
        "backspace" => KeyCode::Backspace,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Char(' '),
        c if c.chars().count() == 1 => {
            let c = c.chars().next().unwrap();
            if c.is_ascii_uppercase() || "?:".contains(c) {
                mods |= KeyModifiers::SHIFT;
            }
            KeyCode::Char(c)
        }
        f if f.starts_with('f') && f[1..].parse::<u8>().is_ok() => {
            KeyCode::F(f[1..].parse().unwrap())
        }
        other => panic!("unknown key {other}"),
    };
    KeyEvent::new(code, mods)
}
