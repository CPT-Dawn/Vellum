//! The inotify watch on the Config directory.
//!
//! The Daemon never reads the Config (ADR 0003). It only notices that
//! `config.toml` may have changed, and the Daemon then spawns `vellum
//! render --sync` to read it. The watch copes with a Config directory that
//! does not exist yet (it watches the nearest existing ancestor until it
//! appears), with editors that save by rename, and with a `config.toml`
//! that is a symlink into a dotfiles repository.

use std::ffi::OsStr;
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};

use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
use rustix::io::Errno;

const FILE_NAME: &str = "config.toml";

pub struct ConfigWatch {
    fd: OwnedFd,
    /// The Config directory, or `None` if there is nowhere to look.
    dir: Option<PathBuf>,
    dir_wd: Option<i32>,
    /// `config.toml` itself, followed through symlinks.
    file_wd: Option<i32>,
    /// The nearest existing ancestor, while the directory is missing.
    ancestor_wd: Option<i32>,
}

impl ConfigWatch {
    pub fn new(dir: Option<PathBuf>) -> Result<ConfigWatch, String> {
        let fd = inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)
            .map_err(|err| format!("cannot watch the Config: {err}"))?;
        let mut watch = ConfigWatch {
            fd,
            dir,
            dir_wd: None,
            file_wd: None,
            ancestor_wd: None,
        };
        watch.arm();
        Ok(watch)
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    /// Reads every pending event. Returns whether the Config may have
    /// changed since the last call.
    pub fn changed(&mut self) -> bool {
        let mut buf = [MaybeUninit::<u8>::uninit(); 4096];
        let mut changed = false;
        let mut rearm = false;
        let mut reader = inotify::Reader::new(&self.fd, &mut buf);
        loop {
            let event = match reader.next() {
                Ok(event) => event,
                Err(Errno::INTR) => continue,
                Err(_) => break, // WOULDBLOCK: drained
            };
            let wd = Some(event.wd());
            let flags = event.events();
            if flags.contains(ReadFlags::QUEUE_OVERFLOW) {
                changed = true;
                rearm = true;
            } else if wd == self.dir_wd {
                if flags
                    .intersects(ReadFlags::IGNORED | ReadFlags::DELETE_SELF | ReadFlags::MOVE_SELF)
                {
                    self.dir_wd = None;
                    changed = true;
                    rearm = true;
                } else if event
                    .file_name()
                    .is_some_and(|name| name.to_bytes() == FILE_NAME.as_bytes())
                {
                    changed = true;
                    rearm = true; // (re)watch the file itself
                }
            } else if wd == self.file_wd {
                if flags
                    .intersects(ReadFlags::IGNORED | ReadFlags::DELETE_SELF | ReadFlags::MOVE_SELF)
                {
                    self.file_wd = None;
                    rearm = true;
                }
                changed = true;
            } else if wd == self.ancestor_wd {
                rearm = true;
            }
        }
        if rearm && self.arm() {
            changed = true;
        }
        changed
    }

    /// Watches the directory (or the nearest ancestor while it is missing)
    /// and the file. Returns whether the directory watch was just set up,
    /// which means the Config may have appeared.
    fn arm(&mut self) -> bool {
        let Some(dir) = self.dir.clone() else {
            return false;
        };
        let mut appeared = false;
        if self.dir_wd.is_none() {
            appeared = self.watch_dir(&dir);
            if !appeared {
                self.watch_ancestor(&dir);
                // It may have appeared before the ancestor watch was in place.
                appeared = self.watch_dir(&dir);
            }
        }
        if self.file_wd.is_none() && self.dir_wd.is_some() {
            let flags = WatchFlags::CLOSE_WRITE | WatchFlags::DELETE_SELF | WatchFlags::MOVE_SELF;
            self.file_wd = inotify::add_watch(&self.fd, dir.join(FILE_NAME), flags).ok();
        }
        appeared
    }

    /// Watches the Config directory itself, if it exists.
    fn watch_dir(&mut self, dir: &Path) -> bool {
        let flags = WatchFlags::CLOSE_WRITE
            | WatchFlags::MOVED_TO
            | WatchFlags::MOVED_FROM
            | WatchFlags::DELETE
            | WatchFlags::DELETE_SELF
            | WatchFlags::MOVE_SELF
            | WatchFlags::ONLYDIR;
        let Ok(wd) = inotify::add_watch(&self.fd, dir, flags) else {
            return false;
        };
        self.dir_wd = Some(wd);
        if let Some(old) = self.ancestor_wd.take() {
            let _ = inotify::remove_watch(&self.fd, old);
        }
        true
    }

    /// Watches the deepest existing ancestor of `dir` for new entries.
    fn watch_ancestor(&mut self, dir: &Path) {
        let flags = WatchFlags::CREATE | WatchFlags::MOVED_TO | WatchFlags::ONLYDIR;
        for ancestor in dir.ancestors().skip(1) {
            if ancestor.as_os_str() == OsStr::new("") {
                break;
            }
            if let Ok(wd) = inotify::add_watch(&self.fd, ancestor, flags) {
                if let Some(old) = self.ancestor_wd.replace(wd)
                    && old != wd
                {
                    let _ = inotify::remove_watch(&self.fd, old);
                }
                return;
            }
        }
    }
}
