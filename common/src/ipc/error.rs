use core::fmt;

use alloc::boxed::Box;
use rustix::io::Errno;

use crate::path::Path;

/// Failures if IPC with added context
#[derive(Debug)]
pub struct IpcError {
    err: Errno,
    kind: IpcErrorKind,
}

impl IpcError {
    pub(crate) fn new(kind: IpcErrorKind, err: Errno) -> Self {
        Self { err, kind }
    }
}

#[derive(Debug)]
pub enum IpcErrorKind {
    /// Socket address is incorrect
    SocketAddr,
    /// Failed to create file descriptor
    Socket,
    /// Failed to connect to socket
    Connect,
    /// Binding on socket failed
    Bind,
    /// Listening on socket failed
    Listen,
    /// Socket file wasn't found
    NoSocketFile(Box<Path>),
    /// Socket timeout couldn't be set
    SetTimeout,
    /// IPC contained invalid identification code
    BadCode,
    /// IPC payload was broken
    MalformedMsg,
    /// Failed to create memory map
    MemoryMapCreation,
    /// Reading socket failed
    Read,
    /// Writing socket failed
    Write,
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { err, kind } = self;
        match kind {
            IpcErrorKind::SocketAddr => write!(f, "socket address is incorrect: {err}"),
            IpcErrorKind::Socket => write!(f, "failed to create socket file descriptor: {err}"),
            IpcErrorKind::Connect => write!(f, "failed to connect to socket: {err}"),
            IpcErrorKind::Bind => write!(f, "failed to bind to socket: {err}"),
            IpcErrorKind::Listen => write!(f, "failed to listen on socket: {err}"),
            IpcErrorKind::NoSocketFile(path) => write!(
                f,
                "Socket file '{}' not found. Make sure awww-daemon is running, \
                    and that the --namespace argument matches for the client and the daemon",
                path.display()
            ),
            IpcErrorKind::SetTimeout => write!(f, "failed to set read timeout for socket: {err}"),
            IpcErrorKind::BadCode => write!(f, "invalid message code: {err}"),
            IpcErrorKind::MalformedMsg => write!(f, "malformed ancillary message: {err}"),
            IpcErrorKind::MemoryMapCreation => write!(f, "failed to create memory map: {err}"),
            IpcErrorKind::Read => write!(f, "failed to receive message: {err}"),
            IpcErrorKind::Write => write!(f, "failed to write message: {err}"),
        }
    }
}

impl core::error::Error for IpcError {}

/// Simplify generating [`IpcError`]s from [`Errno`]
pub(crate) trait ErrnoExt {
    type Output;
    fn context(self, kind: IpcErrorKind) -> Self::Output;
}

impl ErrnoExt for Errno {
    type Output = IpcError;
    fn context(self, kind: IpcErrorKind) -> Self::Output {
        IpcError::new(kind, self)
    }
}

impl<T> ErrnoExt for Result<T, Errno> {
    type Output = Result<T, IpcError>;
    fn context(self, kind: IpcErrorKind) -> Self::Output {
        self.map_err(|error| error.context(kind))
    }
}
