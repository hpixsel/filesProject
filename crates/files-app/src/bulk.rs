//! Renaming many files at once from a pattern.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The new paths for `paths` (in order) from `pattern`: `#` becomes the
/// number (counting from `start`, zero-padded to the widest), `{name}` the
/// old name without its extension; each file keeps its extension.
pub fn names(paths: &[PathBuf], pattern: &str, start: usize) -> Vec<PathBuf> {
    let width = (start + paths.len().saturating_sub(1)).to_string().len();
    paths
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let number = format!("{:0width$}", start + i);
            let mut name = pattern.replace("{name}", &stem).replace('#', &number);
            if let Some(ext) = p.extension() {
                name = format!("{name}.{}", ext.to_string_lossy());
            }
            p.with_file_name(name)
        })
        .collect()
}

/// Why the renames cannot all happen: two files would get the same name,
/// or a name is taken by a file that is not being renamed.
pub fn clash(from: &[PathBuf], to: &[PathBuf]) -> Option<PathBuf> {
    let key = |p: &Path| p.to_string_lossy().to_lowercase();
    let sources: HashSet<String> = from.iter().map(|p| key(p)).collect();
    let mut seen = HashSet::new();
    to.iter()
        .find(|t| !seen.insert(key(t)) || (t.exists() && !sources.contains(&key(t))))
        .cloned()
}

/// Renames `from[i]` to `to[i]`, through temporary names first so that a
/// new name may be one of the old ones ("2.jpg" -> "1.jpg", "1.jpg" -> "2.jpg").
pub fn rename_all(from: &[PathBuf], to: &[PathBuf]) -> std::io::Result<()> {
    let temp: Vec<PathBuf> = from
        .iter()
        .enumerate()
        .map(|(i, p)| p.with_file_name(format!(".filesproject-rename-{}-{i}", std::process::id())))
        .collect();
    // A failure puts back the original name of every file not yet done, so
    // nothing is left under a temporary name.
    let undo = |done: usize| {
        for (t, f) in temp[..done].iter().zip(from) {
            let _ = std::fs::rename(t, f);
        }
    };
    for (i, (f, t)) in from.iter().zip(&temp).enumerate() {
        if let Err(e) = std::fs::rename(f, t) {
            undo(i);
            return Err(e);
        }
    }
    for (i, (t, n)) in temp.iter().zip(to).enumerate() {
        if let Err(e) = std::fs::rename(t, n) {
            for (t, f) in temp[i..].iter().zip(&from[i..]) {
                let _ = std::fs::rename(t, f);
            }
            return Err(e);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_numbers_and_names() {
        let files: Vec<PathBuf> = (0..12)
            .map(|i| PathBuf::from(format!(r"C:\a\IMG_{i}.jpg")))
            .collect();
        let new = names(&files, "Wakacje #", 1);
        assert_eq!(new[0], PathBuf::from(r"C:\a\Wakacje 01.jpg"));
        assert_eq!(new[11], PathBuf::from(r"C:\a\Wakacje 12.jpg"));
        let kept = names(&files[..1], "{name} - kopia", 1);
        assert_eq!(kept[0], PathBuf::from(r"C:\a\IMG_0 - kopia.jpg"));
    }

    #[test]
    fn swaps_and_clashes() {
        let d = std::env::temp_dir().join(format!("files-bulk-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let (a, b, c) = (d.join("1.txt"), d.join("2.txt"), d.join("other.txt"));
        std::fs::write(&a, "one").unwrap();
        std::fs::write(&b, "two").unwrap();
        std::fs::write(&c, "x").unwrap();
        let from = vec![a.clone(), b.clone()];
        assert_eq!(
            clash(&from, &[b.clone(), a.clone()]),
            None,
            "a swap is fine"
        );
        assert_eq!(
            clash(&from, &[c.clone(), d.join("3.txt")]),
            Some(c.clone()),
            "taken"
        );
        assert_eq!(
            clash(&from, &[d.join("x.txt"), d.join("X.txt")]),
            Some(d.join("X.txt")),
            "twice"
        );
        rename_all(&from, &[b.clone(), a.clone()]).unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "two");
        let _ = std::fs::remove_dir_all(&d);
    }
}
