include!(concat!(env!("OUT_DIR"), "/wayland_protocols.rs"));

use common::log;
use thiserror::Error;
use waybackend::{Waybackend, objman::ObjectManager, wire::Receiver};

use crate::WaylandObject;

#[derive(Debug, Error)]
pub enum WaylandConnectError {
    #[error("file descriptor in WAYLAND_SOCKET is not a number")]
    InvalidWaylandSocket,
    #[error("failed to get WAYLAND_SOCKET metadata: {0}")]
    GetSocketName(String),
    #[error("WAYLAND_SOCKET has unsupported family: {0}")]
    WrongSocketFamily(u32),
    #[error("invalid Wayland socket address: {0}")]
    InvalidSocketAddress(String),
    #[error("failed to create Wayland socket: {0}")]
    CreateSocket(String),
    #[error("failed to connect to Wayland socket: {0}")]
    ConnectSocket(String),
}

pub fn connect() -> Result<(Waybackend, ObjectManager<WaylandObject>, Receiver), WaylandConnectError>
{
    use rustix::fd::{FromRawFd, OwnedFd};
    use rustix::net::AddressFamily;

    if let Some(txt) = unsafe { common::getenv(c"WAYLAND_SOCKET") } {
        // We should connect to the provided WAYLAND_SOCKET
        let fd = parse_cstr_to_rawfd(txt).ok_or(WaylandConnectError::InvalidWaylandSocket)?;

        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let socket_addr = rustix::net::getsockname(&fd)
            .map_err(|err| WaylandConnectError::GetSocketName(err.to_string()))?;
        if socket_addr.address_family() == AddressFamily::UNIX {
            Ok(unsafe { waybackend::connect_from_fd(WaylandObject::Display, fd) })
        } else {
            Err(WaylandConnectError::WrongSocketFamily(u32::from(
                socket_addr.address_family().as_raw(),
            )))
        }
    } else {
        let socket_name = unsafe { common::getenv(c"WAYLAND_DISPLAY") }.unwrap_or_else(|| {
            log::warn!("WAYLAND_DISPLAY is not set! Defaulting to wayland-0");
            c"wayland-0"
        });

        let unix_addr = if socket_name.to_bytes().first() == Some(&b'/') {
            rustix::net::SocketAddrUnix::new(socket_name)
                .map_err(|err| WaylandConnectError::InvalidSocketAddress(err.to_string()))?
        } else {
            let mut socket_fullpath = common::path::PathBuf::new();
            match unsafe { common::getenv(c"XDG_RUNTIME_DIR") } {
                Some(socket_path) => socket_fullpath.push_cstr(socket_path),
                None => {
                    use rustix::path::DecInt;
                    log::warn!("XDG_RUNTIME_DIR is not set! Defaulting to /run/user/UID");
                    let uid = rustix::process::getuid();
                    socket_fullpath.push_cstr(c"/run/user");
                    socket_fullpath.push_cstr(DecInt::new(uid.as_raw()).as_c_str());
                }
            }
            socket_fullpath.push_cstr(socket_name);
            rustix::net::SocketAddrUnix::new(socket_fullpath)
                .map_err(|err| WaylandConnectError::InvalidSocketAddress(err.to_string()))?
        };

        let socket = rustix::net::socket_with(
            rustix::net::AddressFamily::UNIX,
            rustix::net::SocketType::STREAM,
            rustix::net::SocketFlags::CLOEXEC,
            None,
        )
        .map_err(|err| WaylandConnectError::CreateSocket(err.to_string()))?;

        waybackend::connect_to(WaylandObject::Display, socket, &unix_addr)
            .map_err(|err| WaylandConnectError::ConnectSocket(err.to_string()))
    }
}

/// This function is unlikely to run, as most wayland implementations use WAYLAND_DISPLAY, not
/// WAYLAND_SOCKET
///
/// We are writing our own manual implementation because Rust cannot parse a `cstr` directly.
/// Instead, it demands we first transform it to a str (which goes through a utf8 verification),
/// and THEN try parsing the number, therefore generating code with 2 unwraps and panic conditions,
/// even though 1 would suffice
#[cold]
fn parse_cstr_to_rawfd(s: &core::ffi::CStr) -> Option<rustix::fd::RawFd> {
    let mut fd: rustix::fd::RawFd = 0;
    let mut ptr = s.as_ptr();

    loop {
        let x = unsafe { ptr.read() } as core::ffi::c_int;
        if x == 0 {
            break;
        } else if x < b'0' as core::ffi::c_int || x > b'9' as core::ffi::c_int {
            return None;
        }
        fd = fd * 10 + (x - b'0' as core::ffi::c_int);
        ptr = unsafe { ptr.add(1) };
    }

    Some(fd)
}

#[cfg(test)]
mod tests {
    #[test]
    fn parse_wayland_socket_envvar() {
        use super::parse_cstr_to_rawfd as parse;
        assert_eq!(parse(c"1"), Some(1));
        assert!(parse(c" 1").is_none());
        assert!(parse(c"1 ").is_none());
        assert_eq!(parse(c"5"), Some(5));
        assert!(parse(c"5 ").is_none());
        assert_eq!(parse(c"12"), Some(12));
        assert_eq!(parse(c"165"), Some(165));
    }
}
