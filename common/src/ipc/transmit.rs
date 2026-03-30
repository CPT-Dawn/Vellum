use ::alloc::vec::Vec;

use core::mem::MaybeUninit;

use rustix::io;
use rustix::io::Errno;
use rustix::net;
use rustix::net::RecvFlags;
use rustix::thread;

use super::Animation;
use super::Answer;
use super::BgInfo;
use super::ClearReq;
use super::ErrnoExt;
use super::ImageReq;
use super::ImgReq;
use super::IpcError;
use super::IpcErrorKind;
use super::IpcSocket;
use super::RequestRecv;
use super::RequestSend;
use super::Transition;
use crate::mmap::Mmap;
use crate::mmap::MmappedStr;

// could be enum
pub struct RawMsg {
    code: Code,
    shm: Option<Mmap>,
}

impl From<RequestSend> for RawMsg {
    fn from(value: RequestSend) -> Self {
        let code = match value {
            RequestSend::Ping => Code::ReqPing,
            RequestSend::Query => Code::ReqQuery,
            RequestSend::Clear(_) => Code::ReqClear,
            RequestSend::Img(_) => Code::ReqImg,
            RequestSend::Pause => Code::ReqPause,
            RequestSend::Kill => Code::ReqKill,
        };

        let shm = match value {
            RequestSend::Clear(mem) | RequestSend::Img(mem) => Some(mem),
            _ => None,
        };

        Self { code, shm }
    }
}

impl TryFrom<Answer> for RawMsg {
    type Error = IpcError;

    fn try_from(value: Answer) -> Result<Self, Self::Error> {
        let code = match value {
            Answer::Ok => Code::ResOk,
            Answer::Ping(true) => Code::ResConfigured,
            Answer::Ping(false) => Code::ResAwait,
            Answer::Info(_) => Code::ResInfo,
        };

        let shm = if let Answer::Info(infos) = value {
            let len = 1 + infos.iter().map(BgInfo::serialized_size).sum::<usize>();
            let mut mmap = Mmap::create(len).context(IpcErrorKind::MemoryMapCreation)?;
            let bytes = mmap.slice_mut();

            bytes[0] = infos.len() as u8;
            let mut i = 1;

            for info in infos {
                i += info.serialize(&mut bytes[i..]);
            }

            Some(mmap)
        } else {
            None
        };

        Ok(Self { code, shm })
    }
}

impl TryFrom<RawMsg> for RequestRecv {
    type Error = IpcError;

    fn try_from(value: RawMsg) -> Result<Self, Self::Error> {
        match value.code {
            Code::ReqPing => Ok(Self::Ping),
            Code::ReqQuery => Ok(Self::Query),
            Code::ReqClear => {
                let mmap = value
                    .shm
                    .ok_or(Errno::BADMSG)
                    .context(IpcErrorKind::MalformedMsg)?;
                let bytes = mmap.slice();
                let len = *bytes
                    .first()
                    .ok_or(Errno::BADMSG)
                    .context(IpcErrorKind::MalformedMsg)? as usize;

                let mut outputs = Vec::with_capacity(len);
                let mut i = 1;
                for _ in 0..len {
                    let rest = bytes
                        .get(i..)
                        .ok_or(Errno::BADMSG)
                        .context(IpcErrorKind::MalformedMsg)?;
                    let output = MmappedStr::new(&mmap, rest);
                    i += 4 + output.str().len();
                    outputs.push(output);
                }

                let color_slice = bytes
                    .get(i..i + 4)
                    .ok_or(Errno::BADMSG)
                    .context(IpcErrorKind::MalformedMsg)?;
                let color = [
                    color_slice[0],
                    color_slice[1],
                    color_slice[2],
                    color_slice[3],
                ];

                Ok(Self::Clear(ClearReq {
                    color,
                    outputs: outputs.into(),
                }))
            }
            Code::ReqImg => {
                let mmap = value
                    .shm
                    .ok_or(Errno::BADMSG)
                    .context(IpcErrorKind::MalformedMsg)?;
                let bytes = mmap.slice();
                if bytes.len() < 52 {
                    return Err(Errno::BADMSG).context(IpcErrorKind::MalformedMsg);
                }

                let transition = Transition::deserialize(&bytes[0..]);
                let len = bytes[51] as usize;

                let mut imgs = Vec::with_capacity(len);
                let mut outputs = Vec::with_capacity(len);
                let mut animations = Vec::with_capacity(len);

                let mut i = 52;
                for _ in 0..len {
                    let img_bytes = bytes
                        .get(i..)
                        .ok_or(Errno::BADMSG)
                        .context(IpcErrorKind::MalformedMsg)?;
                    let (img, offset) = ImgReq::deserialize(&mmap, img_bytes);
                    i += offset;
                    imgs.push(img);

                    let n_outputs = *bytes
                        .get(i)
                        .ok_or(Errno::BADMSG)
                        .context(IpcErrorKind::MalformedMsg)?
                        as usize;
                    i += 1;
                    let mut out = Vec::with_capacity(n_outputs);
                    for _ in 0..n_outputs {
                        let rest = bytes
                            .get(i..)
                            .ok_or(Errno::BADMSG)
                            .context(IpcErrorKind::MalformedMsg)?;
                        let output = MmappedStr::new(&mmap, rest);
                        i += 4 + output.str().len();
                        out.push(output);
                    }
                    outputs.push(out.into());

                    match *bytes
                        .get(i)
                        .ok_or(Errno::BADMSG)
                        .context(IpcErrorKind::MalformedMsg)?
                    {
                        0 => i += 1,
                        1 => {
                            let animation_bytes = bytes
                                .get(i + 1..)
                                .ok_or(Errno::BADMSG)
                                .context(IpcErrorKind::MalformedMsg)?;
                            let (animation, offset) =
                                Animation::deserialize(&mmap, animation_bytes)
                                    .ok_or(Errno::BADMSG)
                                    .context(IpcErrorKind::MalformedMsg)?;
                            i += 1 + offset;
                            animations.push(animation);
                        }
                        _ => return Err(Errno::BADMSG).context(IpcErrorKind::MalformedMsg),
                    }
                }

                Ok(Self::Img(ImageReq {
                    transition,
                    imgs,
                    outputs,
                    animations: if animations.is_empty() {
                        None
                    } else {
                        Some(animations)
                    },
                }))
            }
            Code::ReqPause => Ok(Self::Pause),
            Code::ReqKill => Ok(Self::Kill),
            _ => Err(Errno::BADMSG).context(IpcErrorKind::MalformedMsg),
        }
    }
}

impl TryFrom<RawMsg> for Answer {
    type Error = IpcError;

    fn try_from(value: RawMsg) -> Result<Self, Self::Error> {
        match value.code {
            Code::ResOk => Ok(Self::Ok),
            Code::ResConfigured => Ok(Self::Ping(true)),
            Code::ResAwait => Ok(Self::Ping(false)),
            Code::ResInfo => {
                let mmap = value
                    .shm
                    .ok_or(Errno::BADMSG)
                    .context(IpcErrorKind::MalformedMsg)?;
                let bytes = mmap.slice();
                let len = *bytes
                    .first()
                    .ok_or(Errno::BADMSG)
                    .context(IpcErrorKind::MalformedMsg)? as usize;
                let mut bg_infos = Vec::with_capacity(len);

                let mut i = 1;
                for _ in 0..len {
                    let (info, offset) = BgInfo::deserialize(
                        bytes
                            .get(i..)
                            .ok_or(Errno::BADMSG)
                            .context(IpcErrorKind::MalformedMsg)?,
                    );
                    i += offset;
                    bg_infos.push(info);
                }

                Ok(Self::Info(bg_infos.into()))
            }
            _ => Err(Errno::BADMSG).context(IpcErrorKind::MalformedMsg),
        }
    }
}

macro_rules! code {
    ($($name:ident $num:literal),* $(,)?) => {
        #[derive(Debug)]
        pub enum Code {
            $($name,)*
        }

        impl Code {
            const fn into(self) -> u64 {
                match self {
                     $(Self::$name => $num,)*
                }
            }

            const fn from(num: u64) -> Option<Self> {
                 match num {
                     $($num => Some(Self::$name),)*
                     _ => None
                 }
            }
        }

        impl core::fmt::Display for Code {
            fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                match self {
                    Code::ReqPing       => f.write_str("ReqPing"),
                    Code::ReqQuery      => f.write_str("ReqQuery"),
                    Code::ReqClear      => f.write_str("ReqClear"),
                    Code::ReqImg        => f.write_str("ReqImg"),
                    Code::ReqKill       => f.write_str("ReqKill"),
                    Code::ResOk         => f.write_str("ResOk"),
                    Code::ResConfigured => f.write_str("ResConfigured"),
                    Code::ResAwait      => f.write_str("ResAwait"),
                    Code::ResInfo       => f.write_str("ResInfo"),
                    Code::ReqPause      => f.write_str("ReqPause"),
                }
            }
        }

    };
}

code! {
    ReqPing       0,
    ReqQuery      1,
    ReqClear      2,
    ReqImg        3,
    ReqKill       4,

    ResOk         5,
    ResConfigured 6,
    ResAwait      7,
    ResInfo       8,

    ReqPause      9,
}

impl TryFrom<u64> for Code {
    type Error = IpcError;
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::from(value).ok_or(IpcError::new(IpcErrorKind::BadCode, Errno::DOM))
    }
}

impl IpcSocket {
    // RawMsg is intentionally exposed here to keep a single transport implementation used by
    // both daemon and client crates.
    pub fn send(&self, msg: RawMsg) -> Result<(), IpcError> {
        const FLAGS: net::SendFlags = net::SendFlags::empty();

        let mut payload = [0u8; 16];
        payload[0..8].copy_from_slice(&msg.code.into().to_ne_bytes());

        let mut ancillary_buf = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut ancillary = net::SendAncillaryBuffer::new(&mut ancillary_buf);

        let fd;
        if let Some(ref mmap) = msg.shm {
            payload[8..].copy_from_slice(&(mmap.len() as u64).to_ne_bytes());
            fd = [mmap.fd()];
            let msg = net::SendAncillaryMessage::ScmRights(&fd);
            ancillary.push(msg);
        }

        let mut i = 0;
        loop {
            let iov = io::IoSlice::new(&payload[i..]);
            i += net::sendmsg(self.as_fd(), &[iov], &mut ancillary, FLAGS)
                .context(IpcErrorKind::Write)?;
            if i >= payload.len() {
                break;
            } else if i >= 1 {
                // posix in principle guarantees the ancillary data will be sent with the first
                // data octet, so make user not to send it again
                ancillary = net::SendAncillaryBuffer::new(&mut []);
            }
        }
        Ok(())
    }

    pub fn recv(&self) -> Result<RawMsg, IpcError> {
        let mut buf = [0u8; 16];
        let mut ancillary_buf = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];

        let mut control = net::RecvAncillaryBuffer::new(&mut ancillary_buf);

        for _ in 0..5 {
            let iov = io::IoSliceMut::new(&mut buf);
            match net::recvmsg(self.as_fd(), &mut [iov], &mut control, RecvFlags::WAITALL) {
                Ok(_) => break,
                Err(Errno::WOULDBLOCK | Errno::INTR) => {
                    _ = thread::nanosleep(&thread::Timespec {
                        tv_sec: 0,
                        tv_nsec: 1_000_000,
                    });
                }
                Err(err) => return Err(err).context(IpcErrorKind::Read),
            }
        }

        let code = u64::from_ne_bytes(buf[0..8].try_into().unwrap()).try_into()?;
        let len = u64::from_ne_bytes(buf[8..16].try_into().unwrap()) as usize;

        let shm = if len == 0 {
            debug_assert!(
                !matches!(code, Code::ReqImg | Code::ReqClear | Code::ResInfo),
                "Received: Code {code}, which should have sent a shm fd",
            );
            None
        } else {
            let file = control
                .drain()
                .next()
                .and_then(|msg| match msg {
                    net::RecvAncillaryMessage::ScmRights(mut iter) => iter.next(),
                    _ => None,
                })
                .ok_or(Errno::BADMSG)
                .context(IpcErrorKind::MalformedMsg)?;
            Some(Mmap::from_fd(file, len))
        };
        Ok(RawMsg { code, shm })
    }
}
