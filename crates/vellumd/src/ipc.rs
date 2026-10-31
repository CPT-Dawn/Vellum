//! The Daemon's IPC socket: one request and one response per connection.

use std::io::ErrorKind;
use std::os::fd::OwnedFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use vellum_ipc::{Request, Response};

/// How long a client may take to send its request.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(2);

/// The listening socket. Removes its path when dropped.
pub struct Listener {
    listener: UnixListener,
    path: PathBuf,
}

impl Listener {
    /// Binds `path`, refusing if another Daemon already answers there. A
    /// stale socket left by a crashed Daemon is replaced.
    pub fn bind(path: &Path) -> Result<Listener, String> {
        if path.exists() {
            if UnixStream::connect(path).is_ok() {
                return Err(format!(
                    "another vellumd is already running on this display ({})",
                    path.display()
                ));
            }
            let _ = std::fs::remove_file(path);
        }
        let listener = UnixListener::bind(path)
            .map_err(|err| format!("cannot listen on {}: {err}", path.display()))?;
        listener
            .set_nonblocking(true)
            .map_err(|err| err.to_string())?;
        Ok(Listener {
            listener,
            path: path.to_owned(),
        })
    }

    pub fn listener(&self) -> &UnixListener {
        &self.listener
    }

    /// Accepts the next pending client, if any.
    pub fn accept(&self) -> Option<Client> {
        match self.listener.accept() {
            Ok((stream, _)) => Some(Client { stream }),
            Err(err) if err.kind() == ErrorKind::WouldBlock => None,
            Err(err) => {
                eprintln!("vellumd: accept failed: {err}");
                None
            }
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A connected client.
pub struct Client {
    stream: UnixStream,
}

impl Client {
    /// Reads the client's request and the fds attached to it.
    pub fn read(&self) -> Result<(Request, Vec<OwnedFd>), vellum_ipc::Error> {
        self.stream.set_nonblocking(false)?;
        self.stream.set_read_timeout(Some(CLIENT_TIMEOUT))?;
        self.stream.set_write_timeout(Some(CLIENT_TIMEOUT))?;
        vellum_ipc::recv(&self.stream)
    }

    pub fn reply(&self, response: &Response) {
        if let Err(err) = vellum_ipc::send(&self.stream, response, &[]) {
            eprintln!("vellumd: cannot reply to client: {err}");
        }
    }
}
