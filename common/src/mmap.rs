use core::ptr::NonNull;

use rustix::fd::AsFd;
use rustix::fd::BorrowedFd;
use rustix::fd::OwnedFd;
use rustix::fs;
use rustix::io;
use rustix::io::Errno;
use rustix::mm::MapFlags;
use rustix::mm::ProtFlags;
use rustix::mm::mmap;
use rustix::mm::munmap;
use rustix::shm;
use rustix::shm::Mode;
use rustix::shm::OFlags;

use crate::log;

#[derive(Debug)]
pub struct Mmap {
    fd: OwnedFd,
    ptr: NonNull<core::ffi::c_void>,
    len: usize,
    mmapped: bool,
}

impl Mmap {
    const PROT: ProtFlags = ProtFlags::WRITE.union(ProtFlags::READ);
    const FLAGS: MapFlags = MapFlags::SHARED;

    #[inline]
    pub fn create(len: usize) -> io::Result<Self> {
        let fd = Self::mmap_fd()?;
        rustix::io::retry_on_intr(|| rustix::fs::ftruncate(&fd, len as u64))?;

        let ptr = unsafe {
            let ptr = mmap(core::ptr::null_mut(), len, Self::PROT, Self::FLAGS, &fd, 0)?;
            // SAFETY: the function above will never return a null pointer if it succeeds
            // POSIX says that the implementation will never select an address at 0
            NonNull::new_unchecked(ptr)
        };
        Ok(Self {
            fd,
            ptr,
            len,
            mmapped: true,
        })
    }

    #[cfg(target_os = "linux")]
    fn mmap_fd() -> io::Result<OwnedFd> {
        match Self::memfd() {
            Ok(fd) => Ok(fd),
            // Not supported, use fallback.
            Err(Errno::NOSYS) => Self::shm(),
            Err(err) => Err(err),
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn mmap_fd() -> io::Result<OwnedFd> {
        Self::shm()
    }

    fn shm() -> io::Result<OwnedFd> {
        use rustix::path::DecInt;
        use rustix::time::{ClockId, clock_gettime};

        const PREFIX: &[u8] = b"/vellum-ipc-";
        const FLAGS: OFlags = OFlags::CREATE.union(OFlags::EXCL).union(OFlags::RDWR);
        const MODE: Mode = Mode::RUSR.union(Mode::WUSR);

        let mut write_buf = ::alloc::vec::Vec::from(PREFIX);

        loop {
            let filename = {
                let time = clock_gettime(ClockId::Monotonic);
                write_buf.truncate(PREFIX.len());
                write_buf.extend_from_slice(DecInt::new(time.tv_nsec).as_bytes_with_nul());
                // SAFETY: write buf is null terminated and cannot hold any other internal null
                // bytes, since it is composed of the PREFIX (which has no null bytes) and the
                // integer formatting (which also has no null bytes, except the final one)
                unsafe { core::ffi::CStr::from_bytes_with_nul_unchecked(write_buf.as_slice()) }
            };
            match shm::open(filename, FLAGS, MODE) {
                Ok(fd) => return shm::unlink(filename).map(|()| fd),
                Err(Errno::EXIST | Errno::INTR) => continue,
                Err(err) => return Err(err),
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn memfd() -> io::Result<OwnedFd> {
        use rustix::fs::MemfdFlags;
        use rustix::fs::SealFlags;

        let name = c"vellum-ipc";
        let flags = MemfdFlags::ALLOW_SEALING | MemfdFlags::CLOEXEC;

        loop {
            match fs::memfd_create(name, flags) {
                Ok(fd) => {
                    // We only need to seal for the purposes of optimization, ignore the errors.
                    let _ = fs::fcntl_add_seals(&fd, SealFlags::SHRINK | SealFlags::SEAL);
                    return Ok(fd);
                }
                Err(Errno::INTR) => continue,
                Err(err) => return Err(err),
            }
        }
    }

    #[inline]
    /// Unmaps without destroying the file descriptor
    ///
    /// This is only ever used in the daemon, when animations finish, in order to free up memory
    pub fn unmap(&mut self) {
        if let Err(e) = unsafe { munmap(self.ptr.as_ptr(), self.len) } {
            log::error!("failed to unmap memory: {e}");
        } else {
            self.mmapped = false;
        }
    }

    #[inline]
    /// Ensures that the underlying file descriptor is mapped
    ///
    /// Because `unmap`, above, is only used in the daemon, this is also only used there
    pub fn ensure_mapped(&mut self) {
        if !self.mmapped {
            let mapped = unsafe {
                mmap(
                    core::ptr::null_mut(),
                    self.len,
                    Self::PROT,
                    Self::FLAGS,
                    &self.fd,
                    0,
                )
            };
            match mapped {
                Ok(ptr) => {
                    self.mmapped = true;
                    // SAFETY: mmap does not return a null pointer on success.
                    self.ptr = unsafe { NonNull::new_unchecked(ptr) };
                }
                Err(e) => {
                    log::error!("failed to map memory: {e}");
                }
            }
        }
    }

    #[inline]
    pub fn remap(&mut self, new: usize) {
        if let Err(e) = io::retry_on_intr(|| fs::ftruncate(&self.fd, new as u64)) {
            log::error!("failed to resize mmap backing fd: {e}");
            return;
        }

        #[cfg(target_os = "linux")]
        {
            use rustix::mm;

            let result =
                unsafe { mm::mremap(self.ptr.as_ptr(), self.len, new, mm::MremapFlags::MAYMOVE) };

            if let Ok(ptr) = result {
                // SAFETY: the mremap above will never return a null pointer if it succeeds
                let ptr = unsafe { NonNull::new_unchecked(ptr) };
                self.ptr = ptr;
                self.len = new;
                return;
            }
        }

        let mapped = unsafe {
            mmap(
                core::ptr::null_mut(),
                new,
                Self::PROT,
                Self::FLAGS,
                &self.fd,
                0,
            )
        };
        let ptr = match mapped {
            Ok(ptr) => ptr,
            Err(e) => {
                log::error!("failed to remap memory: {e}");
                return;
            }
        };

        self.unmap();
        self.len = new;
        // SAFETY: mmap does not return a null pointer on success.
        self.ptr = unsafe { NonNull::new_unchecked(ptr) };
        self.mmapped = true;
    }

    #[must_use]
    pub(crate) fn from_fd(fd: OwnedFd, len: usize) -> Self {
        let ptr = match unsafe {
            mmap(
                core::ptr::null_mut(),
                len,
                ProtFlags::READ,
                Self::FLAGS,
                &fd,
                0,
            )
        } {
            Ok(ptr) => {
                // SAFETY: mmap does not return a null pointer on success.
                unsafe { NonNull::new_unchecked(ptr) }
            }
            Err(e) => panic!("failed to map fd-backed shared memory: {e}"),
        };
        Self {
            fd,
            ptr,
            len,
            mmapped: true,
        }
    }

    #[inline]
    #[must_use]
    pub fn slice_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.ptr.as_ptr().cast(), self.len) }
    }

    #[inline]
    #[must_use]
    pub fn slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.ptr.as_ptr().cast(), self.len) }
    }

    #[inline]
    #[must_use]
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    #[must_use]
    pub fn fd(&'_ self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl Drop for Mmap {
    #[inline]
    fn drop(&mut self) {
        if self.mmapped {
            self.unmap();
        }
    }
}

pub struct Mmapped<const UTF8: bool> {
    base_ptr: NonNull<core::ffi::c_void>,
    ptr: NonNull<core::ffi::c_void>,
    len: usize,
}

pub type MmappedBytes = Mmapped<false>;
pub type MmappedStr = Mmapped<true>;

impl<const UTF8: bool> Mmapped<UTF8> {
    const PROT: ProtFlags = ProtFlags::READ;
    const FLAGS: MapFlags = MapFlags::SHARED;

    #[must_use]
    pub(crate) fn new(map: &Mmap, bytes: &[u8]) -> Self {
        let Some(raw_len) = bytes.get(0..4) else {
            return Self::new_with_len(map, &[], 0);
        };
        let len = u32::from_ne_bytes([raw_len[0], raw_len[1], raw_len[2], raw_len[3]]) as usize;
        let bytes = &bytes[4..];
        Self::new_with_len(map, bytes, len)
    }

    #[must_use]
    pub(crate) fn new_with_len(map: &Mmap, bytes: &[u8], len: usize) -> Self {
        let offset = bytes.as_ptr() as usize - map.ptr.as_ptr() as usize;
        let page_size = rustix::param::page_size();
        let page_offset = offset - offset % page_size;

        let total_len = len + (offset - page_offset);
        let base_ptr = match unsafe {
            mmap(
                core::ptr::null_mut(),
                total_len,
                Self::PROT,
                Self::FLAGS,
                &map.fd,
                page_offset as u64,
            )
        } {
            Ok(ptr) => {
                // SAFETY: mmap does not return a null pointer on success.
                unsafe { NonNull::new_unchecked(ptr) }
            }
            Err(e) => panic!("failed to map mmapped view: {e}"),
        };
        let ptr =
            unsafe { NonNull::new_unchecked(base_ptr.as_ptr().byte_add(offset - page_offset)) };

        let mut len = len;
        if UTF8 {
            // Keep UTF8 invariant for `MmappedStr::str()` by downgrading invalid payloads to
            // empty strings instead of panicking.
            let s = unsafe { core::slice::from_raw_parts(ptr.as_ptr().cast(), len) };
            if core::str::from_utf8(s).is_err() {
                log::error!("received a non utf8 string from socket");
                len = 0;
            }
        }

        Self { base_ptr, ptr, len }
    }

    #[inline]
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.ptr.as_ptr().cast(), self.len) }
    }
}

impl Mmapped<true> {
    #[inline]
    #[must_use]
    pub const fn str(&self) -> &str {
        unsafe {
            let slice = core::slice::from_raw_parts(self.ptr.as_ptr().cast(), self.len);
            core::str::from_utf8_unchecked(slice)
        }
    }
}

impl<const UTF8: bool> Drop for Mmapped<UTF8> {
    fn drop(&mut self) {
        let len = self.len + self.ptr.as_ptr() as usize - self.base_ptr.as_ptr() as usize;
        if let Err(e) = unsafe { munmap(self.base_ptr.as_ptr(), len) } {
            log::error!("failed to unmap memory: {e}");
        }
    }
}
