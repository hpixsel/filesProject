//! Folder sizes in the background: a small fixed pool in background mode,
//! cancelled when the scan is dropped (e.g. on navigation).

use crate::CancelToken;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Folders scanned at once. Folder sizes are disk-bound; more workers only
/// make a disk seek more (G3).
// shortcut: one pool for all volumes; per-volume limits (2 per disk, 1 per share) when M2 grows a queue
pub const WORKERS: usize = 2;

pub struct SizeScan {
    cancel: CancelToken,
    results: Arc<Mutex<HashMap<PathBuf, u64>>>,
}

impl SizeScan {
    /// Total bytes under `dir`, once its scan has finished.
    pub fn get(&self, dir: &Path) -> Option<u64> {
        self.results.lock().unwrap().get(dir).copied()
    }

    pub fn results(&self) -> Arc<Mutex<HashMap<PathBuf, u64>>> {
        self.results.clone()
    }
}

impl Drop for SizeScan {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Starts computing the total size of each of `dirs`.
pub fn scan_sizes(dirs: Vec<PathBuf>) -> SizeScan {
    let cancel = CancelToken::default();
    let results = Arc::new(Mutex::new(HashMap::new()));
    let queue = Arc::new(Mutex::new(dirs.into_iter().rev().collect::<Vec<_>>()));
    for _ in 0..WORKERS {
        let (cancel, results, queue) = (cancel.clone(), results.clone(), queue.clone());
        std::thread::spawn(move || {
            let _bg = files_win::io::background_mode();
            while let Some(dir) = queue.lock().unwrap().pop() {
                match dir_size(&dir, &cancel) {
                    Some(n) => {
                        results.lock().unwrap().insert(dir, n);
                    }
                    None => return, // cancelled
                }
            }
        });
    }
    SizeScan { cancel, results }
}

/// Sum of file sizes below `dir`. Links and junctions are not followed, so
/// nothing is counted twice and cycles cannot happen. Unreadable folders
/// count as empty. `None` means cancelled.
pub fn dir_size(dir: &Path, cancel: &CancelToken) -> Option<u64> {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if cancel.is_cancelled() {
            return None;
        }
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let Ok(t) = e.file_type() else { continue };
            if t.is_symlink() {
                continue; // covers junctions too
            }
            if t.is_dir() {
                stack.push(e.path());
            } else if let Ok(m) = e.metadata() {
                total += m.len();
            }
        }
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn sums_nested_files_and_reports_each_folder() {
        let root = std::env::temp_dir().join(format!("files-test-sizes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::create_dir_all(root.join("c")).unwrap();
        std::fs::write(root.join("a/1"), [0u8; 10]).unwrap();
        std::fs::write(root.join("a/b/2"), [0u8; 5]).unwrap();
        std::fs::write(root.join("c/3"), [0u8; 7]).unwrap();

        let cancel = CancelToken::default();
        assert_eq!(dir_size(&root, &cancel), Some(22));

        let scan = scan_sizes(vec![root.join("a"), root.join("c")]);
        let until = Instant::now() + Duration::from_secs(5);
        while (scan.get(&root.join("a")).is_none() || scan.get(&root.join("c")).is_none())
            && Instant::now() < until
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(scan.get(&root.join("a")), Some(15));
        assert_eq!(scan.get(&root.join("c")), Some(7));

        cancel.cancel();
        assert_eq!(dir_size(&root, &cancel), None);
        std::fs::remove_dir_all(root).unwrap();
    }
}
