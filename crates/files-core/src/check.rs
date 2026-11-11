//! Whole-list check before writing: nothing is written unless this passes.

use crate::{Entry, EntryKind, RelPath, DISK_RESERVE};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// What to do when a pasted file already exists in the target folder.
/// Only `Fail` reports such conflicts as problems; the others resolve them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictPolicy {
    Fail,
    Overwrite,
    /// Leave the existing file and do not paste this one.
    Skip,
    /// Keep both: the pasted file gets a free `name (2).ext` name.
    Rename,
}

/// `dir\stem (n).ext`, the name Explorer uses for "keep both".
pub fn numbered(path: &Path, n: u32) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default();
    let name = match path.extension() {
        Some(ext) => format!("{stem} ({n}).{}", ext.to_string_lossy()),
        None => format!("{stem} ({n})"),
    };
    path.with_file_name(name)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// Two paths with the same key where at least one is a file or `.partial`.
    Duplicate(String),
    PartialNameTooLong(String),
    PartialExists(PathBuf),
    Conflict {
        path: PathBuf,
        overwritable: bool,
    },
    NotEnoughSpace {
        need: u64,
        free: u64,
    },
}

/// Returns every problem in one pass; an empty list means "go".
/// `kind_of` gets absolute paths under `target`.
pub fn check(
    entries: &[Entry],
    target: &Path,
    policy: ConflictPolicy,
    kind_of: impl Fn(&Path) -> Option<EntryKind>,
    free_bytes: u64,
) -> Vec<Problem> {
    let mut problems = Vec::new();
    let mut seen: HashMap<String, bool> = HashMap::new(); // key -> is folder
    let mut add = |p: &RelPath, is_dir: bool, problems: &mut Vec<Problem>| {
        match seen.get(&p.key()) {
            Some(&was_dir) if was_dir && is_dir => {} // folders merge
            Some(_) => problems.push(Problem::Duplicate(p.to_string())),
            None => {
                seen.insert(p.key(), is_dir);
            }
        }
    };

    let mut dirs: Vec<RelPath> = Vec::new();
    let mut dir_keys = HashSet::new();
    for e in entries {
        for a in e.rel_path.ancestors() {
            add(&a, true, &mut problems);
            if dir_keys.insert(a.key()) {
                dirs.push(a);
            }
        }
        match e.kind {
            EntryKind::Dir => {
                add(&e.rel_path, true, &mut problems);
                if dir_keys.insert(e.rel_path.key()) {
                    dirs.push(e.rel_path.clone());
                }
            }
            EntryKind::File { .. } => {
                add(&e.rel_path, false, &mut problems);
                add(&e.rel_path.with_suffix(".partial"), false, &mut problems);
            }
        }
    }

    for d in &dirs {
        let path = d.to_path_under(target);
        if let Some(EntryKind::File { .. }) = kind_of(&path) {
            problems.push(Problem::Conflict {
                path,
                overwritable: false,
            });
        }
    }

    let mut need = Some(DISK_RESERVE);
    for e in entries {
        let EntryKind::File { size } = e.kind else {
            continue;
        };
        need = need.and_then(|n| n.checked_add(size.unwrap_or(0)));
        let path = e.rel_path.to_path_under(target);
        match kind_of(&path) {
            Some(EntryKind::Dir) => problems.push(Problem::Conflict {
                path,
                overwritable: false,
            }),
            Some(EntryKind::File { .. }) if policy == ConflictPolicy::Fail => {
                problems.push(Problem::Conflict {
                    path,
                    overwritable: true,
                })
            }
            _ => {}
        }
        let partial = e.rel_path.with_suffix(".partial");
        if partial.file_name().encode_utf16().count() > 255 {
            problems.push(Problem::PartialNameTooLong(e.rel_path.to_string()));
        }
        let partial_path = partial.to_path_under(target);
        if kind_of(&partial_path).is_some() {
            problems.push(Problem::PartialExists(partial_path));
        }
    }

    let need = need.unwrap_or(u64::MAX);
    if free_bytes < need {
        problems.push(Problem::NotEnoughSpace {
            need,
            free: free_bytes,
        });
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_names() {
        let n = |p: &str| numbered(Path::new(p), 2).to_string_lossy().into_owned();
        assert_eq!(n(r"D:\in\report.pdf"), r"D:\in\report (2).pdf");
        assert_eq!(n(r"D:\in\Makefile"), r"D:\in\Makefile (2)");
        assert_eq!(n(r"D:\in\.bashrc"), r"D:\in\.bashrc (2)");
        assert_eq!(n(r"D:\in\a.tar.gz"), r"D:\in\a.tar (2).gz");
    }

    #[test]
    fn only_fail_reports_existing_files() {
        for policy in [
            ConflictPolicy::Skip,
            ConflictPolicy::Rename,
            ConflictPolicy::Overwrite,
        ] {
            assert_eq!(
                run(&[file("f", None)], policy, &[("f", false)], 100 * GB),
                [],
                "{policy:?}"
            );
        }
        assert_eq!(
            run(
                &[file("f", None)],
                ConflictPolicy::Fail,
                &[("f", false)],
                100 * GB
            )
            .len(),
            1
        );
    }

    const GB: u64 = 1 << 30;

    fn file(p: &str, size: Option<u64>) -> Entry {
        Entry {
            rel_path: RelPath::parse(p).unwrap(),
            kind: EntryKind::File { size },
            modified: None,
        }
    }
    fn dir(p: &str) -> Entry {
        Entry {
            rel_path: RelPath::parse(p).unwrap(),
            kind: EntryKind::Dir,
            modified: None,
        }
    }
    /// Runs `check` against a fake target holding `existing` (name, is_dir).
    fn run(
        entries: &[Entry],
        policy: ConflictPolicy,
        existing: &[(&str, bool)],
        free: u64,
    ) -> Vec<Problem> {
        let target = Path::new("T:\\t");
        let existing: HashMap<PathBuf, EntryKind> = existing
            .iter()
            .map(|(p, d)| {
                let kind = if *d {
                    EntryKind::Dir
                } else {
                    EntryKind::File { size: Some(1) }
                };
                (target.join(p), kind)
            })
            .collect();
        check(entries, target, policy, |p| existing.get(p).copied(), free)
    }
    fn ok(entries: &[Entry]) -> Vec<Problem> {
        run(entries, ConflictPolicy::Fail, &[], 100 * GB)
    }

    #[test]
    fn nested_tree_without_folder_entries_passes() {
        assert_eq!(ok(&[file("x\\a", Some(1)), file("x\\b", Some(1))]), []);
        assert_eq!(ok(&[dir("X"), file("x\\a", Some(1))]), []);
    }

    #[test]
    fn duplicates() {
        let d = |p: &str| Problem::Duplicate(p.into());
        assert_eq!(
            ok(&[file("A.txt", None), file("a.txt", None)]),
            [d("a.txt"), d("a.txt.partial")]
        );
        assert_eq!(ok(&[file("x", None), file("x\\y", None)]), [d("x")]);
        assert_eq!(
            ok(&[file("a", None), file("a.partial", None)]),
            [d("a.partial")]
        );
    }

    #[test]
    fn conflicts_under_both_policies() {
        let t = Path::new("T:\\t");
        let es = [
            file("f", None),
            dir("d"),
            file("g", None),
            file("x\\y", None),
        ];
        let existing = [("f", false), ("d", false), ("g", true), ("x", false)];
        let fail = run(&es, ConflictPolicy::Fail, &existing, 100 * GB);
        assert!(fail.contains(&Problem::Conflict {
            path: t.join("f"),
            overwritable: true
        }));
        assert!(fail.contains(&Problem::Conflict {
            path: t.join("d"),
            overwritable: false
        }));
        assert!(fail.contains(&Problem::Conflict {
            path: t.join("g"),
            overwritable: false
        }));
        assert!(fail.contains(&Problem::Conflict {
            path: t.join("x"),
            overwritable: false
        }));
        let over = run(&es, ConflictPolicy::Overwrite, &existing, 100 * GB);
        assert!(!over.contains(&Problem::Conflict {
            path: t.join("f"),
            overwritable: true
        }));
        assert_eq!(over.len(), 3);
        // Existing folder where a folder goes: merge.
        assert_eq!(
            run(&[dir("d")], ConflictPolicy::Fail, &[("d", true)], 100 * GB),
            []
        );
    }

    #[test]
    fn partial_rules() {
        let t = Path::new("T:\\t");
        assert_eq!(
            run(
                &[file("f", None)],
                ConflictPolicy::Fail,
                &[("f.partial", false)],
                100 * GB
            ),
            [Problem::PartialExists(t.join("f.partial"))]
        );
        let long = "x".repeat(250);
        assert_eq!(
            ok(&[file(&long, None)]),
            [Problem::PartialNameTooLong(long)]
        );
    }

    #[test]
    fn disk_space() {
        let small = run(
            &[file("a", Some(5 * GB)), file("b", None)],
            ConflictPolicy::Fail,
            &[],
            3 * GB,
        );
        assert_eq!(
            small,
            [Problem::NotEnoughSpace {
                need: 6 * GB,
                free: 3 * GB
            }]
        );
        let huge = u64::MAX / 2 + 1;
        let over = ok(&[file("a", Some(huge)), file("b", Some(huge))]);
        assert_eq!(
            over,
            [Problem::NotEnoughSpace {
                need: u64::MAX,
                free: 100 * GB
            }]
        );
    }
}
