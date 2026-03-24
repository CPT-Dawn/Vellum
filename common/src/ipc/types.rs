use ::alloc::boxed::Box;
use ::alloc::string::String;
use ::alloc::vec::Vec;

use core::fmt;
use core::num::{NonZeroI32, NonZeroU8};

use rustix::io;

use crate::compression::BitPack;
use crate::mmap::Mmap;
use crate::mmap::MmappedBytes;
use crate::mmap::MmappedStr;

use super::ImageRequestBuilder;

#[inline]
fn read_u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    let raw = bytes.get(offset..offset + 4)?;
    Some(u32::from_ne_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

#[inline]
fn read_i32_at(bytes: &[u8], offset: usize) -> Option<i32> {
    let raw = bytes.get(offset..offset + 4)?;
    Some(i32::from_ne_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

#[inline]
fn read_u64_at(bytes: &[u8], offset: usize) -> Option<u64> {
    let raw = bytes.get(offset..offset + 8)?;
    Some(u64::from_ne_bytes([
        raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
    ]))
}

#[inline]
fn read_f32_at(bytes: &[u8], offset: usize) -> Option<f32> {
    Some(f32::from_ne_bytes(
        read_u32_at(bytes, offset)?.to_ne_bytes(),
    ))
}

#[inline]
fn read_f64_at(bytes: &[u8], offset: usize) -> Option<f64> {
    Some(f64::from_ne_bytes(
        read_u64_at(bytes, offset)?.to_ne_bytes(),
    ))
}

#[derive(Clone, Debug, PartialEq)]
pub enum Coord {
    Pixel(f32),
    Percent(f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Position {
    pub x: Coord,
    pub y: Coord,
}

impl Position {
    #[must_use]
    pub fn new(x: Coord, y: Coord) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub fn to_pixel(&self, dim: (u32, u32), invert_y: bool) -> (f32, f32) {
        let x = match self.x {
            Coord::Pixel(x) => x,
            Coord::Percent(x) => x * dim.0 as f32,
        };

        let y = match self.y {
            Coord::Pixel(y) => {
                if invert_y {
                    y
                } else {
                    dim.1 as f32 - y
                }
            }
            Coord::Percent(y) => {
                if invert_y {
                    y * dim.1 as f32
                } else {
                    (1.0 - y) * dim.1 as f32
                }
            }
        };

        (x, y)
    }

    #[must_use]
    pub fn to_percent(&self, dim: (u32, u32)) -> (f32, f32) {
        let x = match self.x {
            Coord::Pixel(x) => x / dim.0 as f32,
            Coord::Percent(x) => x,
        };

        let y = match self.y {
            Coord::Pixel(y) => y / dim.1 as f32,
            Coord::Percent(y) => y,
        };

        (x, y)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum BgImg {
    Color([u8; 4]),
    Img(Box<str>),
}

impl BgImg {
    fn serialized_size(&self) -> usize {
        1 //discriminant
        + match self {
            Self::Color(_) => 4,
            Self::Img(s) => 4 + s.len()
        }
    }

    pub fn is_set(&self) -> bool {
        matches!(self, Self::Img(_))
    }
}

impl fmt::Display for BgImg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BgImg::Color(color) => {
                write!(f, "color: {:02X}{:02X}{:02X}", color[0], color[1], color[2])
            }
            BgImg::Img(p) => write!(f, "image: {p}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u8)]
pub enum PixelFormat {
    /// No swap, can copy directly onto WlBuffer
    Bgr = 0,
    /// Swap R and B channels at client, can copy directly onto WlBuffer
    Rgb = 1,
    /// No swap, must extend pixel with an extra byte when displaying animations
    Abgr = 2,
    /// Swap R and B channels at client, must extend pixel with an extra byte when displaying
    /// animations
    Argb = 3,
}

impl fmt::Display for PixelFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PixelFormat::Bgr => f.write_str("Bgr"),
            PixelFormat::Rgb => f.write_str("Rgb"),
            PixelFormat::Abgr => f.write_str("Abgr"),
            PixelFormat::Argb => f.write_str("Argb"),
        }
    }
}

impl PixelFormat {
    #[inline]
    #[must_use]
    pub const fn channels(&self) -> u8 {
        match self {
            Self::Rgb | Self::Bgr => 3,
            Self::Abgr | Self::Argb => 4,
        }
    }

    #[inline]
    #[must_use]
    pub const fn must_swap_r_and_b_channels(&self) -> bool {
        match self {
            Self::Bgr | Self::Abgr => false,
            Self::Rgb | Self::Argb => true,
        }
    }

    #[inline]
    #[must_use]
    pub const fn can_copy_directly_onto_wl_buffer(&self) -> bool {
        match self {
            Self::Bgr | Self::Rgb => true,
            Self::Abgr | Self::Argb => false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Scale {
    /// sent by wl_output::scale events
    Output(NonZeroI32),
    /// sent by wl_surface::preferred_buffer_scale events
    Preferred(NonZeroI32),
    /// sent by wp_fractional_scale_v1::preferred_scale events
    Fractional(NonZeroI32),
}

impl Scale {
    #[inline]
    #[must_use]
    pub fn priority(&self) -> u32 {
        match self {
            Self::Output(_) => 0,
            Self::Preferred(_) => 1,
            Self::Fractional(_) => 2,
        }
    }

    #[inline]
    #[must_use]
    pub fn mul_dim(&self, width: i32, height: i32) -> (i32, i32) {
        match self {
            Self::Output(i) | Self::Preferred(i) => (width * i.get(), height * i.get()),
            Self::Fractional(f) => {
                let width = (width * f.get() + 60) / 120;
                let height = (height * f.get() + 60) / 120;
                (width, height)
            }
        }
    }

    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn to_f32(&self) -> f32 {
        match self {
            Self::Output(i) | Self::Preferred(i) => i.get() as f32,
            Self::Fractional(f) => f.get() as f32 / 120.0,
        }
    }
}

impl PartialEq for Scale {
    fn eq(&self, other: &Self) -> bool {
        (match self {
            Self::Output(i) | Self::Preferred(i) => i.get() * 120,
            Self::Fractional(f) => f.get(),
        }) == (match other {
            Self::Output(i) | Self::Preferred(i) => i.get() * 120,
            Self::Fractional(f) => f.get(),
        })
    }
}

impl fmt::Display for Scale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_f32())
    }
}

#[derive(Clone)]
pub struct BgInfo {
    pub name: Box<str>,
    pub dim: (u32, u32),
    pub scale_factor: Scale,
    pub img: BgImg,
    pub pixel_format: PixelFormat,
}

impl BgInfo {
    #[inline]
    #[must_use]
    pub fn real_dim(&self) -> (u32, u32) {
        let width = i32::try_from(self.dim.0).unwrap_or(i32::MAX);
        let height = i32::try_from(self.dim.1).unwrap_or(i32::MAX);
        let dim = self.scale_factor.mul_dim(width, height);
        (
            u32::try_from(dim.0).unwrap_or(0),
            u32::try_from(dim.1).unwrap_or(0),
        )
    }

    pub(super) fn serialized_size(&self) -> usize {
        4 // name len
            + self.name.len()
            + 8 //dim
            + 5 //scale_factor (discriminant + value)
            + self.img.serialized_size()
            + 1 //pixel_format
    }

    pub(super) fn serialize(&self, buf: &mut [u8]) -> usize {
        let Self {
            name,
            dim,
            scale_factor,
            img,
            pixel_format,
        } = self;

        let name_len = name.len();
        let name_len_u32 = u32::try_from(name_len).unwrap_or(u32::MAX);
        buf[0..4].copy_from_slice(&name_len_u32.to_ne_bytes());
        buf[4..4 + name_len].copy_from_slice(name.as_bytes());
        let mut i = 4 + name_len;
        buf[i..i + 4].copy_from_slice(&dim.0.to_ne_bytes());
        buf[i + 4..i + 8].copy_from_slice(&dim.1.to_ne_bytes());
        i += 8;

        match scale_factor {
            Scale::Output(value) => {
                buf[i] = 0;
                buf[i + 1..i + 5].copy_from_slice(&value.get().to_ne_bytes());
            }
            Scale::Preferred(value) => {
                buf[i] = 1;
                buf[i + 1..i + 5].copy_from_slice(&value.get().to_ne_bytes());
            }
            Scale::Fractional(value) => {
                buf[i] = 2;
                buf[i + 1..i + 5].copy_from_slice(&value.get().to_ne_bytes());
            }
        }
        i += 5;

        match img {
            BgImg::Color(color) => {
                buf[i] = 0;
                buf[i + 1..i + 5].copy_from_slice(color);
                i += 5;
            }
            BgImg::Img(path) => {
                buf[i] = 1;
                i += 1;
                let path_len = path.len();
                let path_len_u32 = u32::try_from(path_len).unwrap_or(u32::MAX);
                buf[i..i + 4].copy_from_slice(&path_len_u32.to_ne_bytes());
                buf[i + 4..i + 4 + path_len].copy_from_slice(path.as_bytes());
                i += 4 + path_len;
            }
        }

        buf[i] = *pixel_format as u8;
        i + 1
    }

    pub(super) fn deserialize(bytes: &[u8]) -> (Self, usize) {
        let name = deserialize_boxed_str(bytes);
        let mut i = name.len() + 4;

        if bytes.len() <= i + 17 {
            return (
                Self {
                    name,
                    dim: (0, 0),
                    scale_factor: Scale::Output(NonZeroI32::MIN),
                    img: BgImg::Color([0, 0, 0, 0]),
                    pixel_format: PixelFormat::Argb,
                },
                bytes.len(),
            );
        }

        let dim = (
            read_u32_at(bytes, i).unwrap_or(0),
            read_u32_at(bytes, i + 4).unwrap_or(0),
        );
        i += 8;

        let scale_factor = if bytes[i] == 0 {
            Scale::Output(
                NonZeroI32::new(read_i32_at(bytes, i + 1).unwrap_or(1)).unwrap_or(NonZeroI32::MIN),
            )
        } else if bytes[i] == 1 {
            Scale::Preferred(
                NonZeroI32::new(read_i32_at(bytes, i + 1).unwrap_or(1)).unwrap_or(NonZeroI32::MIN),
            )
        } else {
            Scale::Fractional(
                NonZeroI32::new(read_i32_at(bytes, i + 1).unwrap_or(120))
                    .unwrap_or(NonZeroI32::MIN),
            )
        };
        i += 5;

        let img = if bytes[i] == 0 {
            i += 5;
            BgImg::Color([bytes[i - 4], bytes[i - 3], bytes[i - 2], bytes[i - 1]])
        } else {
            i += 1;
            let path = deserialize_boxed_str(&bytes[i..]);
            i += 4 + path.len();
            BgImg::Img(path)
        };

        let pixel_format = match bytes[i] {
            0 => PixelFormat::Bgr,
            1 => PixelFormat::Rgb,
            2 => PixelFormat::Abgr,
            _ => PixelFormat::Argb,
        };
        i += 1;

        (
            Self {
                name,
                dim,
                scale_factor,
                img,
                pixel_format,
            },
            i,
        )
    }
}

impl fmt::Display for BgInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {}x{}, scale: {}, currently displaying: {}",
            self.name, self.dim.0, self.dim.1, self.scale_factor, self.img
        )
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub enum TransitionType {
    Simple = 0,
    Fade = 1,
    Outer = 2,
    Wipe = 3,
    Grow = 4,
    Wave = 5,
    None = 6,
}

#[derive(Debug)]
pub struct Transition {
    pub transition_type: TransitionType,
    pub duration: f32,
    pub step: NonZeroU8,
    pub fps: u16,
    pub angle: f64,
    pub pos: Position,
    pub bezier: (f32, f32, f32, f32),
    pub wave: (f32, f32),
    pub invert_y: bool,
}

impl Transition {
    pub(super) fn serialize(&self, buf: &mut ImageRequestBuilder) {
        let Self {
            transition_type,
            duration,
            step,
            fps,
            angle,
            pos,
            bezier,
            wave,
            invert_y,
        } = self;

        buf.push_byte(*transition_type as u8);
        buf.extend(&duration.to_ne_bytes());
        buf.push_byte(step.get());
        buf.extend(&fps.to_ne_bytes());
        buf.extend(&angle.to_ne_bytes());
        match pos.x {
            Coord::Pixel(f) => {
                buf.push_byte(0);
                buf.extend(&f.to_ne_bytes());
            }
            Coord::Percent(f) => {
                buf.push_byte(1);
                buf.extend(&f.to_ne_bytes());
            }
        }
        match pos.y {
            Coord::Pixel(f) => {
                buf.push_byte(0);
                buf.extend(&f.to_ne_bytes());
            }
            Coord::Percent(f) => {
                buf.push_byte(1);
                buf.extend(&f.to_ne_bytes());
            }
        }
        buf.extend(&bezier.0.to_ne_bytes());
        buf.extend(&bezier.1.to_ne_bytes());
        buf.extend(&bezier.2.to_ne_bytes());
        buf.extend(&bezier.3.to_ne_bytes());
        buf.extend(&wave.0.to_ne_bytes());
        buf.extend(&wave.1.to_ne_bytes());
        buf.push_byte(u8::from(*invert_y));
    }

    pub(super) fn deserialize(bytes: &[u8]) -> Self {
        if bytes.len() <= 50 {
            return Self {
                transition_type: TransitionType::None,
                duration: 0.0,
                step: NonZeroU8::MIN,
                fps: 60,
                angle: 0.0,
                pos: Position::new(Coord::Percent(0.5), Coord::Percent(0.5)),
                bezier: (0.0, 0.0, 1.0, 1.0),
                wave: (10.0, 10.0),
                invert_y: false,
            };
        }
        let transition_type = match bytes[0] {
            0 => TransitionType::Simple,
            1 => TransitionType::Fade,
            2 => TransitionType::Outer,
            3 => TransitionType::Wipe,
            4 => TransitionType::Grow,
            5 => TransitionType::Wave,
            _ => TransitionType::None,
        };
        let duration = read_f32_at(bytes, 1).unwrap_or(0.0);
        let step = NonZeroU8::new(bytes[5]).unwrap_or(NonZeroU8::MIN);
        let fps = read_u32_at(bytes, 4)
            .map(|_| {
                let raw = bytes.get(6..8).unwrap_or(&[60, 0]);
                u16::from_ne_bytes([raw[0], raw[1]])
            })
            .unwrap_or(60);
        let angle = read_f64_at(bytes, 8).unwrap_or(0.0);
        let pos = {
            let x = if bytes[16] == 0 {
                Coord::Pixel(read_f32_at(bytes, 17).unwrap_or(0.5))
            } else {
                Coord::Percent(read_f32_at(bytes, 17).unwrap_or(0.5))
            };
            let y = if bytes[21] == 0 {
                Coord::Pixel(read_f32_at(bytes, 22).unwrap_or(0.5))
            } else {
                Coord::Percent(read_f32_at(bytes, 22).unwrap_or(0.5))
            };
            Position { x, y }
        };

        let bezier = (
            read_f32_at(bytes, 26).unwrap_or(0.0),
            read_f32_at(bytes, 30).unwrap_or(0.0),
            read_f32_at(bytes, 34).unwrap_or(1.0),
            read_f32_at(bytes, 38).unwrap_or(1.0),
        );

        let wave = (
            read_f32_at(bytes, 42).unwrap_or(10.0),
            read_f32_at(bytes, 46).unwrap_or(10.0),
        );

        let invert_y = bytes[50] != 0;

        Self {
            transition_type,
            duration,
            step,
            fps,
            angle,
            pos,
            bezier,
            wave,
            invert_y,
        }
    }
}

pub struct ClearSend {
    pub color: [u8; 4],
    pub outputs: Box<[String]>,
}

impl ClearSend {
    pub fn create_request(self) -> io::Result<Mmap> {
        // 1 - output length
        // 4 - color bytes
        // 4 + output.len() - output len + bytes
        let len = 5 + self.outputs.iter().map(|o| 4 + o.len()).sum::<usize>();
        let mut mmap = Mmap::create(len)?;
        let bytes = mmap.slice_mut();
        // we assume someone does not have more than
        // 255 monitors. Seems reasonable
        bytes[0] = self.outputs.len() as u8;
        let mut i = 1;
        for output in self.outputs {
            let len = output.len() as u32;
            bytes[i..i + 4].copy_from_slice(&len.to_ne_bytes());
            bytes[i + 4..i + 4 + len as usize].copy_from_slice(output.as_bytes());
            i += 4 + len as usize;
        }
        bytes[i..i + 4].copy_from_slice(&self.color);
        Ok(mmap)
    }
}

pub struct ClearReq {
    pub color: [u8; 4],
    pub outputs: Box<[MmappedStr]>,
}

pub struct ImgSend {
    pub path: String,
    pub dim: (u32, u32),
    pub format: PixelFormat,
    pub img: Box<[u8]>,
}

pub struct ImgReq {
    pub path: MmappedStr,
    pub dim: (u32, u32),
    pub format: PixelFormat,
    pub img: MmappedBytes,
}

impl ImgReq {
    pub(super) fn deserialize(mmap: &Mmap, bytes: &[u8]) -> (Self, usize) {
        let mut i = 0;
        let path = MmappedStr::new(mmap, &bytes[i..]);
        i += 4 + path.str().len();

        let img = MmappedBytes::new(mmap, &bytes[i..]);
        i += 4 + img.bytes().len();

        let dim = (
            read_u32_at(bytes, i).unwrap_or(0),
            read_u32_at(bytes, i + 4).unwrap_or(0),
        );
        i += 8;

        let format = match bytes[i] {
            0 => PixelFormat::Bgr,
            1 => PixelFormat::Rgb,
            2 => PixelFormat::Abgr,
            _ => PixelFormat::Argb,
        };
        i += 1;

        (
            Self {
                path,
                dim,
                format,
                img,
            },
            i,
        )
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy)]
pub struct Nanos(u64);

impl Nanos {
    #[inline]
    pub fn from_nanos(nanos: u64) -> Self {
        Self(nanos)
    }

    #[inline]
    pub fn from_millis(millis: u64) -> Self {
        Self(millis * 1_000_000)
    }

    #[inline]
    pub fn into_timespec(self) -> rustix::time::Timespec {
        let secs = (self.0 / 1_000_000_000) as rustix::time::Secs;
        let nsecs = (self.0 % 1_000_000_000) as rustix::time::Nsecs;
        rustix::time::Timespec {
            tv_sec: secs,
            tv_nsec: nsecs,
        }
    }
}

impl core::ops::AddAssign for Nanos {
    fn add_assign(&mut self, rhs: Self) {
        self.0 = self.0 + rhs.0;
    }
}

pub struct Animation {
    pub animation: Box<[(BitPack, Nanos)]>,
}

impl Animation {
    pub(crate) fn serialize(&self, buf: &mut ImageRequestBuilder) {
        let Self { animation } = self;

        buf.extend(&(animation.len() as u32).to_ne_bytes());
        for (bitpack, duration) in animation {
            bitpack.serialize(buf);
            buf.extend(&duration.0.to_ne_bytes());
        }
    }

    pub(crate) fn deserialize(mmap: &Mmap, bytes: &[u8]) -> Option<(Self, usize)> {
        let mut i = 0;
        let animation_len = read_u32_at(bytes, i)? as usize;
        i += 4;
        let mut animation = Vec::with_capacity(animation_len);
        for _ in 0..animation_len {
            let (anim, offset) = BitPack::deserialize(mmap, bytes.get(i..)?)?;
            i += offset;
            let duration = Nanos(read_u64_at(bytes, i)?);
            i += 8;
            animation.push((anim, duration));
        }

        Some((
            Self {
                animation: animation.into(),
            },
            i,
        ))
    }
}

pub struct ImageReq {
    pub transition: Transition,
    pub imgs: Vec<ImgReq>,
    pub outputs: Vec<Box<[MmappedStr]>>,
    pub animations: Option<Vec<Animation>>,
}

fn deserialize_boxed_str(bytes: &[u8]) -> Box<str> {
    if bytes.len() < 4 {
        return "".into();
    }

    let Some(size) = read_u32_at(bytes, 0).map(|s| s as usize) else {
        return "".into();
    };
    if bytes.len() < 4 + size {
        return "".into();
    }

    match core::str::from_utf8(&bytes[4..4 + size]) {
        Ok(text) => text.into(),
        Err(_) => "".into(),
    }
}
