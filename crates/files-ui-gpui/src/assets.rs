//! Icons: ours (`assets/icons`: a few Lucide file and folder icons, ISC, and
//! the app logo) first, then gpui-kit's default set. Embedding Lucide's full
//! catalog instead would add 2.8 MB for a dozen icons. Material Icon
//! Theme's coloured file icons (`assets/material`, MIT) come with their own
//! lookup by name, for the setting that turns them on.

use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::OnceLock;

macro_rules! own {
    ($($name:literal),* $(,)?) => {
        &[$((
            concat!("icons/", $name, ".svg"),
            include_bytes!(concat!("../assets/icons/", $name, ".svg")) as &[u8],
        )),*]
    };
}

const OWN: &[(&str, &[u8])] = own!(
    "app",
    "monitor",
    "trash",
    "usb",
    "file-archive",
    "file-symlink",
    "folder-symlink",
    "file-code",
    "file-cog",
    "file-image",
    "file-key",
    "file-music",
    "file-play",
    "file-spreadsheet",
    "file-terminal",
    "file-text",
    "file-type",
    "folder-dot",
    "folder-down",
    "folder-git-2",
);

const MATERIAL: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/material.rs"));

/// map.txt: "file"/"folder" defaults, then `kind<TAB>key<TAB>icon` lines.
struct MaterialMap {
    file: &'static str,
    folder: &'static str,
    /// By extension, file name and folder name.
    ext: HashMap<&'static str, &'static str>,
    names: HashMap<&'static str, &'static str>,
    dirs: HashMap<&'static str, &'static str>,
}

fn material_map() -> &'static MaterialMap {
    static MAP: OnceLock<MaterialMap> = OnceLock::new();
    MAP.get_or_init(|| {
        let text = include_str!("../assets/material/map.txt");
        let mut map = MaterialMap {
            file: "file",
            folder: "folder",
            ext: HashMap::new(),
            names: HashMap::new(),
            dirs: HashMap::new(),
        };
        for line in text.lines() {
            match line.split('\t').collect::<Vec<_>>()[..] {
                ["file", icon] => map.file = icon,
                ["folder", icon] => map.folder = icon,
                ["e", key, icon] => _ = map.ext.insert(key, icon),
                ["n", key, icon] => _ = map.names.insert(key, icon),
                ["d", key, icon] => _ = map.dirs.insert(key, icon),
                _ => {}
            }
        }
        map
    })
}

/// The Material Icon Theme picture for a file or folder name, as VS Code
/// picks it: the whole name, then the longest known extension ("d.ts"
/// before "ts"), else the plain file or folder icon.
pub fn material_icon(name: &str, is_dir: bool) -> SharedString {
    let m = material_map();
    let name = name.to_lowercase();
    let icon = if is_dir {
        m.dirs.get(name.as_str()).copied().unwrap_or(m.folder)
    } else {
        m.names
            .get(name.as_str())
            .copied()
            .or_else(|| {
                name.match_indices('.')
                    .find_map(|(i, _)| m.ext.get(&name[i + 1..]).copied())
            })
            .unwrap_or(m.file)
    };
    format!("material/{icon}.svg").into()
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match OWN.iter().chain(MATERIAL).find(|(p, _)| *p == path) {
            Some((_, bytes)) => Ok(Some(Cow::Borrowed(bytes))),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        gpui_kit::assets::Assets.list(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_lookup() {
        let icon = |n, d| material_icon(n, d).to_string();
        assert_eq!(icon("main.PY", false), "material/python.svg");
        assert_eq!(icon("package.json", false), "material/nodejs.svg");
        assert_eq!(icon("x.d.ts", false), "material/typescript-def.svg");
        assert_eq!(icon("Src", true), "material/folder-src.svg");
        assert_eq!(icon("whatever.zzz", false), "material/file.svg");
        assert_eq!(icon("Pliki", true), "material/folder.svg");
        // Every icon the map names is embedded.
        for line in include_str!("../assets/material/map.txt").lines() {
            let name = format!("material/{}.svg", line.rsplit('\t').next().unwrap());
            assert!(MATERIAL.iter().any(|(p, _)| *p == name), "{name}");
        }
    }
}
