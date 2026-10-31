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
    pub const PRESENT: u16 = 0x0201;

    pub const OK: u16 = 0x8000;
    pub const ERROR: u16 = 0x8001;
    pub const MONITORS_REPLY: u16 = 0x8101;
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
        }
    }

    fn write(&self, w: &mut Writer) {
        match self {
            Request::Monitors => {}
            Request::Present(p) => {
                w.str(&p.monitor);
                w.u32(p.width);
                w.u32(p.height);
                w.u32(p.stride);
                w.u32(p.format as u32);
                w.str(&p.source);
                w.u8(p.transition.kind as u8);
                w.u8(p.transition.direction as u8);
                w.u8(p.transition.easing as u8);
                w.u32(p.transition.duration_ms);
            }
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
            }),
            other => return Err(Error::UnknownKind(other)),
        })
    }
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
            other => return Err(Error::UnknownKind(other)),
        })
    }
}
