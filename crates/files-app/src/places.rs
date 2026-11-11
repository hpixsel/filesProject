//! Shell places that are not folders: This PC and the Recycle Bin, under the
//! names Windows itself uses (`shell:...`, which Explorer accepts too).

use crate::Item;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const THIS_PC: &str = "shell:MyComputerFolder";
pub const RECYCLE_BIN: &str = "shell:RecycleBinFolder";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Place {
    #[default]
    Folder,
    ThisPc,
    RecycleBin,
}

impl Place {
    pub fn of(path: &Path) -> Place {
        match path.to_str() {
            Some(s) if s.eq_ignore_ascii_case(THIS_PC) => Place::ThisPc,
            Some(s) if s.eq_ignore_ascii_case(RECYCLE_BIN) => Place::RecycleBin,
            _ => Place::Folder,
        }
    }
}

/// True for `shell:...` locations, which are not paths on a disk.
pub fn is_place(path: &Path) -> bool {
    Place::of(path) != Place::Folder
}

/// What the Recycle Bin holds on the given drives, newest first. Each item's
/// `path` is the `$R...` file holding the content; its name, size and
/// deletion time come from the `$I...` file next to it.
pub fn recycle_bin(roots: &[PathBuf]) -> Vec<Item> {
    let mut items: Vec<Item> = roots
        .iter()
        // One folder per user SID; other users' folders cannot be read.
        .filter_map(|root| std::fs::read_dir(root.join("$Recycle.Bin")).ok())
        .flatten()
        .flatten()
        .filter_map(|user| std::fs::read_dir(user.path()).ok())
        .flatten()
        .flatten()
        .filter_map(|e| {
            let info = e.path();
            let content = content_file(&info)?;
            let (size, deleted, original) = read_info(&info).ok()?;
            Some(Item {
                name: original.file_name()?.to_string_lossy().into_owned(),
                is_dir: content.is_dir(),
                // Folders too: the size is known, so no folder-size scan.
                size: Some(size),
                modified: Some(deleted),
                origin: original.parent().map(Path::to_path_buf),
                link: None,
                path: content,
            })
        })
        .collect();
    items.sort_by_key(|i| std::cmp::Reverse(i.modified));
    items
}

/// `$I123.txt` -> `$R123.txt`, when that file exists.
fn content_file(info: &Path) -> Option<PathBuf> {
    let name = info.file_name()?.to_str()?;
    let content = info.with_file_name(format!("$R{}", name.strip_prefix("$I")?));
    content.exists().then_some(content)
}

fn info_file(content: &Path) -> Option<PathBuf> {
    let name = content.file_name()?.to_str()?;
    Some(content.with_file_name(format!("$I{}", name.strip_prefix("$R")?)))
}

/// Parses a `$I` file: (size, deletion time, original path). Version 1
/// (Vista to 8.1) has a fixed 260-character path, version 2 a counted one.
fn read_info(info: &Path) -> io::Result<(u64, SystemTime, PathBuf)> {
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "not a Recycle Bin $I file");
    let b = std::fs::read(info)?;
    let u64_at = |at: usize| -> io::Result<u64> {
        let bytes = b.get(at..at + 8).ok_or_else(bad)?;
        Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
    };
    let name_at = match u64_at(0)? {
        1 => 24,
        2 => 28,
        _ => return Err(bad()),
    };
    let wide: Vec<u16> = b
        .get(name_at..)
        .ok_or_else(bad)?
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| u16::from_le_bytes(c))
        .take_while(|&c| c != 0)
        .collect();
    if wide.is_empty() {
        return Err(bad());
    }
    // FILETIME counts 100 ns steps from 1601.
    let since_1601 = Duration::from_nanos(u64_at(16)?.saturating_mul(100));
    let deleted = (UNIX_EPOCH - Duration::from_secs(11_644_473_600)) + since_1601;
    Ok((
        u64_at(8)?,
        deleted,
        PathBuf::from(String::from_utf16_lossy(&wide)),
    ))
}

/// Puts a Recycle Bin item (its `$R` path) back where it was deleted from,
/// as Explorer does. Refuses when something already sits there.
pub fn restore(content: &Path) -> io::Result<PathBuf> {
    let info = info_file(content).ok_or(io::ErrorKind::InvalidInput)?;
    let (_, _, original) = read_info(&info)?;
    if original.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", original.display()),
        ));
    }
    if let Some(dir) = original.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // The bin lives on the same drive as the original, so this is a rename.
    std::fs::rename(content, &original)?;
    std::fs::remove_file(&info)?;
    Ok(original)
}

/// A Recycle Bin item's two files, for deleting it for good.
pub fn bin_files(content: &Path) -> Vec<PathBuf> {
    std::iter::once(content.to_path_buf())
        .chain(info_file(content))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_info(path: &Path, original: &str, size: u64) {
        let name: Vec<u16> = original.encode_utf16().chain([0]).collect();
        let mut b = 2u64.to_le_bytes().to_vec();
        b.extend(size.to_le_bytes());
        b.extend(133_000_000_000_000_000u64.to_le_bytes()); // a 2022 FILETIME
        b.extend((name.len() as u32).to_le_bytes());
        b.extend(name.iter().flat_map(|c| c.to_le_bytes()));
        std::fs::write(path, b).unwrap();
    }

    #[test]
    fn lists_and_restores_a_deleted_file() {
        let root = std::env::temp_dir().join(format!("files-bin-{}", std::process::id()));
        let bin = root.join("$Recycle.Bin").join("S-1-5-21-test");
        std::fs::create_dir_all(&bin).unwrap();
        let original = root.join("docs").join("Zażółć.txt");
        write_info(&bin.join("$IABC123.txt"), &original.to_string_lossy(), 5);
        std::fs::write(bin.join("$RABC123.txt"), b"hello").unwrap();
        write_info(&bin.join("$IORPHAN.txt"), r"C:\gone.txt", 1); // no $R: skipped

        let items = recycle_bin(std::slice::from_ref(&root));
        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert_eq!(
            (it.name.as_str(), it.size, it.is_dir),
            ("Zażółć.txt", Some(5), false)
        );
        assert_eq!(it.origin.as_deref(), original.parent());
        let year = it
            .modified
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            / 31_557_600;
        assert_eq!(1970 + year, 2022);

        assert_eq!(restore(&it.path).unwrap(), original);
        assert_eq!(std::fs::read(&original).unwrap(), b"hello");
        assert!(!bin.join("$IABC123.txt").exists());
        assert!(recycle_bin(std::slice::from_ref(&root)).is_empty());

        // Something already at the original path: nothing moves.
        write_info(&bin.join("$IDEF.txt"), &original.to_string_lossy(), 5);
        std::fs::write(bin.join("$RDEF.txt"), b"other").unwrap();
        assert!(restore(&bin.join("$RDEF.txt")).is_err());
        assert!(bin.join("$RDEF.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn places_by_name() {
        assert_eq!(
            Place::of(Path::new("shell:mycomputerfolder")),
            Place::ThisPc
        );
        assert_eq!(Place::of(Path::new(RECYCLE_BIN)), Place::RecycleBin);
        assert_eq!(Place::of(Path::new(r"C:\shell")), Place::Folder);
    }
}
