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

const MAX_IPC_SHM_LEN: usize = 256 * 1024 * 1024;

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

// TODO: remove this ugly mess
impl From<RawMsg> for RequestRecv {
    fn from(value: RawMsg) -> Self {
        match value.code {
            Code::ReqPing => Self::Ping,
            Code::ReqQuery => Self::Query,
            Code::ReqClear => {
                let Some(mmap) = value.shm else {
                    return Self::Kill;
                };
                let bytes = mmap.slice();
                if bytes.len() < 5 {
                    return Self::Kill;
                }
                let len = bytes[0] as usize;
                let mut outputs = Vec::with_capacity(len);
                let mut i = 1;
                for _ in 0..len {
                    if i + 4 > bytes.len() {
                        return Self::Kill;
                    }
                    let output_len =
                        u32::from_ne_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
                    if i + 4 + output_len > bytes.len() {
                        return Self::Kill;
                    }
                    let output = MmappedStr::new(&mmap, &bytes[i..]);
                    i += 4 + output.str().len();
                    outputs.push(output);
                }
                if i + 4 > bytes.len() {
                    return Self::Kill;
                }
                let color = [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]];
                Self::Clear(ClearReq {
                    color,
                    outputs: outputs.into(),
                })
            }
            Code::ReqImg => {
                let Some(mmap) = value.shm else {
                    return Self::Kill;
                };
                let bytes = mmap.slice();
                if bytes.len() < 52 {
                    return Self::Kill;
                }
                let transition = Transition::deserialize(&bytes[0..]);
                let len = bytes[51] as usize;

                let mut imgs = Vec::with_capacity(len);
                let mut outputs = Vec::with_capacity(len);
                let mut animations = Vec::with_capacity(len);

                let mut i = 52;
                for _ in 0..len {
                    if i >= bytes.len() {
                        return Self::Kill;
                    }
                    let (img, offset) = ImgReq::deserialize(&mmap, &bytes[i..]);
                    if i + offset > bytes.len() {
                        return Self::Kill;
                    }
                    i += offset;
                    imgs.push(img);

                    if i >= bytes.len() {
                        return Self::Kill;
                    }
                    let n_outputs = bytes[i] as usize;
                    i += 1;
                    let mut out = Vec::with_capacity(n_outputs);
                    for _ in 0..n_outputs {
                        if i + 4 > bytes.len() {
                            return Self::Kill;
                        }
                        let output_len =
                            u32::from_ne_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
                        if i + 4 + output_len > bytes.len() {
                            return Self::Kill;
                        }
                        let output = MmappedStr::new(&mmap, &bytes[i..]);
                        i += 4 + output.str().len();
                        out.push(output);
                    }
                    outputs.push(out.into());

                    if i >= bytes.len() {
                        return Self::Kill;
                    }
                    if bytes[i] == 1 {
                        let Some((animation, offset)) =
                            Animation::deserialize(&mmap, &bytes[i + 1..])
                        else {
                            return Self::Kill;
                        };
                        if i + offset > bytes.len() {
                            return Self::Kill;
                        }
                        i += offset;
                        animations.push(animation);
                    }
                    i += 1;
                }

                Self::Img(ImageReq {
                    transition,
                    imgs,
                    outputs,
                    animations: if animations.is_empty() {
                        None
                    } else {
                        Some(animations)
                    },
                })
            }
            Code::ReqPause => Self::Pause,
            Code::ReqKill => Self::Kill,
            _ => Self::Kill,
        }
    }
}

impl From<RawMsg> for Answer {
    fn from(value: RawMsg) -> Self {
        match value.code {
            Code::ResOk => Self::Ok,
            Code::ResConfigured => Self::Ping(true),
            Code::ResAwait => Self::Ping(false),
            Code::ResInfo => {
                let Some(mmap) = value.shm else {
                    return Self::Ok;
                };
                let bytes = mmap.slice();
                if bytes.is_empty() {
                    return Self::Info(Vec::new().into());
                }
                let len = bytes[0] as usize;
                let mut bg_infos = Vec::with_capacity(len);

                let mut i = 1;
                for _ in 0..len {
                    if i >= bytes.len() {
                        break;
                    }
                    let (info, offset) = BgInfo::deserialize(&bytes[i..]);
                    if offset == 0 {
                        break;
                    }
                    i += offset;
                    bg_infos.push(info);
                }

                Self::Info(bg_infos.into())
            }
            _ => Self::Ok,
        }
    }
}
// TODO: end remove ugly mess block

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

// TODO: this along with `RawMsg` should be implementation detail
impl IpcSocket {
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
        let mut received = 0usize;

        for _ in 0..5 {
            let iov = io::IoSliceMut::new(&mut buf);
            match net::recvmsg(self.as_fd(), &mut [iov], &mut control, RecvFlags::WAITALL) {
                Ok(msg) => {
                    received = msg.bytes;
                    break;
                }
                Err(Errno::WOULDBLOCK | Errno::INTR) => {
                    _ = thread::nanosleep(&thread::Timespec {
                        tv_sec: 0,
                        tv_nsec: 1_000_000,
                    });
                }
                Err(err) => return Err(err).context(IpcErrorKind::Read),
            }
        }

        if received != buf.len() {
            return Err(Errno::BADMSG).context(IpcErrorKind::Read);
        }

        let code = u64::from_ne_bytes(buf[0..8].try_into().unwrap()).try_into()?;
        let len = u64::from_ne_bytes(buf[8..16].try_into().unwrap()) as usize;

        if len > MAX_IPC_SHM_LEN {
            return Err(Errno::MSGSIZE).context(IpcErrorKind::MalformedMsg);
        }

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
