include!(concat!(env!("OUT_DIR"), "/wayland_protocols.rs"));

use common::log;
use waybackend::{Waybackend, objman::ObjectManager, wire::Receiver};

use crate::WaylandObject;

pub fn connect() -> (Waybackend, ObjectManager<WaylandObject>, Receiver) {
    use rustix::fd::{FromRawFd, OwnedFd};
    use rustix::net::AddressFamily;

    if let Some(txt) = unsafe { common::getenv(c"WAYLAND_SOCKET") } {
        // We should connect to the provided WAYLAND_SOCKET
        let fd =
            parse_cstr_to_rawfd(txt).expect("file descriptor in WAYLAND_SOCKET is not a number");

        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let socket_addr = rustix::net::getsockname(&fd).expect("failed to getsocketname");
        if socket_addr.address_family() == AddressFamily::UNIX {
            unsafe { waybackend::connect_from_fd(WaylandObject::Display, fd) }
        } else {
            panic!(
                "Socket in WAYLAND_SOCKET has wrong family: {}",
                socket_addr.address_family().as_raw()
            );
        }
    } else {
        let socket_name = unsafe { common::getenv(c"WAYLAND_DISPLAY") }.unwrap_or_else(|| {
            log::warn!("WAYLAND_DISPLAY is not set! Defaulting to wayland-0");
            c"wayland-0"
        });

        let unix_addr = if socket_name.to_bytes().first() == Some(&b'/') {
            rustix::net::SocketAddrUnix::new(socket_name).unwrap()
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
            rustix::net::SocketAddrUnix::new(socket_fullpath).unwrap()
        };

        let socket = rustix::net::socket_with(
            rustix::net::AddressFamily::UNIX,
            rustix::net::SocketType::STREAM,
            rustix::net::SocketFlags::CLOEXEC,
            None,
        )
        .expect("failed to create socket");

        waybackend::connect_to(WaylandObject::Display, socket, &unix_addr)
            .expect("failed to connect to socket")
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
