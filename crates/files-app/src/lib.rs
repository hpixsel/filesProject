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

/// Favourite folders, one path per line in a plain text file, so the list
/// can also be edited by hand.
pub mod favourites {
    use std::path::{Path, PathBuf};

    /// `%APPDATA%\FilesProject\favourites.txt`.
    pub fn file() -> Option<PathBuf> {
        std::env::var_os("APPDATA")
            .map(|d| PathBuf::from(d).join("FilesProject").join("favourites.txt"))
    }

    /// Desktop, Documents and Downloads, used until the file exists.
    pub fn defaults() -> Vec<PathBuf> {
        let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) else {
            return Vec::new();
        };
        ["Desktop", "Documents", "Downloads"]
            .iter()
            .map(|d| home.join(d))
            .filter(|p| p.is_dir())
            .collect()
    }

    pub fn load(file: &Path) -> Vec<PathBuf> {
        match std::fs::read_to_string(file) {
            Ok(text) => text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(PathBuf::from)
                .collect(),
            Err(_) => defaults(),
        }
    }

    pub fn save(file: &Path, list: &[PathBuf]) -> std::io::Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text: String = list
            .iter()
            .map(|p| format!("{}\r\n", p.display()))
            .collect();
        std::fs::write(file, text)
    }

    /// Adds `dir`, or removes it when already there. Returns whether it is now a favourite.
    pub fn toggle(list: &mut Vec<PathBuf>, dir: &Path) -> bool {
        match list.iter().position(|p| p == dir) {
            Some(i) => {
                list.remove(i);
                false
            }
            None => {
                list.push(dir.to_path_buf());
                true
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn round_trip_and_toggle() {
            let f = std::env::temp_dir()
                .join(format!("files-test-fav-{}", std::process::id()))
                .join("fav.txt");
            let mut list = vec![PathBuf::from(r"C:\Temp"), PathBuf::from(r"D:\Zażółć gęślą")];
            save(&f, &list).unwrap();
            assert_eq!(load(&f), list);
            assert!(!toggle(&mut list, Path::new(r"C:\Temp")));
            assert!(toggle(&mut list, Path::new(r"E:\new")));
            assert_eq!(
                list,
                [PathBuf::from(r"D:\Zażółć gęślą"), PathBuf::from(r"E:\new")]
            );
            let _ = std::fs::remove_dir_all(f.parent().unwrap());
        }
    }
}

/// What the window remembers between runs, in
/// `%APPDATA%\FilesProject\settings.txt` as `key=value` lines.
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
        /// `None` follows the Windows display language.
        pub polish: Option<bool>,
    }

    impl Default for Settings {
        fn default() -> Self {
            Settings {
                accent: Some((217., 0.85)),
                dark: true,
                density: 0,
                polish: None,
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
            "accent={accent}\r\ndark={}\r\ndensity={}\r\n",
            s.dark, s.density
        );
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
                polish: Some(true),
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
