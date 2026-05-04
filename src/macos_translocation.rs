//! Resolve macOS App Translocation sandbox paths back to the user's
//! real install location.
//!
//! macOS copies a quarantined .app to a randomized, read-only sandbox
//! at `/private/var/folders/.../AppTranslocation/<UUID>/d/<name>.app`
//! the first time it is launched, and runs the binary from there. From
//! inside the sandbox, `current_exe()` points at the sandbox path and
//! the user's filesystem is invisible — so the sibling-vault probe
//! that drives SATCHEL's USB-stick layout cannot find anything.
//!
//! macOS does, however, expose `SecTranslocateCreateOriginalPathForURL`
//! in `Security.framework` (public API since 10.12). It takes the
//! translocated URL and returns the URL the .app actually lives at on
//! the user's disk. We FFI to it directly here rather than pulling in
//! `core-foundation`, `security-framework`, and friends, since the
//! call site is tiny and self-contained.
//!
//! On non-macOS targets the resolver is a stub returning `None`.

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::c_void;
    use std::path::{Path, PathBuf};

    // Opaque CF pointer types. We never deref them; we only pass them
    // around and call CFRelease at the right moments.
    type CFTypeRef = *const c_void;
    type CFAllocatorRef = *const c_void;
    type CFURLRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFErrorRef = *const c_void;
    // CoreFoundation's `Boolean` is `unsigned char`; CFIndex is `long`,
    // which is `isize` on 64-bit Apple platforms.
    type Boolean = u8;
    type CFIndex = isize;

    #[allow(non_upper_case_globals)]
    const K_CF_URL_POSIX_PATH_STYLE: u32 = 0;
    #[allow(non_upper_case_globals)]
    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(
            allocator: CFAllocatorRef,
            buffer: *const u8,
            buf_len: CFIndex,
            is_directory: Boolean,
        ) -> CFURLRef;
        fn CFURLCopyFileSystemPath(url: CFURLRef, path_style: u32) -> CFStringRef;
        fn CFStringGetLength(string: CFStringRef) -> CFIndex;
        fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: u32) -> CFIndex;
        fn CFStringGetCString(
            string: CFStringRef,
            buffer: *mut u8,
            buffer_size: CFIndex,
            encoding: u32,
        ) -> Boolean;
        fn CFRelease(cf: CFTypeRef);
    }

    #[link(name = "Security", kind = "framework")]
    extern "C" {
        fn SecTranslocateCreateOriginalPathForURL(
            translocated_path: CFURLRef,
            err: *mut CFErrorRef,
        ) -> CFURLRef;
    }

    /// Given a translocated `.app/Contents/MacOS/<bin>` path, return
    /// the real path that .app lives at on the user's disk. Returns
    /// `None` if the OS API errors, returns NULL, or yields a string
    /// we cannot decode as UTF-8.
    ///
    /// Caller is expected to verify `is_translocated(translocated)`
    /// first; passing a non-translocated path is harmless but wastes
    /// a couple of CF allocations.
    pub fn resolve_translocated_path(translocated: &Path) -> Option<PathBuf> {
        let path_str = translocated.to_str()?;
        let path_bytes = path_str.as_bytes();
        unsafe {
            // Wrap the input path in a CFURL.
            let url = CFURLCreateFromFileSystemRepresentation(
                std::ptr::null(),
                path_bytes.as_ptr(),
                path_bytes.len() as CFIndex,
                /* is_directory */ 0,
            );
            if url.is_null() {
                return None;
            }

            // Ask Security.framework for the original path. NULL out
            // the error CFRef in advance; if SecTranslocate populates
            // it on failure, we release it.
            let mut err: CFErrorRef = std::ptr::null();
            let original_url = SecTranslocateCreateOriginalPathForURL(url, &mut err);
            CFRelease(url);

            if original_url.is_null() {
                if !err.is_null() {
                    CFRelease(err);
                }
                return None;
            }

            // CFURL -> CFString (POSIX path) -> Rust String. Allocate
            // enough room for the worst-case UTF-8 expansion of the
            // CFString (CFStringGetMaximumSizeForEncoding), plus one
            // byte for the trailing NUL the CF call writes.
            let cfstring = CFURLCopyFileSystemPath(original_url, K_CF_URL_POSIX_PATH_STYLE);
            CFRelease(original_url);
            if cfstring.is_null() {
                return None;
            }

            let len = CFStringGetLength(cfstring);
            let max_size = CFStringGetMaximumSizeForEncoding(len, K_CF_STRING_ENCODING_UTF8);
            // Defensive lower bound: CFStringGetMaximumSizeForEncoding
            // can return -1 (kCFNotFound) when the input length is 0.
            let buf_size = if max_size <= 0 { 1 } else { max_size + 1 } as usize;
            let mut buf = vec![0u8; buf_size];

            let ok = CFStringGetCString(
                cfstring,
                buf.as_mut_ptr(),
                buf.len() as CFIndex,
                K_CF_STRING_ENCODING_UTF8,
            );
            CFRelease(cfstring);
            if ok == 0 {
                return None;
            }

            let nul = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            let s = std::str::from_utf8(&buf[..nul]).ok()?;
            if s.is_empty() {
                return None;
            }
            Some(PathBuf::from(s))
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::resolve_translocated_path;

#[cfg(not(target_os = "macos"))]
pub fn resolve_translocated_path(_translocated: &std::path::Path) -> Option<std::path::PathBuf> {
    None
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn ffi_links_and_returns_none_for_normal_path() {
        // /usr/bin/cat is not translocated. SecTranslocate may return
        // NULL or an error CFRef for non-translocated paths; either way
        // our wrapper resolves cleanly to None. The point of this test
        // is to verify that the FFI symbols link at all (any missing
        // CoreFoundation / Security symbol would fail the test binary).
        let result = resolve_translocated_path(std::path::Path::new("/usr/bin/cat"));
        assert!(result.is_none());
    }

    #[test]
    fn empty_path_returns_none() {
        let result = resolve_translocated_path(std::path::Path::new(""));
        assert!(result.is_none());
    }

    #[test]
    fn nonexistent_path_returns_none_safely() {
        // Passing a path that doesn't exist must not crash.
        let result = resolve_translocated_path(std::path::Path::new(
            "/Volumes/DoesNotExist/Whatever.app/Contents/MacOS/bin",
        ));
        assert!(result.is_none());
    }
}
