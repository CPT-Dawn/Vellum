//! Replacing a file in one rename, so a reader never sees half of it.
//!
//! Every file `vellum` writes goes through [`write_atomically`]. Several
//! `vellum` processes often save the same file at once (`vellum reload` and
//! the Daemon's `render --sync`, two TUIs filling the preview cache), so a
//! fixed temp name is never safe: one writer renames the other's file away.

use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

/// Replaces `path` with `bytes`, creating its directory. The temp file is
/// `.<name>.<pid>.<n>.tmp` next to `path`: unique per call, and hidden, so
/// directory listings (the preview cache) skip it.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    std::fs::create_dir_all(path.parent().expect("a parent dir"))?;
    let unique = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut name = std::ffi::OsString::from(".");
    name.push(path.file_name().expect("a file name"));
    name.push(format!(".{}.{unique}.tmp", std::process::id()));
    let tmp = path.with_file_name(name);
    let result = std::fs::File::create(&tmp)
        .and_then(|mut file| file.write_all(bytes))
        .and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_saves_of_one_file_all_succeed() {
        // `vellum reload` and the Daemon's inotify `render --sync` can save
        // the last good Config at the same moment; neither may lose its
        // rename.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/vellum/config.last-good.toml");
        std::thread::scope(|scope| {
            for writer in 0..8 {
                let path = &path;
                scope.spawn(move || {
                    for round in 0..200 {
                        let text = format!("# writer {writer} round {round}\n");
                        write_atomically(path, text.as_bytes()).unwrap();
                    }
                });
            }
        });
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# writer "), "{text}");
        let files = std::fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(files, 1, "temp files were left behind");
    }

    #[test]
    fn a_failed_write_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        // Renaming a file over a directory fails.
        let path = dir.path().join("taken");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("inside"), b"x").unwrap();
        assert!(write_atomically(&path, b"data").is_err());
        let files = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(files, 1, "the temp file was left behind");
    }
}
