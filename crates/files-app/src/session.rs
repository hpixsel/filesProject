//! The open tabs, saved on exit and reopened at start, in
//! `%APPDATA%\FilesProject\session.txt`: `pane <active tab>` lines, each
//! followed by its tabs' folders, and `active <pane>` for the active pane.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Session {
    /// Each pane's tabs (folders) and its active tab.
    pub panes: Vec<(Vec<PathBuf>, usize)>,
    pub active_pane: usize,
}

pub fn file() -> Option<PathBuf> {
    super::favourites::file().map(|f| f.with_file_name("session.txt"))
}

pub fn to_text(s: &Session) -> String {
    let mut text = format!("active {}\r\n", s.active_pane);
    for (tabs, active) in &s.panes {
        text += &format!("pane {active}\r\n");
        for t in tabs {
            text += &format!("{}\r\n", t.display());
        }
    }
    text
}

/// `None` when there is nothing to reopen. Indexes past the end are
/// clamped, so a hand-edited file cannot point nowhere.
pub fn parse(text: &str) -> Option<Session> {
    let mut s = Session::default();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(n) = line.strip_prefix("active ") {
            s.active_pane = n.parse().unwrap_or(0);
        } else if let Some(n) = line.strip_prefix("pane ") {
            s.panes.push((Vec::new(), n.parse().unwrap_or(0)));
        } else if let Some((tabs, _)) = s.panes.last_mut() {
            tabs.push(PathBuf::from(line));
        }
    }
    s.panes.retain(|(tabs, _)| !tabs.is_empty());
    s.panes.truncate(2); // the window has at most two panes
    for (tabs, active) in &mut s.panes {
        *active = (*active).min(tabs.len() - 1);
    }
    s.active_pane = s.active_pane.min(s.panes.len().checked_sub(1)?);
    Some(s)
}

pub fn load(file: &Path) -> Option<Session> {
    parse(&std::fs::read_to_string(file).ok()?)
}

pub fn save(file: &Path, s: &Session) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, to_text(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_bad_files() {
        let s = Session {
            panes: vec![
                (vec![r"C:\a".into(), r"D:\Zażółć".into()], 1),
                (vec![crate::places::RECYCLE_BIN.into()], 0),
            ],
            active_pane: 1,
        };
        assert_eq!(parse(&to_text(&s)), Some(s));
        assert_eq!(parse(""), None);
        let clamped = Session {
            panes: vec![(vec![r"C:\x".into()], 0)],
            active_pane: 0,
        };
        assert_eq!(parse("active 3\npane 9\nC:\\x\n"), Some(clamped));
    }
}
