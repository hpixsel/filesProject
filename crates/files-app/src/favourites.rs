//! The sidebar's groups of favourite folders, in
//! `%APPDATA%\FilesProject\favourites.txt`, editable by hand: a `[Name]` line
//! starts a group, the lines after it are its folders, and `[*drives]` is
//! where the Drives group sits. A file without headers (the old one-list
//! format) is one "Favourites" group.

use std::path::{Path, PathBuf};

pub fn file() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(|d| PathBuf::from(d).join("FilesProject").join("favourites.txt"))
}

#[derive(Debug, Clone, PartialEq)]
pub enum Group {
    Folders {
        name: String,
        paths: Vec<PathBuf>,
    },
    /// The drives, listed live; only its place is stored.
    Drives,
}

/// The sidebar, top to bottom. Always holds exactly one `Drives`.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout(pub Vec<Group>);

const DRIVES: &str = "[*drives]";

impl Default for Layout {
    /// Desktop, Documents and Downloads, then the drives.
    fn default() -> Self {
        let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
        let paths = ["Desktop", "Documents", "Downloads"]
            .iter()
            .filter_map(|d| home.as_ref().map(|h| h.join(d)))
            .filter(|p| p.is_dir())
            .collect();
        Layout(vec![
            Group::Folders {
                name: "Favourites".into(),
                paths,
            },
            Group::Drives,
        ])
    }
}

pub fn parse(text: &str) -> Layout {
    let mut groups = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line == DRIVES {
            if !groups.contains(&Group::Drives) {
                groups.push(Group::Drives);
            }
        } else if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            groups.push(Group::Folders {
                name: name.to_string(),
                paths: Vec::new(),
            });
        } else {
            if !matches!(groups.last(), Some(Group::Folders { .. })) {
                groups.push(Group::Folders {
                    name: "Favourites".into(),
                    paths: Vec::new(),
                });
            }
            if let Some(Group::Folders { paths, .. }) = groups.last_mut() {
                paths.push(PathBuf::from(line));
            }
        }
    }
    if !groups.contains(&Group::Drives) {
        groups.push(Group::Drives);
    }
    Layout(groups)
}

pub fn to_text(l: &Layout) -> String {
    let mut text = String::new();
    for g in &l.0 {
        match g {
            Group::Drives => text += &format!("{DRIVES}\r\n"),
            Group::Folders { name, paths } => {
                text += &format!("[{name}]\r\n");
                for p in paths {
                    text += &format!("{}\r\n", p.display());
                }
            }
        }
    }
    text
}

pub fn load(file: &Path) -> Layout {
    match std::fs::read_to_string(file) {
        Ok(text) => parse(&text),
        Err(_) => Layout::default(),
    }
}

pub fn save(file: &Path, l: &Layout) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, to_text(l))
}

impl Layout {
    fn folders_mut(&mut self, g: usize) -> Option<&mut Vec<PathBuf>> {
        match self.0.get_mut(g)? {
            Group::Folders { paths, .. } => Some(paths),
            Group::Drives => None,
        }
    }

    pub fn contains(&self, dir: &Path) -> bool {
        self.0
            .iter()
            .any(|g| matches!(g, Group::Folders { paths, .. } if paths.iter().any(|p| p == dir)))
    }

    /// Ctrl+D: removes `dir` wherever it is, or adds it to the first group.
    /// Returns whether it is now a favourite.
    pub fn toggle(&mut self, dir: &Path) -> bool {
        if self.contains(dir) {
            for g in &mut self.0 {
                if let Group::Folders { paths, .. } = g {
                    paths.retain(|p| p != dir);
                }
            }
            return false;
        }
        let first = self
            .0
            .iter()
            .position(|g| matches!(g, Group::Folders { .. }));
        let g = first.unwrap_or_else(|| self.add_group("Favourites".into()));
        self.add_to(g, dir.to_path_buf());
        true
    }

    /// Adds `dir` to the end of group `g` unless it is already there.
    pub fn add_to(&mut self, g: usize, dir: PathBuf) {
        if let Some(paths) = self.folders_mut(g) {
            if !paths.contains(&dir) {
                paths.push(dir);
            }
        }
    }

    /// A new empty group at the end; returns its index.
    pub fn add_group(&mut self, name: String) -> usize {
        self.0.push(Group::Folders {
            name,
            paths: Vec::new(),
        });
        self.0.len() - 1
    }

    pub fn rename_group(&mut self, g: usize, new: String) {
        if let Some(Group::Folders { name, .. }) = self.0.get_mut(g) {
            *name = new;
        }
    }

    /// Removes a group of folders (the drives stay).
    pub fn remove_group(&mut self, g: usize) {
        if matches!(self.0.get(g), Some(Group::Folders { .. })) {
            self.0.remove(g);
        }
    }

    pub fn remove_item(&mut self, g: usize, i: usize) {
        if let Some(paths) = self.folders_mut(g).filter(|p| i < p.len()) {
            paths.remove(i);
        }
    }

    /// Moves item `i` of group `g` to just before item `ti` of group `tg`
    /// (`ti` past the end: the end).
    pub fn move_item(&mut self, (g, i): (usize, usize), (tg, ti): (usize, usize)) {
        let Some(path) = self
            .folders_mut(g)
            .filter(|p| i < p.len())
            .map(|p| p.remove(i))
        else {
            return;
        };
        let ti = if g == tg && i < ti { ti - 1 } else { ti };
        match self.folders_mut(tg) {
            Some(paths) => paths.insert(ti.min(paths.len()), path),
            None => self.folders_mut(g).unwrap().insert(i, path), // not a folder group: undo
        }
    }

    /// Moves group `from` to just before group `to` (`to` past the end: last).
    pub fn move_group(&mut self, from: usize, to: usize) {
        if from >= self.0.len() {
            return;
        }
        let g = self.0.remove(from);
        let to = if from < to { to - 1 } else { to };
        self.0.insert(to.min(self.0.len()), g);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(l: &Layout, g: usize) -> Vec<String> {
        match &l.0[g] {
            Group::Folders { paths, .. } => paths.iter().map(|p| p.display().to_string()).collect(),
            Group::Drives => vec!["drives".into()],
        }
    }

    #[test]
    fn old_file_round_trip_and_toggle() {
        let mut l = parse("C:\\Temp\r\nD:\\Zażółć gęślą\r\n");
        assert_eq!(names(&l, 0), [r"C:\Temp", r"D:\Zażółć gęślą"]);
        assert_eq!(l.0[1], Group::Drives, "drives added at the end");
        assert_eq!(parse(&to_text(&l)), l);
        assert!(!l.toggle(Path::new(r"C:\Temp")));
        assert!(l.toggle(Path::new(r"E:\new")));
        assert_eq!(names(&l, 0), [r"D:\Zażółć gęślą", r"E:\new"]);
    }

    #[test]
    fn groups_and_moves() {
        let mut l = parse("[*drives]\n[Work]\nC:\\a\nC:\\b\nC:\\c\n[Play]\nD:\\x\n");
        assert_eq!(l.0[0], Group::Drives);
        l.move_item((1, 0), (1, 3)); // a to the end of Work
        assert_eq!(names(&l, 1), [r"C:\b", r"C:\c", r"C:\a"]);
        l.move_item((1, 2), (2, 0)); // a to the front of Play
        assert_eq!(names(&l, 2), [r"C:\a", r"D:\x"]);
        l.move_item((1, 0), (0, 0)); // onto the drives: nothing moves
        assert_eq!(names(&l, 1), [r"C:\b", r"C:\c"]);
        l.move_group(0, 3); // drives last
        assert_eq!(l.0[2], Group::Drives);
        let g = l.add_group("New".into());
        l.rename_group(g, "Later".into());
        l.remove_group(2); // the drives cannot go
        assert!(l.0.contains(&Group::Drives));
        assert!(to_text(&l).contains("[Later]"));
        assert_eq!(parse(&to_text(&l)), l);
    }
}
