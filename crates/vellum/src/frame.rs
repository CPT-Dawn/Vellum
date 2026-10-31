//! Frame memory: a memfd mapped while `vellum` renders into it, then sealed
//! and handed to the Daemon.

use std::os::fd::OwnedFd;
use std::ptr::NonNull;

use rustix::fs::{MemfdFlags, SealFlags};
use rustix::mm::{MapFlags, ProtFlags};

/// A writable frame in shared memory.
pub struct Frame {
    fd: OwnedFd,
    ptr: NonNull<u8>,
    len: usize,
    pub stride: u32,
}

impl Frame {
    /// Allocates a zeroed XRGB8888 frame of `width`×`height` pixels.
    pub fn new(width: u32, height: u32) -> std::io::Result<Frame> {
        let stride = width
            .checked_mul(4)
            .ok_or_else(|| std::io::Error::other("frame too wide"))?;
        let len = stride as usize * height as usize;
        if len == 0 {
            return Err(std::io::Error::other("frame has no pixels"));
        }
        let fd = rustix::fs::memfd_create(
            "vellum-frame",
            MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
        )?;
        rustix::fs::ftruncate(&fd, len as u64)?;
        // SAFETY: a fresh shared mapping of a file we just sized to `len`.
        let ptr = unsafe {
            rustix::mm::mmap(
                std::ptr::null_mut(),
                len,
                ProtFlags::READ | ProtFlags::WRITE,
                MapFlags::SHARED,
                &fd,
                0,
            )?
        };
        let ptr = NonNull::new(ptr.cast()).expect("mmap never returns null");
        Ok(Frame {
            fd,
            ptr,
            len,
            stride,
        })
    }

    /// The pixel bytes, row by row: B, G, R, X per pixel.
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        // SAFETY: the mapping is `len` bytes, page-aligned and lives as long
        // as `self`; `&mut self` makes this the only reference to it.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }

    /// Unmaps the frame and seals its size, so the Daemon and compositor can
    /// trust it. Returns the fd to send.
    pub fn finish(self) -> std::io::Result<OwnedFd> {
        let this = std::mem::ManuallyDrop::new(self);
        // SAFETY: unmapping our own mapping exactly once; `this` is never
        // dropped, so Drop will not unmap it again.
        unsafe { rustix::mm::munmap(this.ptr.as_ptr().cast(), this.len)? };
        // SAFETY: moving the fd out of a value that is never dropped.
        let fd = unsafe { std::ptr::read(&this.fd) };
        rustix::fs::fcntl_add_seals(&fd, SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL)?;
        Ok(fd)
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        // SAFETY: our own mapping, unmapped once.
        let _ = unsafe { rustix::mm::munmap(self.ptr.as_ptr().cast(), self.len) };
    }
}
