//! State: what each Monitor shows, remembered across restarts.
//!
//! A small line-based file under XDG state, written atomically (temp file,
//! fsync, rename). The Daemon parses no Config and no TOML (ADR 0003), so
//! the format is hand-rolled:
//!
//! ```text
//! vellum-state 1
//! wallpaper <monitor> <fit> <absolute path to the end of the line>
//! rotation running
//! deadline <monitor> <unix milliseconds>
//! ```
//!
//! The `rotation` and `deadline` lines hold the Rotation timer, so a
//! started Rotation survives a restart. The Rotation deck and position
//! belong to `vellum`, in a file of its own.
//!
//! Lines are kept in the order the wallpapers were last set, so the last
//! line is the most recent wallpaper: the default for a Monitor Vellum has
//! never seen. Unknown or malformed lines are skipped, so a damaged file
//! costs at most the damaged entries.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use vellum_ipc::{FitMode, Wallpaper};

use crate::rotation::Rotation;

const HEADER: &str = "vellum-state 1";

pub struct State {
    path: Option<PathBuf>,
    /// Monitor name and its wallpaper, least recently set first.
    wallpapers: Vec<(String, Wallpaper)>,
    /// The Rotation timer. Call [`State::save`] after changing it.
    pub rotation: Rotation,
}

impl State {
    /// Loads State from its XDG location. A missing or unreadable file is
    /// an empty State.
    pub fn load() -> State {
        let path = state_path();
        let (wallpapers, rotation) = path
            .as_deref()
            .and_then(|p| fs::read_to_string(p).ok())
            .map(|text| parse(&text))
            .unwrap_or_default();
        State {
            path,
            wallpapers,
            rotation,
        }
    }

    /// The wallpaper remembered for `monitor`.
    pub fn wallpaper(&self, monitor: &str) -> Option<&Wallpaper> {
        self.wallpapers
            .iter()
            .find(|(name, _)| name == monitor)
            .map(|(_, w)| w)
    }

    /// The most recently set wallpaper, for a Monitor with none of its own.
    pub fn latest(&self) -> Option<&Wallpaper> {
        self.wallpapers.last().map(|(_, w)| w)
    }

    /// Remembers `wallpaper` for `monitor` and saves, unless nothing changed.
    pub fn record(&mut self, monitor: &str, wallpaper: Wallpaper) {
        if wallpaper.path.contains('\n') {
            eprintln!("vellumd: not remembering a wallpaper path with a newline in it");
            return;
        }
        if self.wallpapers.last() == Some(&(monitor.to_owned(), wallpaper.clone())) {
            return;
        }
        self.wallpapers.retain(|(name, _)| name != monitor);
        self.wallpapers.push((monitor.to_owned(), wallpaper));
        self.save();
    }

    /// Writes State to disk, logging a failure.
    pub fn save(&self) {
        let Some(path) = &self.path else {
            eprintln!("vellumd: cannot save State: neither XDG_STATE_HOME nor HOME is set");
            return;
        };
        if let Err(err) = write_atomically(path, &format(&self.wallpapers, &self.rotation)) {
            eprintln!("vellumd: cannot save State: {}: {err}", path.display());
        }
    }
}

/// `$XDG_STATE_HOME/vellum/state`, or `~/.local/state/vellum/state`.
fn state_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|dir| Path::new(dir).is_absolute())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/state")))?;
    Some(base.join("vellum").join("state"))
}

fn parse(text: &str) -> (Vec<(String, Wallpaper)>, Rotation) {
    let mut lines = text.lines();
    let mut rotation = Rotation::default();
    if lines.next() != Some(HEADER) {
        return (Vec::new(), rotation);
    }
    let mut wallpapers: Vec<(String, Wallpaper)> = Vec::new();
    for line in lines {
        if line == "rotation running" {
            rotation.running = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("deadline ") {
            if let Some((monitor, Ok(ms))) = rest.split_once(' ').map(|(m, d)| (m, d.parse())) {
                rotation.deadlines.insert(monitor.to_owned(), ms);
            }
            continue;
        }
        let mut fields = line.splitn(4, ' ');
        let (Some("wallpaper"), Some(monitor), Some(fit), Some(path)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let Some(fit) = FitMode::from_name(fit) else {
            continue;
        };
        wallpapers.retain(|(name, _)| name != monitor);
        wallpapers.push((
            monitor.to_owned(),
            Wallpaper {
                path: path.to_owned(),
                fit,
            },
        ));
    }
    if !rotation.running {
        rotation.deadlines.clear();
    }
    (wallpapers, rotation)
}

fn format(wallpapers: &[(String, Wallpaper)], rotation: &Rotation) -> String {
    let mut text = format!("{HEADER}\n");
    for (monitor, w) in wallpapers {
        text += &format!("wallpaper {monitor} {} {}\n", w.fit.name(), w.path);
    }
    if rotation.running {
        text += "rotation running\n";
        for (monitor, ms) in &rotation.deadlines {
            text += &format!("deadline {monitor} {ms}\n");
        }
    }
    text
}

/// Replaces `path` in one rename. Every call writes its own temp file
/// (pid plus counter): Daemons on different displays share the State file,
/// and a shared temp name let one rename the other's file away.
fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = path.parent().expect("the State path has a parent");
    fs::create_dir_all(dir)?;
    let mut name = path
        .file_name()
        .expect("the State path has a name")
        .to_owned();
    let unique = NEXT.fetch_add(1, Ordering::Relaxed);
    name.push(format!(".{}.{unique}.tmp", std::process::id()));
    let tmp = path.with_file_name(name);
    let result = fs::File::create(&tmp)
        .and_then(|mut file| {
            file.write_all(contents.as_bytes())?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wallpaper(path: &str, fit: FitMode) -> Wallpaper {
        Wallpaper {
            path: path.into(),
            fit,
        }
    }

    #[test]
    fn daemons_saving_state_at_once_all_succeed() {
        // Two Daemons on different displays share one State file; neither
        // may rename the other's temp file away.
        let dir = std::env::temp_dir().join(format!("vellumd-state-{}", std::process::id()));
        let path = dir.join("vellum/state");
        std::thread::scope(|scope| {
            for writer in 0..8 {
                let path = &path;
                scope.spawn(move || {
                    for round in 0..100 {
                        let text = format!("{HEADER}\n# writer {writer} round {round}\n");
                        write_atomically(path, &text).unwrap();
                    }
                });
            }
        });
        let text = fs::read_to_string(&path).unwrap();
        let leftovers = fs::read_dir(path.parent().unwrap()).unwrap().count();
        fs::remove_dir_all(&dir).unwrap();
        assert!(text.contains("# writer "), "{text}");
        assert_eq!(leftovers, 1, "temp files were left behind");
    }

    #[test]
    fn state_survives_a_round_trip_through_its_file_format() {
        let wallpapers = vec![
            ("DP-1".to_owned(), wallpaper("/w/a b.png", FitMode::Center)),
            ("eDP-1".to_owned(), wallpaper("/w/c.jpg", FitMode::Fill)),
        ];
        let mut rotation = Rotation {
            running: true,
            ..Rotation::default()
        };
        rotation.deadlines.insert("DP-1".into(), 1_700_000_000_123);
        assert_eq!(
            parse(&format(&wallpapers, &rotation)),
            (wallpapers.clone(), rotation)
        );
        // A stopped Rotation leaves no trace.
        assert_eq!(
            parse(&format(&wallpapers, &Rotation::default())),
            (wallpapers, Rotation::default())
        );
    }

    #[test]
    fn damaged_lines_cost_only_themselves() {
        let text = "vellum-state 1\n\
                    wallpaper DP-1 fill /w/a.png\n\
                    wallpaper DP-2 tile /w/b.png\n\
                    garbage\n\
                    wallpaper DP-3\n\
                    wallpaper DP-4 fit /w/d.png\n";
        assert_eq!(
            parse(text).0,
            [
                ("DP-1".to_owned(), wallpaper("/w/a.png", FitMode::Fill)),
                ("DP-4".to_owned(), wallpaper("/w/d.png", FitMode::Fit)),
            ]
        );
        assert!(
            parse("something else\nwallpaper DP-1 fill /a\n")
                .0
                .is_empty()
        );
    }
}
