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
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use vellum_ipc::{MonitorInfo, MonitorStatus, Settings, Status};

use super::model::{Meta, Model, PreviewImage};
use super::msg::{ApplyJob, Cmd, DaemonInfo, Msg, PreviewJob, PreviewOutput};
use super::services::{Services, perform};
use super::theme::Theme;
use super::{update, view};
use crate::config::Config;
use crate::config::Named as _;

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
                })
                .collect(),
            settings: Settings::default(),
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

    fn preview(&self, job: &PreviewJob) -> Result<PreviewOutput, String> {
        if self.broken.contains(&job.path) {
            return Err("cannot decode: invalid PNG signature".into());
        }
        let (w, h) = super::preview::frame_size(job.cells, FONT.into(), job.monitor);
        let size = super::preview::cells_for((w, h), FONT.into());
        let name = job.path.file_name().unwrap().to_string_lossy();
        Ok(PreviewOutput {
            image: PreviewImage::Placeholder {
                size,
                label: format!("{name} · {}", job.fit.name()),
            },
            meta: Meta {
                width: 3840,
                height: 2160,
                bytes: 2_516_582,
            },
        })
    }

    fn apply(&self, job: &ApplyJob) -> Result<(), String> {
        self.applied.lock().unwrap().push(job.clone());
        match &self.apply_error {
            Some(err) => Err(err.clone()),
            None => Ok(()),
        }
    }
}

pub struct Harness {
    pub model: Model,
    pub fake: Fake,
}

impl Harness {
    /// A fresh Model in a `width`×`height` terminal with the default
    /// Config and Midnight Bloom. Nothing has run yet.
    pub fn new(width: u16, height: u16) -> Harness {
        Harness::with(width, height, Fake::default())
    }

    pub fn with(width: u16, height: u16, fake: Fake) -> Harness {
        let home = PathBuf::from(HOME);
        let config = Config::defaults(Some(&home));
        let model = Model::new(
            &config,
            Theme::midnight_bloom(),
            Rect::new(0, 0, width, height),
            Some(home),
        );
        Harness { model, fake }
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
        update::update(&mut self.model, msg)
    }

    /// Update, then run every resulting Cmd (and theirs) against the fake.
    pub fn dispatch(&mut self, msg: Msg) {
        let cmds = self.update(msg);
        self.run(cmds);
    }

    pub fn run(&mut self, cmds: Vec<Cmd>) {
        let mut queue = std::collections::VecDeque::from(cmds);
        while let Some(cmd) = queue.pop_front() {
            let msg = perform(&self.fake, cmd);
            queue.extend(self.update(msg));
        }
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

/// A key from a name: "j", "enter", "esc", "down", "ctrl-c".
pub fn key(name: &str) -> KeyEvent {
    let (mods, name) = match name.strip_prefix("ctrl-") {
        Some(rest) => (KeyModifiers::CONTROL, rest),
        None => (KeyModifiers::NONE, name),
    };
    let code = match name {
        "enter" => KeyCode::Enter,
        "esc" => KeyCode::Esc,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "tab" => KeyCode::Tab,
        c if c.chars().count() == 1 => KeyCode::Char(c.chars().next().unwrap()),
        other => panic!("unknown key {other}"),
    };
    KeyEvent::new(code, mods)
}
