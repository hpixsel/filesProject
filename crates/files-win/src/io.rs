//! File and thread helpers that need Win32 calls.

use crate::{Error, Result};
use files_core::ALIGN;
use std::alloc::{alloc_zeroed, dealloc, handle_alloc_error, Layout};
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::ops::{Deref, DerefMut};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    BOOLEAN, ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS, GENERIC_WRITE, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    FileDispositionInfo, GetDiskFreeSpaceExW, MoveFileExW, SetFileInformationByHandle, DELETE,
    FILE_DISPOSITION_INFO, FILE_FLAG_NO_BUFFERING, MOVE_FILE_FLAGS,
};
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN, THREAD_MODE_BACKGROUND_END,
};

/// Low I/O and memory priority for the current thread while alive.
pub struct BackgroundGuard(());

pub fn background_mode() -> Result<BackgroundGuard> {
    // SAFETY: pseudo-handle of the current thread.
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN)? };
    Ok(BackgroundGuard(()))
}

impl Drop for BackgroundGuard {
    fn drop(&mut self) {
        // SAFETY: as above; failure only leaves the thread in background mode.
        let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_END) };
    }
}

/// Absolute path in `\\?\` form, so raw Win32 calls accept long paths.
pub fn long_path(p: &Path) -> std::io::Result<PathBuf> {
    let abs = std::path::absolute(p)?;
    let s = abs.to_string_lossy();
    Ok(if s.starts_with(r"\\?\") {
        abs
    } else if let Some(unc) = s.strip_prefix(r"\\") {
        PathBuf::from(format!(r"\\?\UNC\{unc}"))
    } else {
        PathBuf::from(format!(r"\\?\{s}"))
    })
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

/// Zeroed buffer aligned to [`ALIGN`], as unbuffered writes require.
pub struct AlignedBuf {
    ptr: *mut u8,
    layout: Layout,
}

impl AlignedBuf {
    pub fn new(len: usize) -> AlignedBuf {
        let layout = Layout::from_size_align(len.max(ALIGN), ALIGN).expect("valid layout");
        // SAFETY: layout has non-zero size.
        let ptr = unsafe { alloc_zeroed(layout) };
        if ptr.is_null() {
            handle_alloc_error(layout);
        }
        AlignedBuf { ptr, layout }
    }
}

impl Deref for AlignedBuf {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        // SAFETY: zero-initialised allocation of `layout.size()` bytes owned by self.
        unsafe { std::slice::from_raw_parts(self.ptr, self.layout.size()) }
    }
}

impl DerefMut for AlignedBuf {
    fn deref_mut(&mut self) -> &mut [u8] {
        // SAFETY: as above, uniquely borrowed.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.layout.size()) }
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        // SAFETY: allocated in `new` with the same layout.
        unsafe { dealloc(self.ptr, self.layout) };
    }
}

// SAFETY: plain owned heap memory.
unsafe impl Send for AlignedBuf {}

/// Creates `<path>` (must not exist) and marks it delete-on-close, so it
/// disappears on every exit path until [`set_delete_on_close`]`(false)`.
pub fn create_partial(path: &Path, unbuffered: bool) -> Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true)
        .create_new(true)
        .access_mode(GENERIC_WRITE.0 | DELETE.0);
    if unbuffered {
        opts.custom_flags(FILE_FLAG_NO_BUFFERING.0);
    }
    let file = opts.open(path)?;
    if let Err(e) = set_delete_on_close(&file, true) {
        drop(file);
        let _ = std::fs::remove_file(path); // we created it a moment ago
        return Err(e);
    }
    Ok(file)
}

pub fn set_delete_on_close(file: &File, delete: bool) -> Result<()> {
    let info = FILE_DISPOSITION_INFO {
        DeleteFile: BOOLEAN(delete as u8),
    };
    // SAFETY: valid handle owned by `file`; `info` outlives the call.
    unsafe {
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileDispositionInfo,
            (&info as *const FILE_DISPOSITION_INFO).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )?
    };
    Ok(())
}

/// Writes `buf[..len]`. Precondition: `len % ALIGN == 0` unless `last`; the
/// last write is zero-padded to `ALIGN` and the caller then calls `set_len`.
pub fn write_aligned(
    file: &mut File,
    buf: &mut [u8],
    len: usize,
    last: bool,
) -> std::io::Result<()> {
    debug_assert!(last || len.is_multiple_of(ALIGN));
    // shortcut: assumes sector size <= 4096; query FileStorageInfo if a disk rejects unbuffered writes
    let padded = len.div_ceil(ALIGN) * ALIGN;
    buf[len..padded].fill(0);
    file.write_all(&buf[..padded])
}

/// Renames without replacing an existing target. Works while a handle to
/// `from` is open, because std opens files with `FILE_SHARE_DELETE`.
pub fn rename_no_replace(from: &Path, to: &Path) -> Result<()> {
    let (f, t) = (
        wide(long_path(from)?.as_os_str()),
        wide(long_path(to)?.as_os_str()),
    );
    // SAFETY: both buffers are NUL-terminated and outlive the call.
    let r = unsafe { MoveFileExW(PCWSTR(f.as_ptr()), PCWSTR(t.as_ptr()), MOVE_FILE_FLAGS(0)) };
    r.map_err(|e| {
        if e.code() == ERROR_ALREADY_EXISTS.to_hresult()
            || e.code() == ERROR_FILE_EXISTS.to_hresult()
        {
            Error::TargetExists(to.to_path_buf())
        } else {
            Error::Win(e)
        }
    })
}

/// Free bytes available to the caller on the volume holding `dir`.
pub fn free_bytes(dir: &Path) -> Result<u64> {
    Ok(disk_space(dir)?.0)
}

/// (free, total) bytes on the volume holding `dir`, as this user sees them.
pub fn disk_space(dir: &Path) -> Result<(u64, u64)> {
    let mut s = long_path(dir)?.into_os_string();
    if !s.to_string_lossy().ends_with('\\') {
        s.push("\\");
    }
    let w = wide(&s);
    let (mut free, mut total) = (0u64, 0u64);
    // SAFETY: `w` is NUL-terminated; both outputs outlive the call.
    unsafe { GetDiskFreeSpaceExW(PCWSTR(w.as_ptr()), Some(&mut free), Some(&mut total), None)? };
    Ok((free, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("files-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn long_path_forms() {
        assert_eq!(
            long_path(Path::new(r"C:\a\b")).unwrap(),
            Path::new(r"\\?\C:\a\b")
        );
        assert_eq!(
            long_path(Path::new(r"\\srv\share\x")).unwrap(),
            Path::new(r"\\?\UNC\srv\share\x")
        );
        assert_eq!(
            long_path(Path::new(r"\\?\C:\a")).unwrap(),
            Path::new(r"\\?\C:\a")
        );
    }

    #[test]
    fn rename_no_replace_refuses_existing_target() {
        let d = tmp("rename");
        std::fs::write(d.join("a"), "a").unwrap();
        std::fs::write(d.join("b"), "b").unwrap();
        assert!(matches!(
            rename_no_replace(&d.join("a"), &d.join("b")),
            Err(Error::TargetExists(_))
        ));
        rename_no_replace(&d.join("a"), &d.join("c")).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("c")).unwrap(), "a");
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn free_bytes_on_drive_root_and_folder() {
        let d = tmp("free");
        assert!(free_bytes(&d).unwrap() > 0);
        let root: PathBuf = d.ancestors().last().unwrap().to_path_buf();
        assert!(free_bytes(&root).unwrap() > 0);
        std::fs::remove_dir_all(d).unwrap();
    }

    /// Risk R5: delete-on-close must work together with NO_BUFFERING.
    #[test]
    fn delete_on_close_with_unbuffered_writes() {
        let d = tmp("r5");
        let mut buf = AlignedBuf::new(2 * ALIGN);
        buf[..5].copy_from_slice(b"hello");

        let gone = d.join("gone.partial");
        let mut f = create_partial(&gone, true).unwrap();
        write_aligned(&mut f, &mut buf, 5, true).unwrap();
        drop(f);
        assert!(!gone.exists(), "flag set: file must be deleted on close");

        let kept = d.join("kept.partial");
        let mut f = create_partial(&kept, true).unwrap();
        write_aligned(&mut f, &mut buf, 5, true).unwrap();
        f.set_len(5).unwrap();
        set_delete_on_close(&f, false).unwrap();
        rename_no_replace(&kept, &d.join("kept")).unwrap(); // handle still open
        drop(f);
        assert_eq!(std::fs::read(d.join("kept")).unwrap(), b"hello");

        assert!(
            create_partial(&d.join("kept"), false).is_err(),
            "create_new must refuse"
        );
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn buffered_partial_with_flag_cleared_survives() {
        let d = tmp("buffered");
        let p = d.join("x.partial");
        let mut f = create_partial(&p, false).unwrap();
        f.write_all(b"abc").unwrap();
        set_delete_on_close(&f, false).unwrap();
        drop(f);
        assert_eq!(std::fs::read(&p).unwrap(), b"abc");
        std::fs::remove_dir_all(d).unwrap();
    }
}
