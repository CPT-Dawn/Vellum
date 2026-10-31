//! Sending and receiving frames, with file descriptors, over a Unix socket.

use std::io::{IoSlice, IoSliceMut};
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use rustix::net::{
    RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer,
    SendAncillaryMessage, SendFlags, recvmsg, sendmsg,
};

use crate::codec::{self, HEADER_LEN, Header, Message};
use crate::{Error, Request, Response};

/// Most file descriptors one frame may carry.
const MAX_FDS: usize = 4;

/// Sends `message` as one frame, with `fds` attached to its first byte.
pub fn send<M: Message>(socket: impl AsFd, message: &M, fds: &[BorrowedFd<'_>]) -> Result<(), Error> {
    assert!(fds.len() <= MAX_FDS, "too many fds for one frame");
    let frame = codec::encode(message);
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_FDS))];
    let mut control = SendAncillaryBuffer::new(&mut space);
    if !fds.is_empty() {
        control.push(SendAncillaryMessage::ScmRights(fds));
    }
    let mut sent = sendmsg(
        &socket,
        &[IoSlice::new(&frame)],
        &mut control,
        SendFlags::NOSIGNAL,
    )?;
    while sent < frame.len() {
        sent += rustix::net::send(&socket, &frame[sent..], SendFlags::NOSIGNAL)?;
    }
    Ok(())
}

/// Receives one frame and any file descriptors attached to it.
pub fn recv<M: Message>(socket: impl AsFd) -> Result<(M, Vec<OwnedFd>), Error> {
    let mut fds = Vec::new();
    let mut header = [0u8; HEADER_LEN];
    read_exact(&socket, &mut header, &mut fds)?;
    let header = Header::parse(&header)?;
    let mut payload = vec![0u8; header.len as usize];
    read_exact(&socket, &mut payload, &mut fds)?;
    Ok((codec::decode_payload(header, &payload)?, fds))
}

fn read_exact(socket: &impl AsFd, mut buf: &mut [u8], fds: &mut Vec<OwnedFd>) -> Result<(), Error> {
    while !buf.is_empty() {
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_FDS))];
        let mut control = RecvAncillaryBuffer::new(&mut space);
        let received = recvmsg(
            socket,
            &mut [IoSliceMut::new(buf)],
            &mut control,
            RecvFlags::CMSG_CLOEXEC,
        )?;
        for message in control.drain() {
            if let RecvAncillaryMessage::ScmRights(received_fds) = message {
                fds.extend(received_fds);
            }
        }
        if received.bytes == 0 {
            return Err(Error::Closed);
        }
        buf = &mut buf[received.bytes..];
    }
    Ok(())
}

/// Connects to the Daemon at `socket`, sends `request` with `fds` and waits
/// for the response.
pub fn request(
    socket: &Path,
    request: &Request,
    fds: &[BorrowedFd<'_>],
) -> Result<Response, Error> {
    let stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    send(&stream, request, fds)?;
    let (response, _fds) = recv(&stream)?;
    Ok(response)
}
