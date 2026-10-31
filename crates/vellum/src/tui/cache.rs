//! Preview caches: a small in-memory [`Lru`] of encoded previews, and the
//! on-disk [`DiskCache`] of rendered ones under `~/.cache/vellum/thumbs/`.
//!
//! Both are best-effort. A disk error never fails a preview; the cache just
//! misses.

use std::collections::VecDeque;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

#[cfg(test)]
mod tests;

/// The `capacity` most recently used values.
#[derive(Debug)]
pub struct Lru<K, V> {
    capacity: usize,
    /// Most recently used first.
    entries: VecDeque<(K, V)>,
}

impl<K: PartialEq, V> Lru<K, V> {
    pub fn new(capacity: usize) -> Lru<K, V> {
        Lru {
            capacity,
            entries: VecDeque::with_capacity(capacity),
        }
    }

    /// The value for `key`, now the most recently used.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        let index = self.entries.iter().position(|(k, _)| k == key)?;
        let entry = self.entries.remove(index)?;
        self.entries.push_front(entry);
        self.entries.front().map(|(_, v)| v)
    }

    /// Stores `value`, dropping the least recently used entry if full.
    pub fn put(&mut self, key: K, value: V) {
        if self.capacity == 0 {
            return;
        }
        self.entries.retain(|(k, _)| *k != key);
        self.entries.truncate(self.capacity - 1);
        self.entries.push_front((key, value));
    }
}

/// `$XDG_CACHE_HOME/vellum/thumbs`, falling back to `~/.cache/vellum/thumbs`.
pub fn thumbs_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|dir| Path::new(dir).is_absolute())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".cache")))?;
    Some(base.join("vellum/thumbs"))
}

/// Byte blobs on disk, keyed by a string, holding at most `capacity` bytes.
/// When a write goes over, the least recently used entries are deleted
/// first. A read counts as a use (it bumps the file's mtime).
///
/// Each entry is one file named after a hash of its key. The file starts
/// with the full key, so a hash collision reads as a miss, not as someone
/// else's preview.
#[derive(Debug)]
pub struct DiskCache {
    dir: PathBuf,
    capacity: u64,
    /// Bytes on disk, counted on first write.
    used: Mutex<Option<u64>>,
}

impl DiskCache {
    /// A cache in `dir` (created on first write) capped at `capacity`
    /// bytes. A capacity of 0 turns it off.
    pub fn new(dir: PathBuf, capacity: u64) -> DiskCache {
        DiskCache {
            dir,
            capacity,
            used: Mutex::new(None),
        }
    }

    /// The blob stored under `key`, if any.
    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        if self.capacity == 0 {
            return None;
        }
        let path = self.path(key);
        let mut bytes = fs::read(&path).ok()?;
        let start = strip_key(&bytes, key)?;
        // Mark it used. If this fails the entry is just evicted sooner.
        if let Ok(file) = fs::File::options().write(true).open(&path) {
            let _ = file.set_modified(SystemTime::now());
        }
        bytes.drain(..start);
        Some(bytes)
    }

    /// Stores `data` under `key`, then evicts the least recently used
    /// entries until the cache fits its capacity again.
    pub fn put(&self, key: &str, data: &[u8]) {
        let size = (4 + key.len() + data.len()) as u64;
        if self.capacity == 0 || size > self.capacity {
            return;
        }
        let Ok(mut used) = self.used.lock() else {
            return;
        };
        if self.write(key, data).is_err() {
            return;
        }
        let total = match *used {
            Some(n) => n + size,
            None => self.entries().iter().map(|e| e.size).sum(),
        };
        *used = Some(if total > self.capacity {
            self.evict()
        } else {
            total
        });
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{:016x}", fnv1a(key.as_bytes())))
    }

    /// Writes the entry to a temporary file and renames it into place, so
    /// a reader never sees half an entry.
    fn write(&self, key: &str, data: &[u8]) -> io::Result<()> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join(format!(
            ".tmp-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let written = (|| {
            let mut file = fs::File::create(&tmp)?;
            let len = u32::try_from(key.len()).map_err(io::Error::other)?;
            file.write_all(&len.to_le_bytes())?;
            file.write_all(key.as_bytes())?;
            file.write_all(data)?;
            fs::rename(&tmp, self.path(key))
        })();
        if written.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        written
    }

    /// Deletes the least recently used entries until the rest fit. Returns
    /// the bytes left.
    fn evict(&self) -> u64 {
        let mut entries = self.entries();
        entries.sort_by_key(|e| e.used);
        let mut total: u64 = entries.iter().map(|e| e.size).sum();
        for entry in entries {
            if total <= self.capacity {
                break;
            }
            if fs::remove_file(&entry.path).is_ok() {
                total -= entry.size;
            }
        }
        total
    }

    /// Every finished entry on disk.
    fn entries(&self) -> Vec<Entry> {
        let Ok(dir) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        dir.filter_map(Result::ok)
            .filter(|e| !e.file_name().as_encoded_bytes().starts_with(b"."))
            .filter_map(|e| {
                let meta = e.metadata().ok().filter(fs::Metadata::is_file)?;
                Some(Entry {
                    path: e.path(),
                    size: meta.len(),
                    used: meta.modified().ok()?,
                })
            })
            .collect()
    }
}

struct Entry {
    path: PathBuf,
    size: u64,
    used: SystemTime,
}

/// Where the data starts in an entry file, if the file holds `key`.
fn strip_key(bytes: &[u8], key: &str) -> Option<usize> {
    let (len, rest) = bytes.split_first_chunk::<4>()?;
    let len = usize::try_from(u32::from_le_bytes(*len)).ok()?;
    (rest.get(..len)? == key.as_bytes()).then_some(4 + len)
}

/// 64-bit FNV-1a: stable across Rust versions, unlike `DefaultHasher`, so
/// a toolchain update does not orphan the cache.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}
