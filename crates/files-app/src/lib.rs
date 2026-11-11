//! View state of the explorer, independent of the UI toolkit: folder
//! listing and navigation history.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
}

/// Lists `dir`: folders first, then files, each by name ignoring case.
/// Entries that cannot be read (permissions, races) are skipped.
pub fn list_dir(dir: &Path) -> io::Result<Vec<Item>> {
    let mut items: Vec<Item> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            // Junctions such as "Documents and Settings" are reparse points
            // that std reports as symlinks, not dirs; the attribute says dir.
            let is_dir = meta.is_dir() || {
                use std::os::windows::fs::MetadataExt;
                meta.file_attributes() & 0x10 != 0 // FILE_ATTRIBUTE_DIRECTORY
            };
            Some(Item {
                name: e.file_name().to_string_lossy().into_owned(),
                path: e.path(),
                is_dir,
                size: (!is_dir).then_some(meta.len()),
                modified: meta.modified().ok(),
            })
        })
        .collect();
    items.sort_by_cached_key(|i| (!i.is_dir, i.name.to_lowercase()));
    Ok(items)
}

/// Back / forward / up navigation, like a browser.
#[derive(Debug, Clone)]
pub struct History {
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    current: PathBuf,
}

impl History {
    pub fn new(start: PathBuf) -> History {
        History {
            back: Vec::new(),
            forward: Vec::new(),
            current: start,
        }
    }

    pub fn current(&self) -> &Path {
        &self.current
    }

    /// Goes to `dir`; a new location clears the forward stack.
    pub fn go(&mut self, dir: PathBuf) {
        if dir != self.current {
            self.back.push(std::mem::replace(&mut self.current, dir));
            self.forward.clear();
        }
    }

    pub fn back(&mut self) -> bool {
        let Some(prev) = self.back.pop() else {
            return false;
        };
        self.forward
            .push(std::mem::replace(&mut self.current, prev));
        true
    }

    pub fn forward(&mut self) -> bool {
        let Some(next) = self.forward.pop() else {
            return false;
        };
        self.back.push(std::mem::replace(&mut self.current, next));
        true
    }

    pub fn up(&mut self) -> bool {
        match self.current.parent() {
            Some(p) => {
                self.go(p.to_path_buf());
                true
            }
            None => false,
        }
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }
}

/// Human-readable size: B, KB, MB, GB (1024-based).
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_folders_first_case_insensitive() {
        let d = std::env::temp_dir().join(format!("files-test-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("zeta")).unwrap();
        std::fs::create_dir_all(d.join("Alpha")).unwrap();
        std::fs::write(d.join("b.txt"), "12345").unwrap();
        std::fs::write(d.join("A.txt"), "").unwrap();
        let items = list_dir(&d).unwrap();
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "zeta", "A.txt", "b.txt"]);
        assert_eq!(items[3].size, Some(5));
        assert_eq!(items[0].size, None);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn history_back_forward_up() {
        let mut h = History::new(PathBuf::from(r"C:\a"));
        h.go(PathBuf::from(r"C:\a\b"));
        h.go(PathBuf::from(r"C:\a\b\c"));
        assert!(h.back());
        assert_eq!(h.current(), Path::new(r"C:\a\b"));
        assert!(h.forward());
        assert_eq!(h.current(), Path::new(r"C:\a\b\c"));
        assert!(h.up());
        assert_eq!(h.current(), Path::new(r"C:\a\b"));
        assert!(!h.can_forward(), "a new location clears forward");
        h.go(PathBuf::from(r"C:\a\b")); // same place: no history entry
        assert!(h.back());
        assert_eq!(h.current(), Path::new(r"C:\a\b\c"));
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(5 << 30), "5.0 GB");
    }
}
