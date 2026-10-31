//! The Daemon's single-threaded poll loop.
//!
//! Every event source is one entry in [`Source`]. The loop polls them all
//! with an infinite timeout, so an idle Daemon makes zero wakeups. To add a
//! source (the Rotation timerfd), add a variant,
//! push its fd in [`Daemon::run`] and handle it in the match. Running
//! `vellum render` children are polled after the fixed sources, one pidfd
//! each, so they are reaped the moment they exit.
//!
//! Whenever a Monitor lacks a frame for its current size (startup, hotplug,
//! a scale change) and State has a wallpaper for it, the Daemon spawns
//! `vellum render --restore --monitor <name>`.
//!
//! At startup, and whenever the inotify watch on the Config directory fires,
//! it spawns `vellum render --sync`, which reads the Config and pushes
//! [`Settings`] back. Only one sync runs at a time; a change while one runs
//! queues exactly one more, so the last sync always reads the newest file.

use std::os::fd::{AsFd, OwnedFd};

use rustix::event::{PollFd, PollFlags, poll};
use vellum_ipc::{MonitorStatus, Request, Response, Settings, Status, Wallpaper};

use crate::config_watch::ConfigWatch;
use crate::ipc::{Client, Listener};
use crate::state::State;
use crate::system::{self, Job, Renderer};
use crate::wayland::Wayland;

/// The fds the loop waits on, in poll order.
#[derive(Clone, Copy)]
enum Source {
    Wayland,
    Ipc,
    Config,
}

const SOURCES: [Source; 3] = [Source::Wayland, Source::Ipc, Source::Config];

pub struct Daemon {
    wayland: Wayland,
    listener: Listener,
    config: ConfigWatch,
    state: State,
    /// The last good Config values `vellum render --sync` pushed.
    settings: Settings,
    renderers: Vec<Renderer>,
    /// The Config may have changed since the running sync (if any) read it.
    sync_wanted: bool,
    /// Set by a kill request: exit once the reply is out.
    stopping: bool,
}

impl Daemon {
    pub fn new(wayland: Wayland, listener: Listener, config: ConfigWatch, state: State) -> Daemon {
        Daemon {
            wayland,
            listener,
            config,
            state,
            settings: Settings::default(),
            renderers: Vec::new(),
            sync_wanted: true,
            stopping: false,
        }
    }

    /// Runs until the compositor goes away or a kill request arrives.
    /// Returns `Ok` on a clean exit.
    pub fn run(mut self) -> Result<(), String> {
        loop {
            self.restore_missing_frames();
            self.start_sync();
            self.wayland.flush()?;
            let (ready, exited) = {
                let wayland_fd = self.wayland.fd();
                let mut fds: Vec<PollFd<'_>> = SOURCES
                    .iter()
                    .map(|source| match source {
                        Source::Wayland => wayland_fd.as_fd(),
                        Source::Ipc => self.listener.listener().as_fd(),
                        Source::Config => self.config.fd(),
                    })
                    .chain(self.renderers.iter().map(|r| r.pidfd.as_fd()))
                    .map(|fd| PollFd::from_borrowed_fd(fd, PollFlags::IN))
                    .collect();
                match poll(&mut fds, None) {
                    Ok(_) => {}
                    Err(rustix::io::Errno::INTR) => continue,
                    Err(err) => return Err(format!("poll failed: {err}")),
                }
                let revents: Vec<PollFlags> = fds.iter().map(|fd| fd.revents()).collect();
                let exited: Vec<bool> = revents[SOURCES.len()..]
                    .iter()
                    .map(|r| !r.is_empty())
                    .collect();
                (revents, exited)
            };
            self.reap(&exited);

            for (source, &revents) in SOURCES.into_iter().zip(&ready) {
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
                            if self.stopping {
                                eprintln!("vellumd: asked to stop; exiting");
                                return Ok(());
                            }
                        }
                    }
                    Source::Config => {
                        if self.config.changed() {
                            self.sync_wanted = true;
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

    /// Spawns a restore for every Monitor that needs a frame State can
    /// provide. A Monitor Vellum has never seen gets the latest wallpaper.
    fn restore_missing_frames(&mut self) {
        for monitor in self.wayland.take_frame_needs() {
            let wallpaper = match self.state.wallpaper(&monitor) {
                Some(_) => None,
                None => match self.state.latest() {
                    Some(latest) => Some(latest.clone()),
                    None => continue,
                },
            };
            if let Some(wallpaper) = wallpaper {
                self.state.record(&monitor, wallpaper);
            }
            // A restore still running for an older size presents a frame
            // that `wants_frame` drops, so overlapping restores are safe.
            match system::spawn_render(Job::Restore(monitor.clone())) {
                Ok(renderer) => self.renderers.push(renderer),
                Err(err) => eprintln!("vellumd: {monitor}: {err}"),
            }
        }
    }

    /// Spawns `vellum render --sync` if the Config may have changed and no
    /// sync is running.
    fn start_sync(&mut self) {
        if !self.sync_wanted || self.renderers.iter().any(|r| r.job == Job::Sync) {
            return;
        }
        self.sync_wanted = false;
        match system::spawn_render(Job::Sync) {
            Ok(renderer) => self.renderers.push(renderer),
            Err(err) => eprintln!("vellumd: {err}"),
        }
    }

    /// Reaps the renderers whose pidfd turned readable.
    fn reap(&mut self, exited: &[bool]) {
        let mut index = 0;
        self.renderers.retain_mut(|renderer| {
            let done = exited.get(index).copied().unwrap_or(false);
            index += 1;
            if !done {
                return true;
            }
            match renderer.child.wait() {
                Ok(status) if !status.success() => {
                    eprintln!("vellumd: {} failed ({status})", renderer.job)
                }
                Ok(_) => {}
                Err(err) => eprintln!("vellumd: cannot reap `vellum render`: {err}"),
            }
            false
        });
    }

    fn status(&self) -> Status {
        Status {
            version: env!("CARGO_PKG_VERSION").into(),
            pid: std::process::id(),
            rss_bytes: system::rss_bytes(),
            monitors: self
                .wayland
                .monitors()
                .into_iter()
                .map(|m| MonitorStatus {
                    wallpaper: self.state.wallpaper(&m.name).cloned(),
                    name: m.name,
                })
                .collect(),
            settings: self.settings.clone(),
        }
    }

    /// Answers one request. Add new request kinds here.
    fn handle(&mut self, request: Request, mut fds: Vec<OwnedFd>) -> Response {
        match request {
            Request::Monitors => Response::Monitors(self.wayland.monitors()),
            Request::Status => Response::Status(self.status()),
            Request::Settings(settings) => {
                self.settings = settings;
                Response::Ok
            }
            Request::Kill => {
                self.stopping = true;
                Response::Ok
            }
            Request::Present(frame) => {
                if fds.len() != 1 {
                    return Response::Error(format!(
                        "a present request needs exactly one frame fd, got {}",
                        fds.len()
                    ));
                }
                let memfd = fds.pop().expect("checked above");
                if frame.restore && !self.wayland.wants_frame(&frame) {
                    // A newer frame got there first, or the Monitor changed
                    // size and another restore is on its way.
                    return Response::Ok;
                }
                match self.wayland.present(&frame, memfd) {
                    Ok(()) => {
                        let wallpaper = Wallpaper {
                            path: frame.source,
                            fit: frame.fit,
                        };
                        self.state.record(&frame.monitor, wallpaper);
                        Response::Ok
                    }
                    Err(err) => Response::Error(err),
                }
            }
        }
    }
}
