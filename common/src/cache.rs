//! Implements basic cache functionality.
//!
//! The idea is:
//!   1. the client registers the last image sent for each output in a file
//!   2. the daemon spawns a client that reloads that image when an output is created

use ::alloc::format;
use ::alloc::string::String;
use ::alloc::vec::Vec;

use rustix::path::Arg;
use rustix::{buffer, fd, fs, io};

use crate::ipc::Animation;
use crate::ipc::PixelFormat;
use crate::log;
use crate::mmap::Mmap;
use crate::path::Path;
use crate::path::PathBuf;

const CACHE_DIRNAME: &str = env!("CARGO_PKG_VERSION");

/// Create this with [read_cache_file] and use it with [get_previous_image_cache]. It is its own
/// type just for sake of preventing errors with sending generic buffers to
/// [get_previous_image_cache].
pub struct CacheData(Vec<u8>);

pub struct CacheEntry<'a> {
    pub namespace: &'a str,
    pub resize: &'a str,
    pub filter: &'a str,
    pub img_path: &'a str,
}

impl<'a> CacheEntry<'a> {
    pub(crate) fn new(
        namespace: &'a str,
        resize: &'a str,
        filter: &'a str,
        img_path: &'a str,
    ) -> Self {
        Self {
            namespace,
            resize,
            filter,
            img_path,
        }
    }

    fn parse_file<'b>(output_name: &str, data: &'b [u8]) -> Result<Vec<CacheEntry<'b>>, String> {
        let mut v = Vec::new();
        let mut strings = data.split(|ch| *ch == 0);
        while let Some(namespace) = strings.next() {
            let resize = strings.next().ok_or_else(|| {
                format!("cache file for output {output_name} is in the wrong format (no resize)")
            })?;
            let filter = strings.next().ok_or_else(|| {
                format!("cache file for output {output_name} is in the wrong format (no filter)")
            })?;
            let img_path = strings.next().ok_or_else(|| {
                format!(
                    "cache file for output {output_name} is in the wrong format (no image path)"
                )
            })?;

            let err = format!("cache file for output {output_name} is not valid utf8");
            let namespace = str::from_utf8(namespace).map_err(|_| err.clone())?;
            let resize = str::from_utf8(resize).map_err(|_| err.clone())?;
            let filter = str::from_utf8(filter).map_err(|_| err.clone())?;
            let img_path = str::from_utf8(img_path).map_err(|_| err)?;

            v.push(CacheEntry {
                namespace,
                resize,
                filter,
                img_path,
            });
        }

        Ok(v)
    }

    pub(crate) fn store(self, output_name: &str) -> io::Result<()> {
        let mut filepath = cache_dir()?;
        filepath.push_str(output_name);

        let file = fs::open(
            filepath,
            fs::OFlags::RDWR.union(fs::OFlags::CREATE),
            fs::Mode::RUSR.union(fs::Mode::WUSR),
        )?;

        let data = read_all(&file)?;
        let mut entries = Self::parse_file(output_name, &data).unwrap_or_else(|_| Vec::new());

        if let Some(entry) = entries
            .iter_mut()
            .find(|elem| elem.namespace == self.namespace)
        {
            entry.resize = self.resize;
            entry.filter = self.filter;
            entry.img_path = self.img_path;
        } else {
            entries.push(self);
        }

        fs::seek(&file, fs::SeekFrom::Start(0))?;
        let mut len = 0;
        for entry in entries {
            let CacheEntry {
                namespace,
                resize,
                filter,
                img_path,
            } = entry;
            len += write_all(
                &file,
                format!("{namespace}\0{resize}\0{filter}\0{img_path}").as_bytes(),
            )?;
        }

        fs::ftruncate(file, len as u64)?;
        Ok(())
    }
}

pub(crate) fn store_animation_frames(
    animation: &[u8],
    path: &Path,
    dimensions: (u32, u32),
    resize: &str,
    pixel_format: PixelFormat,
) -> io::Result<()> {
    let filename = animation_filename(&path, dimensions, resize, pixel_format);
    let mut filepath = cache_dir()?;
    filepath.push_str(&filename);

    if fs::access(&filepath, fs::Access::EXISTS).is_ok() {
        Ok(())
    } else {
        let file = fs::open(
            filepath,
            fs::OFlags::WRONLY.union(fs::OFlags::CREATE),
            fs::Mode::WUSR.union(fs::Mode::RUSR),
        )?;
        write_all(&file, animation)?;
        Ok(())
    }
}

pub fn load_animation_frames<P: Arg>(
    path: &P,
    dimensions: (u32, u32),
    resize: &str,
    pixel_format: PixelFormat,
) -> io::Result<Option<Animation>> {
    let filename = animation_filename(path, dimensions, resize, pixel_format);
    let cache_dir = cache_dir()?;

    let mut dir = fs::Dir::new(fs::open(&cache_dir, fs::OFlags::RDONLY, fs::Mode::RUSR)?)?;
    while let Some(entry) = dir.next() {
        if let Ok(entry) = entry
            && entry.file_name().to_bytes() == filename.as_bytes()
        {
            let fd = fs::openat(
                dir.fd()?,
                entry.file_name(),
                fs::OFlags::RDONLY,
                fs::Mode::RUSR,
            )?;
            let len = rustix::fs::seek(&fd, rustix::fs::SeekFrom::End(0))?;
            let mmap = Mmap::from_fd(fd, len as usize);

            match Animation::deserialize(&mmap, mmap.slice()) {
                Some((frames, _)) => return Ok(Some(frames)),
                None => log::error!("failed to load cached animation frames"),
            }
        }
    }
    Ok(None)
}

pub fn read_cache_file(output_name: &str) -> io::Result<CacheData> {
    clean_previous_versions();

    let mut filepath = cache_dir()?;
    filepath.push_str(output_name);

    let file = fs::open(filepath, fs::OFlags::RDONLY, fs::Mode::RUSR)?;
    Ok(CacheData(read_all(&file)?))
}

pub fn get_previous_image_cache<'a>(
    output_name: &str,
    namespace: &str,
    cache_data: &'a CacheData,
) -> Result<Option<CacheEntry<'a>>, String> {
    let entries = CacheEntry::parse_file(output_name, &cache_data.0)?;

    Ok(entries
        .into_iter()
        .find(|entry| entry.namespace == namespace))
}

pub fn clean() -> io::Result<()> {
    let path = user_cache_dir()?;
    // SAFETY: because path is absolute, the file descriptor will be ignored, and can be whatever
    remove_dir_all(unsafe { fd::BorrowedFd::borrow_raw(0) }, path.as_c_str())?;
    fs::rmdir(path)
}

fn clean_previous_versions() {
    let user_cache = match user_cache_dir() {
        Ok(path) => path,
        Err(e) => {
            log::warn!("failed to get user cache dir {e}");
            return;
        }
    };

    let dir_fd = match fs::open(
        &user_cache,
        fs::OFlags::RDONLY,
        fs::Mode::RUSR.union(fs::Mode::WUSR),
    ) {
        Ok(fd) => fd,
        Err(e) => {
            log::warn!("failed to open cache dir at {}: {e}", user_cache.display());
            return;
        }
    };

    let mut dir = match fs::Dir::new(dir_fd) {
        Ok(dir) => dir,
        Err(e) => {
            log::warn!("failed to read cache dir at {}: {e}", user_cache.display());
            return;
        }
    };

    while let Some(entry) = dir.next() {
        if let Ok(entry) = entry {
            let dir_fd = match dir.fd() {
                Ok(fd) => fd,
                Err(e) => {
                    log::warn!(
                        "while reading dir '{}' entries, failed to get dir_fd: {e}",
                        user_cache.display()
                    );
                    return;
                }
            };
            let name = entry.file_name();
            const CACHE_DIRNAME_BYTES: &[u8] = CACHE_DIRNAME.as_bytes();
            match name.to_bytes() {
                b"." | b".." | CACHE_DIRNAME_BYTES => continue,
                _ => {
                    let stat = match fs::statat(dir_fd, name, fs::AtFlags::empty()) {
                        Ok(stat) => stat,
                        Err(e) => {
                            log::warn!(
                                "failed to stat cache entry {}: {e}",
                                name.to_string_lossy()
                            );
                            continue;
                        }
                    };

                    let mut unlink_flags = fs::AtFlags::empty();
                    if let fs::FileType::Directory = fs::FileType::from_raw_mode(stat.st_mode) {
                        if let Err(e) = remove_dir_all(dir_fd, name) {
                            log::warn!(
                                "failed to remove cache directory {}: {e}",
                                name.to_string_lossy()
                            );
                        }
                        unlink_flags = fs::AtFlags::REMOVEDIR;
                    }

                    if let Err(e) = fs::unlinkat(dir_fd, name, unlink_flags) {
                        log::warn!(
                            "failed to remove cache entry {}: {e}",
                            name.to_string_lossy()
                        );
                        continue;
                    }
                }
            }
        }
    }
}

fn create_dir(p: &Path) -> io::Result<()> {
    match fs::access(p, fs::Access::EXISTS) {
        Ok(()) => Ok(()),
        Err(_) => fs::mkdir(p, fs::Mode::RWXU),
    }
}

fn user_cache_dir() -> io::Result<PathBuf> {
    if let Some(path) = unsafe { crate::getenv(c"XDG_CACHE_HOME") } {
        Ok(PathBuf::from_iter([path, c"vellum"]))
    } else if let Some(path) = unsafe { crate::getenv(c"HOME") } {
        Ok(PathBuf::from_iter([path, c".cache", c"vellum"]))
    } else {
        Err(io::Errno::NOENT)
    }
}

fn cache_dir() -> io::Result<PathBuf> {
    let mut path = user_cache_dir()?;
    create_dir(&path)?;
    path.push_str(CACHE_DIRNAME);
    create_dir(&path)?;
    Ok(path)
}

#[must_use]
fn animation_filename<P: Arg>(
    path: &P,
    dimensions: (u32, u32),
    resize: &str,
    pixel_format: PixelFormat,
) -> String {
    format!(
        "{}__{}x{}_{}_{}",
        path.to_string_lossy().replace('/', "_"),
        dimensions.0,
        dimensions.1,
        resize,
        pixel_format,
    )
}

fn write_all(file: &rustix::fd::OwnedFd, buf: &[u8]) -> io::Result<usize> {
    let mut i = 0;
    while i < buf.len() {
        i += io::write(file, &buf[i..])?;
    }
    Ok(i)
}

fn read_all(file: &rustix::fd::OwnedFd) -> io::Result<Vec<u8>> {
    let mut data = Vec::with_capacity(128);

    loop {
        match io::read(file, buffer::spare_capacity(&mut data))? {
            0 => break,
            _ => {
                if data.len() == data.capacity() {
                    data.reserve(data.len());
                }
            }
        }
    }
    Ok(data)
}

fn remove_dir_all(dir: fd::BorrowedFd, file: &core::ffi::CStr) -> io::Result<()> {
    let mut dir = fs::Dir::new(fs::openat(
        dir,
        file,
        fs::OFlags::RDONLY,
        fs::Mode::RUSR.union(fs::Mode::WUSR),
    )?)?;

    while let Some(entry) = dir.next() {
        if let Ok(entry) = entry {
            let dir_fd = dir.fd()?;
            let name = entry.file_name();

            match name.to_bytes() {
                b"." | b".." => continue,
                _ => {
                    let stat = fs::statat(dir_fd, name, fs::AtFlags::empty())?;
                    let mut unlink_flags = fs::AtFlags::empty();
                    if let fs::FileType::Directory = fs::FileType::from_raw_mode(stat.st_mode) {
                        remove_dir_all(dir_fd, name)?;
                        unlink_flags = fs::AtFlags::REMOVEDIR;
                    }

                    fs::unlinkat(dir_fd, name, unlink_flags)?;
                }
            }
        }
    }

    Ok(())
}
