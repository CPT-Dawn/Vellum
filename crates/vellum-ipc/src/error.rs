use std::fmt;
use std::io;

/// Everything that can go wrong while talking the Vellum protocol.
#[derive(Debug)]
pub enum Error {
    /// The socket failed.
    Io(io::Error),
    /// The peer closed the connection before a whole frame arrived.
    Closed,
    /// The bytes do not start with the Vellum magic number.
    BadMagic,
    /// The peer speaks another protocol version.
    VersionMismatch { ours: u16, theirs: u16 },
    /// The frame is shorter than its header or fields claim.
    Truncated,
    /// The payload is longer than [`MAX_PAYLOAD`](crate::MAX_PAYLOAD).
    TooLarge(u32),
    /// The message kind is not known to this protocol version.
    UnknownKind(u16),
    /// A field holds a value outside its allowed set.
    BadValue(&'static str),
    /// A string field is not valid UTF-8.
    BadUtf8,
    /// `XDG_RUNTIME_DIR` is not set.
    NoRuntimeDir,
    /// `WAYLAND_DISPLAY` is not set.
    NoWaylandDisplay,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(err) => write!(f, "{err}"),
            Error::Closed => f.write_str("the connection closed mid-message"),
            Error::BadMagic => f.write_str("the peer does not speak the Vellum protocol"),
            Error::VersionMismatch { ours, theirs } => write!(
                f,
                "protocol version mismatch: the peer speaks v{theirs}, this build speaks v{ours}; \
                 restart vellumd so both come from the same release"
            ),
            Error::Truncated => f.write_str("the message is truncated"),
            Error::TooLarge(len) => write!(f, "the message is too large ({len} bytes)"),
            Error::UnknownKind(kind) => write!(f, "unknown message kind {kind:#06x}"),
            Error::BadValue(field) => write!(f, "invalid value for {field}"),
            Error::BadUtf8 => f.write_str("a text field is not valid UTF-8"),
            Error::NoRuntimeDir => f.write_str("XDG_RUNTIME_DIR is not set"),
            Error::NoWaylandDisplay => {
                f.write_str("WAYLAND_DISPLAY is not set; is a Wayland compositor running?")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::Io(err)
    }
}

impl From<rustix::io::Errno> for Error {
    fn from(err: rustix::io::Errno) -> Self {
        Error::Io(err.into())
    }
}
