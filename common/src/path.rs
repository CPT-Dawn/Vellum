use ::alloc::boxed::Box;
use ::alloc::ffi::CString;
use ::alloc::vec::Vec;
use core::ffi::CStr;

use rustix::path::Arg;

const PATHSEP: u8 = b'/';

/// A `PathBuf` backend by a `Vec<u8>` that is always null terminated.
/// We always make sure the inner bytes can be correctly transformed into a CStr. This makes it
/// more efficient when making syscalls, since those usually expect a null-terminated C string.
///
/// Note that not all valid C string may be valid paths. In some systems, certain bytes may be
/// disallowed. To ensure maximum compatibility, use only characters in the POSIX Portable Filename
/// character set:
///
///   * `[A-Z]`
///   * `[a-z]`
///   * `[0-9]`
///   * `.` (period)
///   * `_` (underscore)
///   * `-` (hyphen)
///
/// # Important
///
/// This does **not** have the same semantics as `std::path::PathBuf`. In particular, pushing an
/// absolute path will **not** replace the existing path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct PathBuf(Vec<u8>);

#[derive(Debug)]
#[repr(transparent)]
pub struct Path(CStr);

impl PathBuf {
    #[inline]
    pub fn new() -> Self {
        let mut v = Vec::with_capacity(16);
        v.push(0);
        Self(v)
    }

    /// This will append an cstr to the final component of the current path, without including the
    /// path separator
    pub fn append_cstr(&mut self, path: &CStr) {
        self.0.pop();
        self.0.extend_from_slice(path.to_bytes_with_nul());
    }

    /// This will append an cstr to the final component of the current path, without including the
    /// path separator. Will only append up until the first null byte. This allows the API to be
    /// infallible.
    pub fn append_str(&mut self, path: &str) {
        let bytes = path.as_bytes();
        let null_index = bytes.iter().position(|p| *p == 0).unwrap_or(bytes.len());
        let bytes = &bytes[..null_index];
        if !bytes.is_empty() {
            self.0.pop();
            self.0.extend_from_slice(bytes);
            self.0.push(0);
        }
    }

    pub fn as_path(&self) -> &Path {
        unsafe {
            let cstr = CStr::from_bytes_with_nul_unchecked(&self.0);
            Path::from_cstr(cstr)
        }
    }

    pub fn push_cstr(&mut self, path: &CStr) {
        if !path.is_empty() {
            let len = self.0.len();
            if len > 1 {
                unsafe { *self.0.get_unchecked_mut(len - 1) = PATHSEP };
            } else {
                self.0.pop();
            }
            self.0.extend_from_slice(path.to_bytes_with_nul());
        }
    }

    /// Will only push up until the first null byte. This allows the API to be infallible.
    pub fn push_str(&mut self, path: &str) {
        if !path.is_empty() {
            let len = self.0.len();
            if len > 1 {
                unsafe { *self.0.get_unchecked_mut(len - 1) = PATHSEP };
            } else {
                self.0.pop();
            }
            let null_index = path
                .as_bytes()
                .iter()
                .position(|p| *p == 0)
                .unwrap_or(path.len());
            self.0.extend_from_slice(&path.as_bytes()[..null_index]);
            if path.as_bytes()[null_index - 1] != 0 {
                self.0.push(0);
            }
        }
    }

    pub fn into_c_string(self) -> CString {
        unsafe { CString::from_vec_with_nul_unchecked(self.0) }
    }

    pub fn into_boxed_path(self) -> Box<Path> {
        let rw = Box::into_raw(self.0.into_boxed_slice()) as *mut Path;
        unsafe { Box::from_raw(rw) }
    }
}

impl<'a> core::iter::FromIterator<&'a CStr> for PathBuf {
    fn from_iter<T: IntoIterator<Item = &'a CStr>>(iter: T) -> Self {
        let mut buf = Self::new();
        for component in iter {
            buf.push_cstr(component);
        }
        buf
    }
}

impl core::ops::Deref for PathBuf {
    type Target = Path;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_path()
    }
}

impl core::borrow::Borrow<Path> for PathBuf {
    #[inline]
    fn borrow(&self) -> &Path {
        core::ops::Deref::deref(self)
    }
}

impl From<&Path> for PathBuf {
    fn from(value: &Path) -> Self {
        PathBuf(value.0.to_bytes_with_nul().to_vec())
    }
}

impl From<&CStr> for PathBuf {
    fn from(value: &CStr) -> Self {
        let mut buf = PathBuf::new();
        buf.push_cstr(value);
        buf
    }
}

impl Path {
    #[inline]
    pub fn as_c_str(&self) -> &CStr {
        &self.0
    }

    #[inline]
    pub fn from_cstr(value: &CStr) -> &Path {
        unsafe { &*(value as *const CStr as *const Path) }
    }

    #[inline]
    pub fn display(&self) -> ::alloc::borrow::Cow<'_, str> {
        self.0.to_string_lossy()
    }

    pub fn file_name(&self) -> Option<&CStr> {
        let bytes = self.0.to_bytes_with_nul();
        if bytes.len() > 1 {
            let sep_index = bytes
                .iter()
                .rposition(|p| *p == PATHSEP)
                .map(|i| i + 1)
                .unwrap_or(0);
            bytes
                .get(sep_index..)
                .map(|s| unsafe { CStr::from_bytes_with_nul_unchecked(s) })
        } else {
            None
        }
    }

    pub fn parent(&self) -> Option<PathBuf> {
        let bytes = self.0.to_bytes_with_nul();
        if bytes.len() > 1 {
            let sep_index = bytes.iter().rposition(|p| *p == PATHSEP)?;
            if sep_index == bytes.len() - 1 || sep_index == 0 {
                None
            } else {
                let mut v = Vec::new();
                v.extend_from_slice(unsafe { bytes.get_unchecked(..sep_index) });
                v.push(0);
                Some(PathBuf(v))
            }
        } else {
            None
        }
    }
}

impl Arg for PathBuf {
    fn as_str(&self) -> rustix::io::Result<&str> {
        let cstr = unsafe { CStr::from_bytes_with_nul_unchecked(&self.0) };
        cstr.to_str().map_err(|_utf8_err| rustix::io::Errno::INVAL)
    }

    fn to_string_lossy(&self) -> ::alloc::borrow::Cow<'_, str> {
        let cstr = unsafe { CStr::from_bytes_with_nul_unchecked(&self.0) };
        cstr.to_string_lossy()
    }

    fn as_cow_c_str(&self) -> rustix::io::Result<::alloc::borrow::Cow<'_, CStr>> {
        let cstr = unsafe { CStr::from_bytes_with_nul_unchecked(&self.0) };
        Ok(::alloc::borrow::Cow::Borrowed(cstr))
    }

    fn into_c_str<'b>(self) -> rustix::io::Result<::alloc::borrow::Cow<'b, CStr>>
    where
        Self: 'b,
    {
        Ok(::alloc::borrow::Cow::Owned(self.into_c_string()))
    }

    fn into_with_c_str<T, F>(self, f: F) -> rustix::io::Result<T>
    where
        Self: Sized,
        F: FnOnce(&CStr) -> rustix::io::Result<T>,
    {
        self.as_path().into_with_c_str(f)
    }
}

impl Arg for &PathBuf {
    fn as_str(&self) -> rustix::io::Result<&str> {
        let cstr = unsafe { CStr::from_bytes_with_nul_unchecked(&self.0) };
        cstr.to_str().map_err(|_utf8_err| rustix::io::Errno::INVAL)
    }

    fn to_string_lossy(&self) -> ::alloc::borrow::Cow<'_, str> {
        let cstr = unsafe { CStr::from_bytes_with_nul_unchecked(&self.0) };
        cstr.to_string_lossy()
    }

    fn as_cow_c_str(&self) -> rustix::io::Result<::alloc::borrow::Cow<'_, CStr>> {
        let cstr = unsafe { CStr::from_bytes_with_nul_unchecked(&self.0) };
        Ok(::alloc::borrow::Cow::Borrowed(cstr))
    }

    fn into_c_str<'b>(self) -> rustix::io::Result<::alloc::borrow::Cow<'b, CStr>>
    where
        Self: 'b,
    {
        let cstr = unsafe { CStr::from_bytes_with_nul_unchecked(&self.0) };
        Ok(::alloc::borrow::Cow::Borrowed(cstr))
    }

    fn into_with_c_str<T, F>(self, f: F) -> rustix::io::Result<T>
    where
        Self: Sized,
        F: FnOnce(&CStr) -> rustix::io::Result<T>,
    {
        self.as_path().into_with_c_str(f)
    }
}

impl Arg for &Path {
    fn as_str(&self) -> rustix::io::Result<&str> {
        self.0
            .to_str()
            .map_err(|_utf8_err| rustix::io::Errno::INVAL)
    }

    fn to_string_lossy(&self) -> ::alloc::borrow::Cow<'_, str> {
        self.0.to_string_lossy()
    }

    fn as_cow_c_str(&self) -> rustix::io::Result<::alloc::borrow::Cow<'_, CStr>> {
        Ok(::alloc::borrow::Cow::Borrowed(&self.0))
    }

    fn into_c_str<'b>(self) -> rustix::io::Result<::alloc::borrow::Cow<'b, CStr>>
    where
        Self: 'b,
    {
        Ok(::alloc::borrow::Cow::Borrowed(&self.0))
    }

    fn into_with_c_str<T, F>(self, f: F) -> rustix::io::Result<T>
    where
        Self: Sized,
        F: FnOnce(&CStr) -> rustix::io::Result<T>,
    {
        f(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pushing_paths() {
        let mut buf = PathBuf::new();
        assert_eq!(buf.0, c"".to_bytes_with_nul());
        assert_eq!(buf.file_name(), None);
        assert_eq!(buf.parent(), None);

        buf.push_str("AHOY");
        assert_eq!(buf.0, c"AHOY".to_bytes_with_nul());
        assert_eq!(buf.file_name(), Some(c"AHOY"));
        assert_eq!(buf.parent(), None);

        buf.push_cstr(c"AHOY");
        assert_eq!(buf.0, c"AHOY/AHOY".to_bytes_with_nul());
        assert_eq!(buf.file_name(), Some(c"AHOY"));
        assert_eq!(buf.parent(), Some(PathBuf::from(c"AHOY")));

        buf.push_str("AHOY");
        assert_eq!(buf.0, c"AHOY/AHOY/AHOY".to_bytes_with_nul());
        assert_eq!(buf.file_name(), Some(c"AHOY"));
        assert_eq!(buf.parent(), Some(PathBuf::from(c"AHOY/AHOY")));

        buf.push_cstr(c"AHOY");
        assert_eq!(buf.0, c"AHOY/AHOY/AHOY/AHOY".to_bytes_with_nul());
        assert_eq!(buf.file_name(), Some(c"AHOY"));
        assert_eq!(buf.parent(), Some(PathBuf::from(c"AHOY/AHOY/AHOY")));

        buf.append_cstr(c"...ahoy");
        assert_eq!(buf.0, c"AHOY/AHOY/AHOY/AHOY...ahoy".to_bytes_with_nul());
        assert_eq!(buf.file_name(), Some(c"AHOY...ahoy"));
        assert_eq!(buf.parent(), Some(PathBuf::from(c"AHOY/AHOY/AHOY")));

        buf.append_str("...ahoy");
        assert_eq!(
            buf.0,
            c"AHOY/AHOY/AHOY/AHOY...ahoy...ahoy".to_bytes_with_nul()
        );
        assert_eq!(buf.file_name(), Some(c"AHOY...ahoy...ahoy"));
        assert_eq!(buf.parent(), Some(PathBuf::from(c"AHOY/AHOY/AHOY")));

        buf.push_str("AHOY\0AHOYAHOY\0AHOYAHOYAHOY");
        assert_eq!(
            buf.0,
            c"AHOY/AHOY/AHOY/AHOY...ahoy...ahoy/AHOY".to_bytes_with_nul()
        );
        assert_eq!(buf.file_name(), Some(c"AHOY"));
        assert_eq!(
            buf.parent(),
            Some(PathBuf::from(c"AHOY/AHOY/AHOY/AHOY...ahoy...ahoy"))
        );
    }

    #[test]
    fn paths_from_iter() {
        let buf = PathBuf::from_iter([c"ONE"]);
        assert_eq!(buf.0, c"ONE".to_bytes_with_nul());

        let buf = PathBuf::from_iter([c"ONE", c"TWO"]);
        assert_eq!(buf.0, c"ONE/TWO".to_bytes_with_nul());

        let buf = PathBuf::from_iter([c"ONE", c"TWO", c"THREE"]);
        assert_eq!(buf.0, c"ONE/TWO/THREE".to_bytes_with_nul());
    }

    #[test]
    fn pushing_empty_paths() {
        let mut buf = PathBuf::new();
        assert_eq!(buf.0, c"".to_bytes_with_nul());
        assert_eq!(buf.file_name(), None);
        assert_eq!(buf.parent(), None);

        buf.push_str("AHOY");
        assert_eq!(buf.0, c"AHOY".to_bytes_with_nul());
        assert_eq!(buf.file_name(), Some(c"AHOY"));
        assert_eq!(buf.parent(), None);

        buf.push_cstr(c"");
        assert_eq!(buf.0, c"AHOY".to_bytes_with_nul());
        assert_eq!(buf.file_name(), Some(c"AHOY"));
        assert_eq!(buf.parent(), None);

        buf.push_str("");
        assert_eq!(buf.0, c"AHOY".to_bytes_with_nul());
        assert_eq!(buf.file_name(), Some(c"AHOY"));
        assert_eq!(buf.parent(), None);

        buf.append_cstr(c"");
        assert_eq!(buf.0, c"AHOY".to_bytes_with_nul());
        assert_eq!(buf.file_name(), Some(c"AHOY"));
        assert_eq!(buf.parent(), None);

        buf.append_str("");
        assert_eq!(buf.0, c"AHOY".to_bytes_with_nul());
        assert_eq!(buf.file_name(), Some(c"AHOY"));
        assert_eq!(buf.parent(), None);
    }
}
