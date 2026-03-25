#![allow(unsafe_op_in_unsafe_fn)]
#![no_std]

extern crate alloc;

pub mod cache;
pub mod compression;
pub mod ipc;
pub mod log;
pub mod mmap;
pub mod path;

/// Manual getenv implementation from an extern environ variable.
///
/// Note: this is marked as `#[inline(never)]` and `#[cold]` because this function will be executed
/// a few times, mostly during initialization, and thus inlining it would serve little purpose other
/// than increasing binary size.
///
/// Note2: we do not use `libc::getenv` because the long-term plan is not depending on `libc` in
/// the `daemon` (currently we can only do that in Rust nightly).
///
/// # Safety
///
/// The `env` parameter must **NOT** end with an `=` byte (before the final null byte, of course).
#[cold]
#[inline(never)]
pub unsafe fn getenv(env: &core::ffi::CStr) -> Option<&core::ffi::CStr> {
    unsafe extern "Rust" {
        static environ: *const *const core::ffi::c_char;
    }

    let mut ptr = environ;
    loop {
        let cptr = unsafe { ptr.read() };
        if cptr.is_null() {
            return None;
        }
        // SAFETY: environ is composed of null terminated strings, so this should be safe
        let cstr = unsafe { core::ffi::CStr::from_ptr(cptr) };
        if let Some(value) = cstr.to_bytes_with_nul().strip_prefix(env.to_bytes()) {
            // SAFETY:
            // Because `env` does not end with a `=` byte, value[1..] will always skip the `=`
            // byte, and the rest of the string is guaranteed to end in a null byte, since it was
            // created by removing the prefix of another CStr, which would also ends in a null byte
            return Some(unsafe { core::ffi::CStr::from_bytes_with_nul_unchecked(&value[1..]) });
        }
        ptr = unsafe { ptr.add(1) };
    }
}
