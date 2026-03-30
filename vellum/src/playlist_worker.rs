use std::collections::HashMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::app::ScalingMode;
use crate::backend::Backend;

const PLAYLIST_STATE_FILENAME: &str = "playlist-state-v1.txt";
const FAVORITES_STATE_FILENAME: &str = "favorites-v1.txt";
const TUI_ACTIVE_FILENAME: &str = "tui-active-v1.lock";
const WORKER_PID_FILENAME: &str = "playlist-worker-v1.pid";
const POLL_SLEEP: Duration = Duration::from_secs(1);
const PLAYLIST_INTERVAL_MIN_SECS: u64 = 10;
const PLAYLIST_INTERVAL_MAX_SECS: u64 = 99 * 3600;

#[derive(Clone, Copy)]
enum PlaylistSource {
    Workspace,
    Favorites,
}

struct PlaylistEntry {
    monitor_name: String,
    source: PlaylistSource,
    interval_secs: u64,
    running: bool,
}

#[derive(Default)]
struct RuntimeState {
    next_at: Option<Instant>,
    last_wallpaper: Option<PathBuf>,
}

pub fn run(namespace: String) -> Result<(), Box<dyn std::error::Error>> {
    if is_tui_active() {
        return Ok(());
    }

    let _lock = match WorkerPidLock::acquire() {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
        Err(error) => return Err(Box::new(error)),
    };

    let mut backend = Backend::new(namespace);
    let mut runtime_by_monitor = HashMap::<String, RuntimeState>::new();

    loop {
        if is_tui_active() {
            return Ok(());
        }

        let entries = load_playlist_state()?;
        if entries.iter().all(|entry| !entry.running) {
            return Ok(());
        }

        if !matches!(backend.status(), crate::app::DaemonStatus::Running) {
            let _ = backend.start_daemon();
        }

        runtime_by_monitor
            .retain(|name, _| entries.iter().any(|entry| entry.monitor_name == *name));

        for entry in entries.into_iter().filter(|entry| entry.running) {
            let now = Instant::now();
            let runtime = runtime_by_monitor
                .entry(entry.monitor_name.clone())
                .or_default();

            let due = runtime.next_at.map(|next| next <= now).unwrap_or(true);
            if !due {
                continue;
            }

            let candidates = match entry.source {
                PlaylistSource::Favorites => load_favorites_candidates(),
                PlaylistSource::Workspace => collect_workspace_candidates(),
            };

            if candidates.is_empty() {
                runtime.next_at = Some(now + Duration::from_secs(entry.interval_secs));
                continue;
            }

            let mut selected_index = fastrand::usize(..candidates.len());
            if candidates.len() > 1
                && runtime
                    .last_wallpaper
                    .as_ref()
                    .is_some_and(|last| last == &candidates[selected_index])
            {
                selected_index = (selected_index + 1) % candidates.len();
            }

            let selected = candidates[selected_index].clone();
            match backend.apply_wallpaper(&selected, &entry.monitor_name, ScalingMode::Fill) {
                Ok(()) => {
                    runtime.last_wallpaper = Some(selected);
                    runtime.next_at = Some(now + Duration::from_secs(entry.interval_secs));
                }
                Err(_) => {
                    runtime.next_at = Some(now + Duration::from_secs(5));
                }
            }
        }

        std::thread::sleep(POLL_SLEEP);
    }
}

pub fn spawn_background_worker(namespace: &str) -> io::Result<()> {
    if is_tui_active() {
        return Ok(());
    }

    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--playlist-worker")
        .arg(namespace)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let _ = command.spawn()?;
    Ok(())
}

pub fn mark_tui_active() -> io::Result<()> {
    let path = tui_active_file_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{}", std::process::id()))
}

pub fn clear_tui_active_marker() {
    let path = tui_active_file_path();
    let _ = fs::remove_file(path);
}

fn is_tui_active() -> bool {
    let path = tui_active_file_path();
    let pid = match fs::read_to_string(&path)
        .ok()
        .and_then(|raw| raw.trim().parse::<u32>().ok())
    {
        Some(pid) => pid,
        None => {
            let _ = fs::remove_file(path);
            return false;
        }
    };

    if process_is_alive(pid) {
        true
    } else {
        let _ = fs::remove_file(path);
        false
    }
}

fn process_is_alive(pid: u32) -> bool {
    PathBuf::from("/proc").join(pid.to_string()).exists()
}

fn load_playlist_state() -> io::Result<Vec<PlaylistEntry>> {
    let path = playlist_state_file_path();
    let data = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut entries = Vec::new();
    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut fields = line.splitn(4, '\t');
        let Some(monitor_name) = fields.next() else {
            continue;
        };
        let Some(source_field) = fields.next() else {
            continue;
        };
        let Some(interval_field) = fields.next() else {
            continue;
        };
        let Some(running_field) = fields.next() else {
            continue;
        };

        let source = match source_field {
            "workspace" => PlaylistSource::Workspace,
            "favorites" => PlaylistSource::Favorites,
            _ => continue,
        };

        let interval_secs = match interval_field.parse::<u64>() {
            Ok(value) => value.clamp(PLAYLIST_INTERVAL_MIN_SECS, PLAYLIST_INTERVAL_MAX_SECS),
            Err(_) => continue,
        };

        let running = match running_field {
            "1" => true,
            "0" => false,
            _ => continue,
        };

        entries.push(PlaylistEntry {
            monitor_name: monitor_name.to_string(),
            source,
            interval_secs,
            running,
        });
    }

    Ok(entries)
}

fn load_favorites_candidates() -> Vec<PathBuf> {
    let path = favorites_state_file_path();
    let data = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };

    data.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(PathBuf::from)
        .filter(|path| path.is_file() && is_supported_media(path.as_path()))
        .collect()
}

fn collect_workspace_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let root = pictures_dir();

    let mut stack = vec![root];
    while let Some(path) = stack.pop() {
        let Ok(read_dir) = fs::read_dir(&path) else {
            continue;
        };

        for entry in read_dir.flatten() {
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else {
                continue;
            };

            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file() && is_supported_media(&path) {
                out.push(path);
            }
        }
    }

    out
}

fn is_supported_media(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png"
                    | "jpg"
                    | "jpeg"
                    | "gif"
                    | "bmp"
                    | "webp"
                    | "tif"
                    | "tiff"
                    | "svg"
                    | "avif"
                    | "heic"
                    | "heif"
            )
        })
        .unwrap_or(false)
}

fn pictures_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Pictures")
}

fn playlist_state_file_path() -> PathBuf {
    state_file_path(PLAYLIST_STATE_FILENAME)
}

fn favorites_state_file_path() -> PathBuf {
    state_file_path(FAVORITES_STATE_FILENAME)
}

fn tui_active_file_path() -> PathBuf {
    state_file_path(TUI_ACTIVE_FILENAME)
}

fn worker_pid_file_path() -> PathBuf {
    state_file_path(WORKER_PID_FILENAME)
}

fn state_file_path(filename: &str) -> PathBuf {
    if let Some(path) = env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(path).join("vellum").join(filename);
    }

    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("vellum")
            .join(filename);
    }

    PathBuf::from(filename)
}

struct WorkerPidLock {
    path: PathBuf,
}

impl WorkerPidLock {
    fn acquire() -> io::Result<Self> {
        let path = worker_pid_file_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        if let Some(existing_pid) = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| raw.trim().parse::<u32>().ok())
            && process_is_alive(existing_pid)
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "playlist worker already running",
            ));
        }

        fs::write(&path, format!("{}", std::process::id()))?;
        Ok(Self { path })
    }
}

impl Drop for WorkerPidLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
