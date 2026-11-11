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
    /// Recycle Bin items: the folder they were deleted from.
    pub origin: Option<PathBuf>,
    /// A `.lnk` shortcut's target (then `name` has no `.lnk`).
    pub link: Option<PathBuf>,
}

/// Lists `dir`: folders first, then files, each by name ignoring case.
/// Hidden and system entries (like `desktop.ini`) only with `show_hidden`.
/// Entries that cannot be read (permissions, races) are skipped.
pub fn list_dir(dir: &Path, show_hidden: bool) -> io::Result<Vec<Item>> {
    let mut items: Vec<Item> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            use std::os::windows::fs::MetadataExt;
            const HIDDEN_OR_SYSTEM: u32 = 0x2 | 0x4;
            if !show_hidden && meta.file_attributes() & HIDDEN_OR_SYSTEM != 0 {
                return None;
            }
            // Junctions such as "Documents and Settings" are reparse points
            // that std reports as symlinks, not dirs; the attribute says dir.
            let is_dir = meta.is_dir() || meta.file_attributes() & 0x10 != 0; // FILE_ATTRIBUTE_DIRECTORY
            Some(Item {
                name: e.file_name().to_string_lossy().into_owned(),
                path: e.path(),
                is_dir,
                size: (!is_dir).then_some(meta.len()),
                modified: meta.modified().ok(),
                origin: None,
                link: None,
            })
        })
        .collect();
    items.sort_by_cached_key(|i| (!i.is_dir, i.name.to_lowercase()));
    Ok(items)
}

/// What a listing is ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortBy {
    #[default]
    Name,
    Size,
    Modified,
    /// By extension (what the Type column shows).
    Type,
}

/// Orders `items` folders first (like Explorer), then by `by`; ties and
/// equal values fall back to the name. `size_of` gives folder sizes, which
/// a scan fills in later.
pub fn sort_items(
    items: &mut [Item],
    by: SortBy,
    descending: bool,
    size_of: impl Fn(&Item) -> Option<u64>,
) {
    items.sort_by_cached_key(|i| {
        let key = match by {
            SortBy::Name | SortBy::Type => 0,
            SortBy::Size => size_of(i).unwrap_or(0),
            SortBy::Modified => i
                .modified
                .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs()),
        };
        let ext = match by {
            SortBy::Type => i
                .path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase()),
            _ => None,
        };
        (!i.is_dir, key, ext, i.name.to_lowercase())
    });
    if descending {
        // Reverse within folders and within files, keeping folders first.
        let split = items.iter().position(|i| !i.is_dir).unwrap_or(items.len());
        items[..split].reverse();
        items[split..].reverse();
    }
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

    /// Up to the parent folder; from a drive root to This PC.
    pub fn up(&mut self) -> bool {
        let parent = match self.current.parent() {
            _ if places::is_place(&self.current) => return false,
            Some(p) => p.to_path_buf(),
            None => PathBuf::from(places::THIS_PC),
        };
        self.go(parent);
        true
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Where Back leads, nearest first.
    pub fn back_list(&self) -> impl Iterator<Item = &Path> {
        self.back.iter().rev().map(PathBuf::as_path)
    }

    /// Where Forward leads, nearest first.
    pub fn forward_list(&self) -> impl Iterator<Item = &Path> {
        self.forward.iter().rev().map(PathBuf::as_path)
    }

    /// Back (`n` > 0) or forward (`n` < 0) that many steps at once.
    pub fn step(&mut self, n: isize) {
        for _ in 0..n.unsigned_abs() {
            if !(if n > 0 { self.back() } else { self.forward() }) {
                break;
            }
        }
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
        std::fs::write(d.join("desktop.ini"), "").unwrap();
        let hidden = std::process::Command::new("attrib")
            .args(["+h", "+s"])
            .arg(d.join("desktop.ini"))
            .status()
            .unwrap();
        assert!(hidden.success());
        let items = list_dir(&d, false).unwrap();
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "zeta", "A.txt", "b.txt"]);
        assert_eq!(items[3].size, Some(5));
        assert_eq!(items[0].size, None);
        assert_eq!(
            list_dir(&d, true).unwrap().len(),
            5,
            "hidden shown when asked"
        );
        let _ = std::process::Command::new("attrib")
            .args(["-h", "-s"])
            .arg(d.join("desktop.ini"))
            .status();
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

        let mut j = History::new(PathBuf::from("a"));
        for d in ["b", "c", "d"] {
            j.go(PathBuf::from(d));
        }
        assert_eq!(
            j.back_list().collect::<Vec<_>>(),
            [Path::new("c"), Path::new("b"), Path::new("a")]
        );
        j.step(2);
        assert_eq!(j.current(), Path::new("b"));
        assert_eq!(j.forward_list().next(), Some(Path::new("c")));
        j.step(-2);
        assert_eq!(j.current(), Path::new("d"));

        let mut h = History::new(PathBuf::from(r"C:\"));
        assert!(h.up(), "a drive root goes up to This PC");
        assert_eq!(h.current(), Path::new(places::THIS_PC));
        assert!(!h.up(), "This PC is the top");
    }

    #[test]
    fn sorting_keeps_folders_first() {
        let item = |name: &str, is_dir: bool, size: u64| Item {
            name: name.into(),
            path: PathBuf::from(name),
            is_dir,
            size: Some(size),
            modified: None,
            origin: None,
            link: None,
        };
        let mut v = vec![
            item("b.txt", false, 1),
            item("Z", true, 9),
            item("a.txt", false, 5),
            item("y", true, 2),
        ];
        let names = |v: &[Item]| v.iter().map(|i| i.name.clone()).collect::<Vec<_>>();
        sort_items(&mut v, SortBy::Name, false, |i| i.size);
        assert_eq!(names(&v), ["y", "Z", "a.txt", "b.txt"]);
        sort_items(&mut v, SortBy::Size, true, |i| i.size);
        assert_eq!(names(&v), ["Z", "y", "a.txt", "b.txt"]);
        sort_items(&mut v, SortBy::Name, true, |i| i.size);
        assert_eq!(names(&v), ["Z", "y", "b.txt", "a.txt"]);
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(5 << 30), "5.0 GB");
    }
}

pub mod favourites;

/// What the window remembers between runs, in
/// `%APPDATA%\FilesProject\settings.txt` as `key=value` lines.
pub mod places;
pub mod replace_explorer;
pub mod session;
pub mod update;

pub mod settings {
    use std::path::{Path, PathBuf};

    #[derive(Debug, Clone, PartialEq)]
    pub struct Settings {
        /// (hue in degrees, saturation 0–1), or `None` to follow the
        /// Windows accent colour, which Windows can take from the wallpaper.
        pub accent: Option<(f32, f32)>,
        pub dark: bool,
        /// 0 normal, 1 compact, 2 very compact.
        pub density: usize,
        /// Base text size in px for each density; it also scales paddings.
        pub fonts: [f32; 3],
        /// `None` follows the Windows display language.
        pub polish: Option<bool>,
        /// Reopen the last tabs and panes at start.
        pub restore_tabs: bool,
        /// Show hidden and system files, like Explorer's option.
        pub show_hidden: bool,
        /// The list's columns after the name, in order: "size", "modified", "type".
        pub columns: Vec<String>,
        /// Column widths in px by column name, as last dragged.
        pub widths: Vec<(String, f32)>,
    }

    impl Default for Settings {
        fn default() -> Self {
            Settings {
                accent: Some((217., 0.85)),
                dark: true,
                density: 0,
                fonts: [16., 14., 13.],
                polish: None,
                restore_tabs: true,
                show_hidden: false,
                columns: vec!["size".into(), "modified".into(), "type".into()],
                widths: Vec::new(),
            }
        }
    }

    pub fn file() -> Option<PathBuf> {
        super::favourites::file().map(|f| f.with_file_name("settings.txt"))
    }

    /// Unknown keys and bad values keep their defaults.
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for (key, value) in text.lines().filter_map(|l| l.split_once('=')) {
            let value = value.trim();
            match key.trim() {
                "accent" if value == "wallpaper" => s.accent = None,
                "accent" => {
                    let mut n = value.split(' ').filter_map(|v| v.parse::<f32>().ok());
                    if let (Some(h), Some(sat)) = (n.next(), n.next()) {
                        s.accent = Some((h.rem_euclid(360.), sat.clamp(0., 1.)));
                    }
                }
                "dark" => s.dark = value != "false",
                "density" => s.density = value.parse::<usize>().unwrap_or(0).min(2),
                "lang" => s.polish = Some(value == "pl"),
                "restore_tabs" => s.restore_tabs = value != "false",
                "show_hidden" => s.show_hidden = value == "true",
                "columns" => {
                    s.columns = value
                        .split(',')
                        .map(str::trim)
                        .filter(|c| ["size", "modified", "type"].contains(c))
                        .map(String::from)
                        .collect()
                }
                "widths" => {
                    s.widths = value
                        .split(',')
                        .filter_map(|w| w.split_once(':'))
                        .filter_map(|(n, w)| {
                            Some((
                                n.trim().to_string(),
                                w.trim().parse::<f32>().ok()?.clamp(40., 2000.),
                            ))
                        })
                        .collect()
                }
                "fonts" => {
                    let n: Vec<f32> = value.split(' ').filter_map(|v| v.parse().ok()).collect();
                    if let [a, b, c] = n[..] {
                        s.fonts = [a, b, c].map(|f| f.clamp(10., 24.));
                    }
                }
                _ => {}
            }
        }
        s
    }

    pub fn to_text(s: &Settings) -> String {
        let accent = match s.accent {
            Some((h, sat)) => format!("{h} {sat}"),
            None => "wallpaper".into(),
        };
        let mut text = format!(
            "accent={accent}\r\ndark={}\r\ndensity={}\r\nrestore_tabs={}\r\n",
            s.dark, s.density, s.restore_tabs
        );
        text += &format!("show_hidden={}\r\n", s.show_hidden);
        text += &format!("columns={}\r\n", s.columns.join(","));
        let widths: Vec<String> = s.widths.iter().map(|(n, w)| format!("{n}:{w}")).collect();
        text += &format!("widths={}\r\n", widths.join(","));
        let [a, b, c] = s.fonts;
        text += &format!("fonts={a} {b} {c}\r\n");
        if let Some(pl) = s.polish {
            text += if pl { "lang=pl\r\n" } else { "lang=en\r\n" };
        }
        text
    }

    pub fn load(file: &Path) -> Settings {
        std::fs::read_to_string(file)
            .map(|t| parse(&t))
            .unwrap_or_default()
    }

    pub fn save(file: &Path, s: &Settings) -> std::io::Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(file, to_text(s))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn round_trip_and_bad_values() {
            let s = Settings {
                accent: Some((30.5, 0.4)),
                dark: false,
                density: 2,
                fonts: [18., 12.5, 11.],
                polish: Some(true),
                restore_tabs: false,
                show_hidden: true,
                columns: vec!["type".into(), "size".into()],
                widths: vec![("name".into(), 300.), ("type".into(), 120.5)],
            };
            assert_eq!(parse(&to_text(&s)), s);
            let w = Settings {
                accent: None,
                ..Settings::default()
            };
            assert_eq!(parse(&to_text(&w)), w);
            assert_eq!(
                parse("density=9\naccent=x\njunk"),
                Settings {
                    density: 2,
                    ..Settings::default()
                }
            );
        }
    }
}
