//! The Daemon's single-threaded poll loop.
//!
//! Every event source is one entry in [`Source`]. The loop polls them all
//! with an infinite timeout, so an idle Daemon makes zero wakeups. To add a
//! source (the Config inotify watch, the Rotation timerfd), add a variant,
//! push its fd in [`Daemon::run`] and handle it in the match.

use std::os::fd::{AsFd, OwnedFd};

use rustix::event::{PollFd, PollFlags, poll};
use vellum_ipc::{Request, Response};

use crate::ipc::{Client, Listener};
use crate::wayland::Wayland;

/// The fds the loop waits on, in poll order.
#[derive(Clone, Copy)]
enum Source {
    Wayland,
    Ipc,
}

const SOURCES: [Source; 2] = [Source::Wayland, Source::Ipc];

pub struct Daemon {
    wayland: Wayland,
    listener: Listener,
}

impl Daemon {
    pub fn new(wayland: Wayland, listener: Listener) -> Daemon {
        Daemon { wayland, listener }
    }

    /// Runs until the compositor goes away. Returns `Ok` on a clean
    /// disconnect.
    pub fn run(mut self) -> Result<(), String> {
        loop {
            self.wayland.flush()?;
            let ready = {
                let wayland_fd = self.wayland.fd();
                let mut fds = SOURCES.map(|source| {
                    let fd = match source {
                        Source::Wayland => wayland_fd.as_fd(),
                        Source::Ipc => self.listener.listener().as_fd(),
                    };
                    PollFd::from_borrowed_fd(fd, PollFlags::IN)
                });
                match poll(&mut fds, None) {
                    Ok(_) => {}
                    Err(rustix::io::Errno::INTR) => continue,
                    Err(err) => return Err(format!("poll failed: {err}")),
                }
                fds.map(|fd| fd.revents())
            };

            for (source, revents) in SOURCES.into_iter().zip(ready) {
                if revents.is_empty() {
                    continue;
                }
                match source {
                    Source::Wayland => {
                        if revents.intersects(PollFlags::ERR | PollFlags::HUP)
                            && !revents.contains(PollFlags::IN)
                        {
                            eprintln!("vellumd: the compositor went away; exiting");
                            return Ok(());
                        }
                        if let Err(err) = self.wayland.dispatch() {
                            eprintln!("vellumd: {err}; exiting");
                            return Ok(());
                        }
                    }
                    Source::Ipc => {
                        while let Some(client) = self.listener.accept() {
                            self.serve(client);
                        }
                    }
                }
            }
        }
    }

    fn serve(&mut self, client: Client) {
        let (request, fds) = match client.read() {
            Ok(read) => read,
            Err(err) => {
                // A version mismatch still gets a readable answer: the client
                // decodes our header and reports both versions.
                client.reply(&Response::Error(err.to_string()));
                return;
            }
        };
        let response = self.handle(request, fds);
        client.reply(&response);
    }

    /// Answers one request. Add new request kinds here.
    fn handle(&mut self, request: Request, mut fds: Vec<OwnedFd>) -> Response {
        match request {
            Request::Monitors => Response::Monitors(self.wayland.monitors()),
            Request::Present(frame) => {
                if fds.len() != 1 {
                    return Response::Error(format!(
                        "a present request needs exactly one frame fd, got {}",
                        fds.len()
                    ));
                }
                let memfd = fds.pop().expect("checked above");
                match self.wayland.present(&frame, memfd) {
                    Ok(()) => Response::Ok,
                    Err(err) => Response::Error(err),
                }
            }
        }
    }
}
