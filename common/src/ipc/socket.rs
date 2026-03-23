use ::alloc::boxed::Box;
use ::alloc::string::String;
use ::alloc::vec::Vec;

use core::time::Duration;

use rustix::fd::OwnedFd;
use rustix::fs;
use rustix::io::Errno;
use rustix::net;
use rustix::time::Timespec;

use crate::log;
use crate::path::Path;
use crate::path::PathBuf;

use super::ErrnoExt;
use super::IpcError;
use super::IpcErrorKind;

fn get_socket_path_or_init() -> &'static Path {
    use core::ffi::{CStr, c_char};
    use core::sync::atomic;

    static INITIAL_PATH: &CStr = c"";
    static SOCKET_PATH: atomic::AtomicPtr<c_char> =
        atomic::AtomicPtr::new(INITIAL_PATH.as_ptr().cast_mut());

    if SOCKET_PATH.load(atomic::Ordering::Relaxed) == INITIAL_PATH.as_ptr().cast_mut() {
        let path = Box::leak(IpcSocket::socket_file().into_c_string().into_boxed_c_str());
        SOCKET_PATH.store(path.as_ptr().cast_mut(), atomic::Ordering::Relaxed);
    }

    // SAFETY: even we somehow get here without initializing, the worse that can happen is we use
    // an incorrect empty path, which will cause the first syscall we use it with to fail
    Path::from_cstr(unsafe { CStr::from_ptr(SOCKET_PATH.load(atomic::Ordering::Relaxed)) })
}

pub struct IpcSocket {
    fd: OwnedFd,
}

impl IpcSocket {
    /// Creates new [`IpcSocket`] from provided [`OwnedFd`]
    ///
    /// TODO: remove external ability to construct [`Self`] from random file descriptors
    #[must_use]
    pub fn new(fd: OwnedFd) -> Self {
        Self { fd }
    }

    #[must_use]
    pub fn to_fd(self) -> OwnedFd {
        self.fd
    }

    fn socket_file() -> PathBuf {
        let mut runtime: PathBuf = unsafe { crate::getenv(c"XDG_RUNTIME_DIR") }.map_or_else(
            || {
                use rustix::path::DecInt;
                let mut p = PathBuf::from(c"/run/user");
                let uid = rustix::process::getuid();
                p.push_cstr(DecInt::new(uid.as_raw()).as_c_str());
                p
            },
            <PathBuf as From<&core::ffi::CStr>>::from,
        );

        if let Some(wayland_socket) = unsafe { crate::getenv(c"WAYLAND_DISPLAY") } {
            let mut path = Path::from_cstr(wayland_socket);
            if let Some(final_component) = path.file_name() {
                path = Path::from_cstr(final_component);
            }
            runtime.push_cstr(path.as_c_str());
            runtime.append_cstr(c"-vellum-daemon");
        } else {
            log::warn!("WAYLAND_DISPLAY variable not set. Defaulting to wayland-0");
            runtime.push_cstr(c"wayland-0-vellum-daemon");
        }

        runtime
    }

    /// Retrieves path to socket file
    ///
    /// If you get errors with missing generics, you can shove any type as `T`, but
    /// [`Client`] or [`Server`] are recommended.
    #[must_use]
    pub fn path(namespace: &str) -> PathBuf {
        let mut p = PathBuf::from(get_socket_path_or_init());
        if !namespace.is_empty() {
            p.append_cstr(c".");
            p.append_str(namespace);
        }
        p.append_cstr(c".sock");
        p
    }

    /// Retrieves all currently in-use namespaces
    pub fn all_namespaces() -> rustix::io::Result<Vec<String>> {
        let p = get_socket_path_or_init();
        let parent = match p.parent() {
            Some(parent) => parent,
            None => return Ok(Vec::new()),
        };

        let filename = match p.file_name() {
            Some(filename) => filename,
            None => return Err(Errno::NOENT),
        };

        let dir = fs::Dir::new(fs::open(parent, fs::OFlags::RDONLY, fs::Mode::RUSR)?)?;

        Ok(dir
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let mut namespace = entry
                    .file_name()
                    .to_bytes()
                    .strip_suffix(b".sock")?
                    .strip_prefix(filename.to_bytes())?;
                namespace = namespace.strip_prefix(b".").unwrap_or(namespace);
                String::from_utf8(namespace.to_vec()).ok()
            })
            .collect())
    }

    #[must_use]
    pub fn as_fd(&self) -> &OwnedFd {
        &self.fd
    }

    /// Connects to already running `Daemon`, if there is one.
    pub fn client(namespace: &str) -> Result<Self, IpcError> {
        const ATTEMPTS: usize = 5;
        const INTERVAL: Timespec = Timespec {
            tv_sec: 0,
            tv_nsec: 100_000_000,
        };

        let socket = net::socket_with(
            net::AddressFamily::UNIX,
            net::SocketType::STREAM,
            net::SocketFlags::CLOEXEC,
            None,
        )
        .context(IpcErrorKind::Socket)?;

        let path = Self::path(namespace);
        let addr = net::SocketAddrUnix::new(&path).context(IpcErrorKind::SocketAddr)?;

        // this will be overwritten, Rust just doesn't know it
        let mut error = Errno::INVAL;
        for _ in 0..ATTEMPTS {
            match net::connect(&socket, &addr) {
                Ok(()) => {
                    #[cfg(debug_assertions)]
                    let timeout = Duration::from_secs(30); //Some operations take a while to respond in debug mode
                    #[cfg(not(debug_assertions))]
                    let timeout = Duration::from_secs(5);
                    return net::sockopt::set_socket_timeout(
                        &socket,
                        net::sockopt::Timeout::Recv,
                        Some(timeout),
                    )
                    .context(IpcErrorKind::SetTimeout)
                    .map(|()| Self::new(socket));
                }
                Err(e) => error = e,
            }
            let _ = rustix::thread::nanosleep(&INTERVAL);
        }

        let kind = if error == Errno::NOENT {
            IpcErrorKind::NoSocketFile(path.into_boxed_path())
        } else {
            IpcErrorKind::Connect
        };

        Err(error.context(kind))
    }

    /// Creates [`IpcSocket`] for use in server (i.e `Daemon`)
    pub fn server(namespace: &str) -> Result<Self, IpcError> {
        let addr =
            net::SocketAddrUnix::new(Self::path(namespace)).context(IpcErrorKind::SocketAddr)?;
        let socket = net::socket_with(
            net::AddressFamily::UNIX,
            net::SocketType::STREAM,
            net::SocketFlags::CLOEXEC.union(rustix::net::SocketFlags::NONBLOCK),
            None,
        )
        .context(IpcErrorKind::Socket)?;
        net::bind(&socket, &addr).context(IpcErrorKind::Bind)?;
        net::listen(&socket, 0).context(IpcErrorKind::Listen)?;
        Ok(Self::new(socket))
    }
}
