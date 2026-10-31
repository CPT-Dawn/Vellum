//! Requests and responses.
//!
//! Message kinds are grouped by request family in the high byte, so new
//! messages slot in without renumbering:
//!
//! | range    | family                                   |
//! |----------|------------------------------------------|
//! | `0x01xx` | Query (monitors, status, version)        |
//! | `0x02xx` | Present (frames)                         |
//! | `0x03xx` | Settings push                            |
//! | `0x04xx` | Rotation control                         |
//! | `0x05xx` | Lifecycle (reload, kill)                 |
//! | `0x80xx` | Responses                                |

use crate::Error;
use crate::codec::{self, Body, Reader, Writer};

mod kind {
    pub const MONITORS: u16 = 0x0101;
    pub const STATUS: u16 = 0x0102;
    pub const PRESENT: u16 = 0x0201;
    pub const SETTINGS: u16 = 0x0301;
    pub const KILL: u16 = 0x0502;

    pub const OK: u16 = 0x8000;
    pub const ERROR: u16 = 0x8001;
    pub const MONITORS_REPLY: u16 = 0x8101;
    pub const STATUS_REPLY: u16 = 0x8102;
}

/// A message from `vellum` to the Daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// List the Monitors the Daemon can draw on. Answered by
    /// [`Response::Monitors`].
    Monitors,
    /// Show a frame on a Monitor. The frame's pixels travel as exactly one
    /// memfd alongside the request. Answered by [`Response::Ok`] once the
    /// frame is committed.
    Present(Present),
    /// What the Daemon shows and remembers. Answered by [`Response::Status`].
    Status,
    /// Stop the Daemon. Answered by [`Response::Ok`] just before it exits.
    Kill,
    /// The settings `vellum render --sync` read from a valid Config. They
    /// replace the previous ones whole. Answered by [`Response::Ok`].
    Settings(Settings),
}

/// The Config values the Daemon holds, pushed by `vellum render --sync`
/// whenever the Config changes. The Daemon never reads the Config itself
/// (ADR 0003); it keeps the last good values it was sent and reports them
/// in [`Status`]. Until the first push, every field has its default.
///
/// To add a setting: add a field here, write and read it at the end of
/// [`write_settings`] and [`read_settings`], and bump `PROTOCOL_VERSION`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    /// The Transition `vellum set` plays when no flag overrides it.
    pub transition: Transition,
}

/// A frame for one Monitor. The pixels are in the accompanying memfd at
/// offset 0, `stride * height` bytes, sealed against shrinking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Present {
    /// Connector name of the target Monitor, e.g. `DP-1`.
    pub monitor: String,
    /// Width in physical pixels. Must equal the Monitor's physical width.
    pub width: u32,
    /// Height in physical pixels. Must equal the Monitor's physical height.
    pub height: u32,
    /// Bytes per row.
    pub stride: u32,
    pub format: PixelFormat,
    /// The wallpaper file the frame was rendered from, recorded in State.
    pub source: String,
    /// How the frame replaces what the Monitor shows now.
    pub transition: Transition,
    /// The Fit mode the frame was rendered with, recorded in State.
    pub fit: FitMode,
    /// Sent by `vellum render --restore`. The Daemon drops a restore frame
    /// when the Monitor already shows an up-to-date one, so a restore never
    /// overwrites a newer `vellum set`.
    pub restore: bool,
}

/// The Transition that plays while a presented frame replaces the current
/// wallpaper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    pub kind: TransitionKind,
    /// Which way a Wipe's edge or a Slide's frame moves. Fade and None
    /// ignore it.
    pub direction: Direction,
    /// Total length in milliseconds. 0 behaves like [`TransitionKind::None`].
    pub duration_ms: u32,
    pub easing: Easing,
}

impl Default for Transition {
    /// Fade, 600 ms, ease-out.
    fn default() -> Self {
        Transition {
            kind: TransitionKind::Fade,
            direction: Direction::Right,
            duration_ms: 600,
            easing: Easing::EaseOut,
        }
    }
}

/// The Transition's visual effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TransitionKind {
    /// An immediate swap.
    None = 0,
    /// The new frame fades in over the old one.
    Fade = 1,
    /// A moving edge reveals the new frame in place.
    Wipe = 2,
    /// The new frame slides in over the old one.
    Slide = 3,
}

/// The direction a Wipe's edge or a Slide's frame travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Direction {
    Left = 0,
    Right = 1,
    Up = 2,
    Down = 3,
}

/// The timing curve of a Transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Easing {
    Linear = 0,
    EaseIn = 1,
    EaseOut = 2,
    EaseInOut = 3,
}

impl Easing {
    /// Maps linear progress `t` in `0..=1` onto the curve (cubic).
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::EaseIn => t * t * t,
            Easing::EaseOut => 1.0 - (1.0 - t).powi(3),
            Easing::EaseInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
        }
    }
}

/// How a wallpaper is sized to a Monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FitMode {
    /// Cover the Monitor and crop the overflow.
    #[default]
    Fill,
    /// Letterbox: the whole wallpaper, bars around it.
    Fit,
    /// Actual size, centred.
    Center,
    /// Ignore the aspect ratio.
    Stretch,
}

impl FitMode {
    const ALL: [FitMode; 4] = [
        FitMode::Fill,
        FitMode::Fit,
        FitMode::Center,
        FitMode::Stretch,
    ];

    /// The lowercase name used on the command line and in State.
    pub fn name(self) -> &'static str {
        match self {
            FitMode::Fill => "fill",
            FitMode::Fit => "fit",
            FitMode::Center => "center",
            FitMode::Stretch => "stretch",
        }
    }

    /// The Fit mode called `name`, if any.
    pub fn from_name(name: &str) -> Option<FitMode> {
        FitMode::ALL.into_iter().find(|fit| fit.name() == name)
    }

    fn code(self) -> u8 {
        self as u8
    }

    fn from_code(code: u8) -> Result<FitMode, Error> {
        FitMode::ALL
            .get(usize::from(code))
            .copied()
            .ok_or(Error::BadValue("fit mode"))
    }
}

/// What the Daemon shows and remembers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// The Daemon's release, e.g. `2.0.0`.
    pub version: String,
    pub pid: u32,
    /// The Daemon's resident memory in bytes.
    pub rss_bytes: u64,
    /// Every connected Monitor, sorted by name.
    pub monitors: Vec<MonitorStatus>,
    /// The settings in effect: the last good Config pushed, or defaults.
    pub settings: Settings,
}

/// One connected Monitor in a [`Status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorStatus {
    pub name: String,
    /// The wallpaper State holds for it: the one on screen, or the one a
    /// restore is about to put there. `None` if Vellum has nothing to show.
    pub wallpaper: Option<Wallpaper>,
}

/// A wallpaper file and the Fit mode it is shown with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wallpaper {
    pub path: String,
    pub fit: FitMode,
}

/// Pixel layout of a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PixelFormat {
    /// 32 bits per pixel, `[31:0] x:R:G:B` little-endian, so the bytes in
    /// memory are B, G, R, X. Every compositor supports it.
    Xrgb8888 = 1,
}

impl PixelFormat {
    /// Bytes per pixel.
    pub fn bytes_per_pixel(self) -> u32 {
        match self {
            PixelFormat::Xrgb8888 => 4,
        }
    }
}

/// A Monitor as the Daemon sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorInfo {
    /// Connector name, e.g. `DP-1`.
    pub name: String,
    pub logical_width: u32,
    pub logical_height: u32,
    /// The size a frame for this Monitor must have.
    pub physical_width: u32,
    pub physical_height: u32,
    /// Scale in 120ths (120 = 1x, 180 = 1.5x), as in fractional-scale-v1.
    pub scale_120: u32,
}

/// The Daemon's answer to a [`Request`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Ok,
    /// The request failed. The text is meant for the user.
    Error(String),
    Monitors(Vec<MonitorInfo>),
    Status(Status),
}

impl Request {
    /// Encodes the request as one complete frame.
    pub fn encode(&self) -> Vec<u8> {
        codec::encode(self)
    }

    /// Decodes one complete frame.
    pub fn decode(frame: &[u8]) -> Result<Self, Error> {
        codec::decode(frame)
    }
}

impl Response {
    /// Encodes the response as one complete frame.
    pub fn encode(&self) -> Vec<u8> {
        codec::encode(self)
    }

    /// Decodes one complete frame.
    pub fn decode(frame: &[u8]) -> Result<Self, Error> {
        codec::decode(frame)
    }
}

impl Body for Request {
    fn kind(&self) -> u16 {
        match self {
            Request::Monitors => kind::MONITORS,
            Request::Present(_) => kind::PRESENT,
            Request::Status => kind::STATUS,
            Request::Kill => kind::KILL,
            Request::Settings(_) => kind::SETTINGS,
        }
    }

    fn write(&self, w: &mut Writer) {
        match self {
            Request::Monitors | Request::Status | Request::Kill => {}
            Request::Present(p) => {
                w.str(&p.monitor);
                w.u32(p.width);
                w.u32(p.height);
                w.u32(p.stride);
                w.u32(p.format as u32);
                w.str(&p.source);
                write_transition(w, &p.transition);
                w.u8(p.fit.code());
                w.u8(u8::from(p.restore));
            }
            Request::Settings(settings) => write_settings(w, settings),
        }
    }

    fn read(kind: u16, r: &mut Reader<'_>) -> Result<Self, Error> {
        Ok(match kind {
            kind::MONITORS => Request::Monitors,
            kind::PRESENT => Request::Present(Present {
                monitor: r.string()?,
                width: r.u32()?,
                height: r.u32()?,
                stride: r.u32()?,
                format: match r.u32()? {
                    1 => PixelFormat::Xrgb8888,
                    _ => return Err(Error::BadValue("pixel format")),
                },
                source: r.string()?,
                transition: read_transition(r)?,
                fit: FitMode::from_code(r.u8()?)?,
                restore: r.u8()? != 0,
            }),
            kind::STATUS => Request::Status,
            kind::KILL => Request::Kill,
            kind::SETTINGS => Request::Settings(read_settings(r)?),
            other => return Err(Error::UnknownKind(other)),
        })
    }
}

fn write_transition(w: &mut Writer, t: &Transition) {
    w.u8(t.kind as u8);
    w.u8(t.direction as u8);
    w.u8(t.easing as u8);
    w.u32(t.duration_ms);
}

fn write_settings(w: &mut Writer, settings: &Settings) {
    write_transition(w, &settings.transition);
}

fn read_settings(r: &mut Reader<'_>) -> Result<Settings, Error> {
    Ok(Settings {
        transition: read_transition(r)?,
    })
}

fn read_transition(r: &mut Reader<'_>) -> Result<Transition, Error> {
    let kind = match r.u8()? {
        0 => TransitionKind::None,
        1 => TransitionKind::Fade,
        2 => TransitionKind::Wipe,
        3 => TransitionKind::Slide,
        _ => return Err(Error::BadValue("transition kind")),
    };
    let direction = match r.u8()? {
        0 => Direction::Left,
        1 => Direction::Right,
        2 => Direction::Up,
        3 => Direction::Down,
        _ => return Err(Error::BadValue("transition direction")),
    };
    let easing = match r.u8()? {
        0 => Easing::Linear,
        1 => Easing::EaseIn,
        2 => Easing::EaseOut,
        3 => Easing::EaseInOut,
        _ => return Err(Error::BadValue("transition easing")),
    };
    Ok(Transition {
        kind,
        direction,
        duration_ms: r.u32()?,
        easing,
    })
}

impl Body for Response {
    fn kind(&self) -> u16 {
        match self {
            Response::Ok => kind::OK,
            Response::Error(_) => kind::ERROR,
            Response::Monitors(_) => kind::MONITORS_REPLY,
            Response::Status(_) => kind::STATUS_REPLY,
        }
    }

    fn write(&self, w: &mut Writer) {
        match self {
            Response::Ok => {}
            Response::Error(message) => w.str(message),
            Response::Monitors(monitors) => {
                w.u16(monitors.len() as u16);
                for m in monitors {
                    w.str(&m.name);
                    w.u32(m.logical_width);
                    w.u32(m.logical_height);
                    w.u32(m.physical_width);
                    w.u32(m.physical_height);
                    w.u32(m.scale_120);
                }
            }
            Response::Status(status) => {
                w.str(&status.version);
                w.u32(status.pid);
                w.u64(status.rss_bytes);
                w.u16(status.monitors.len() as u16);
                for m in &status.monitors {
                    w.str(&m.name);
                    match &m.wallpaper {
                        None => w.u8(0),
                        Some(wallpaper) => {
                            w.u8(1);
                            w.str(&wallpaper.path);
                            w.u8(wallpaper.fit.code());
                        }
                    }
                }
                write_settings(w, &status.settings);
            }
        }
    }

    fn read(kind: u16, r: &mut Reader<'_>) -> Result<Self, Error> {
        Ok(match kind {
            kind::OK => Response::Ok,
            kind::ERROR => Response::Error(r.string()?),
            kind::MONITORS_REPLY => {
                let count = r.u16()?;
                let mut monitors = Vec::with_capacity(count.into());
                for _ in 0..count {
                    monitors.push(MonitorInfo {
                        name: r.string()?,
                        logical_width: r.u32()?,
                        logical_height: r.u32()?,
                        physical_width: r.u32()?,
                        physical_height: r.u32()?,
                        scale_120: r.u32()?,
                    });
                }
                Response::Monitors(monitors)
            }
            kind::STATUS_REPLY => {
                let version = r.string()?;
                let pid = r.u32()?;
                let rss_bytes = r.u64()?;
                let count = r.u16()?;
                let mut monitors = Vec::with_capacity(count.into());
                for _ in 0..count {
                    let name = r.string()?;
                    let wallpaper = match r.u8()? {
                        0 => None,
                        _ => Some(Wallpaper {
                            path: r.string()?,
                            fit: FitMode::from_code(r.u8()?)?,
                        }),
                    };
                    monitors.push(MonitorStatus { name, wallpaper });
                }
                Response::Status(Status {
                    version,
                    pid,
                    rss_bytes,
                    monitors,
                    settings: read_settings(r)?,
                })
            }
            other => return Err(Error::UnknownKind(other)),
        })
    }
}
