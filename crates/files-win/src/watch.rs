//! Watching a folder for changes (files added, removed, renamed, written),
//! so the list can reload without F5.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows::core::HSTRING;
use windows::Win32::Foundation::WAIT_OBJECT_0;
use windows::Win32::Storage::FileSystem::{
    FindCloseChangeNotification, FindFirstChangeNotificationW, FindNextChangeNotification,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
    FILE_NOTIFY_CHANGE_SIZE,
};
use windows::Win32::System::Threading::WaitForSingleObject;

/// Stops watching when dropped.
pub struct Watch(Arc<AtomicBool>);

impl Drop for Watch {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Calls `on_change` (on a background thread) whenever something in `dir`
/// changes; its subfolders' contents are not watched.
pub fn watch(dir: &Path, on_change: impl Fn() + Send + 'static) -> crate::Result<Watch> {
    let what = FILE_NOTIFY_CHANGE_FILE_NAME
        | FILE_NOTIFY_CHANGE_DIR_NAME
        | FILE_NOTIFY_CHANGE_SIZE
        | FILE_NOTIFY_CHANGE_LAST_WRITE;
    // SAFETY: a valid path; the handle is closed by the thread below.
    let handle =
        unsafe { FindFirstChangeNotificationW(&HSTRING::from(dir.as_os_str()), false, what)? };
    let raw = handle.0 as isize; // a HANDLE is not Send; its value is
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    std::thread::spawn(move || {
        let handle = windows::Win32::Foundation::HANDLE(raw as *mut _);
        // Wakes twice a second to notice `stop`; closing a handle that is
        // being waited on is undefined, so this thread alone closes it.
        while !stopped.load(Ordering::SeqCst) {
            // SAFETY: the handle stays open until this loop ends.
            let woke = unsafe { WaitForSingleObject(handle, 500) };
            if woke == WAIT_OBJECT_0 && !stopped.load(Ordering::SeqCst) {
                on_change();
                // SAFETY: as above.
                if unsafe { FindNextChangeNotification(handle) }.is_err() {
                    break; // the folder went away
                }
            }
        }
        // SAFETY: opened above, closed once.
        let _ = unsafe { FindCloseChangeNotification(handle) };
    });
    Ok(Watch(stop))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn notices_a_new_file() {
        let dir = std::env::temp_dir().join(format!("files-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let w = watch(&dir, move || {
            let _ = tx.send(());
        })
        .unwrap();
        std::fs::write(dir.join("new.txt"), b"x").unwrap();
        assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok());
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
