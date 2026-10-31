//! The Vellum wire protocol, shared by `vellumd` (the Daemon) and `vellum`.
//!
//! This crate is the only place the wire format is defined.
//!
//! # Frames
//!
//! Every message travels as one frame. All integers are little-endian.
//!
//! ```text
//! offset  size  field
//! 0       4     magic, the bytes "VLUM"
//! 4       2     protocol version (PROTOCOL_VERSION)
//! 6       2     message kind
//! 8       4     payload length in bytes
//! 12      n     payload
//! ```
//!
//! The header layout is frozen. A peer that speaks another protocol version
//! can always read the header, so it can report a clean version mismatch
//! instead of misparsing the payload.
//!
//! File descriptors, such as a frame's pixel memfd, travel alongside the
//! frame as `SCM_RIGHTS` ancillary data. See [`send`] and [`recv`].
//!
//! # Connections
//!
//! A client connects to [`socket_path`], sends one [`Request`] and reads one
//! [`Response`]. [`request`] does all of that in one call.

mod codec;
mod error;
mod message;
mod transport;

use std::path::{Path, PathBuf};

pub use codec::{HEADER_LEN, MAGIC, MAX_PAYLOAD, Message, PROTOCOL_VERSION};
pub use error::Error;
pub use message::{MonitorInfo, PixelFormat, Present, Request, Response};
pub use transport::{recv, request, send};

/// The Daemon's socket for a Wayland display:
/// `$XDG_RUNTIME_DIR/vellum-$WAYLAND_DISPLAY.sock`.
///
/// A `WAYLAND_DISPLAY` given as an absolute path contributes only its file
/// name, so the socket always lives directly in the runtime directory.
pub fn socket_path(runtime_dir: &Path, wayland_display: &str) -> PathBuf {
    let display = Path::new(wayland_display)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(wayland_display);
    runtime_dir.join(format!("vellum-{display}.sock"))
}

/// [`socket_path`] for the current environment.
pub fn socket_path_from_env() -> Result<PathBuf, Error> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").ok_or(Error::NoRuntimeDir)?;
    let display = std::env::var("WAYLAND_DISPLAY").map_err(|_| Error::NoWaylandDisplay)?;
    Ok(socket_path(Path::new(&runtime_dir), &display))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_lives_in_the_runtime_dir_named_after_the_display() {
        let dir = Path::new("/run/user/1000");
        assert_eq!(
            socket_path(dir, "wayland-1"),
            Path::new("/run/user/1000/vellum-wayland-1.sock")
        );
        assert_eq!(
            socket_path(dir, "/tmp/nested/wayland-9"),
            Path::new("/run/user/1000/vellum-wayland-9.sock")
        );
    }
}
