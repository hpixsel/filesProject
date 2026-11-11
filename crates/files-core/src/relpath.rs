//! Validation of one untrusted relative path received from the remote machine.

use std::fmt;
use std::path::{Path, PathBuf};

/// A relative path whose every segment passed the rules in
/// "Untrusted clipboard data" (design doc).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RelPath(Vec<String>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadName {
    pub input: String,
    pub reason: &'static str,
}

const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM1", "COM2", "COM3", "COM4", "COM5",
    "COM6", "COM7", "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5",
    "LPT6", "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³",
];

impl RelPath {
    pub fn parse(s: &str) -> Result<RelPath, BadName> {
        let bad = |reason| BadName {
            input: s.to_string(),
            reason,
        };
        if s.is_empty() {
            return Err(bad("empty"));
        }
        if s.starts_with(['\\', '/']) {
            return Err(bad("absolute or UNC path"));
        }
        let segments: Vec<String> = s.split(['\\', '/']).map(String::from).collect();
        for seg in &segments {
            check_segment(seg).map_err(bad)?;
        }
        Ok(RelPath(segments))
    }

    pub fn to_path_under(&self, base: &Path) -> PathBuf {
        let mut p = base.to_path_buf();
        p.extend(&self.0);
        p
    }

    /// Proper ancestors, outermost first (`a\b\c` gives `a`, then `a\b`).
    pub fn ancestors(&self) -> impl Iterator<Item = RelPath> + '_ {
        (1..self.0.len()).map(|n| RelPath(self.0[..n].to_vec()))
    }

    /// Same path with `suffix` appended to the last segment (for `.partial`).
    pub fn with_suffix(&self, suffix: &str) -> RelPath {
        let mut segs = self.0.clone();
        if let Some(last) = segs.last_mut() {
            last.push_str(suffix);
        }
        RelPath(segs)
    }

    pub fn file_name(&self) -> &str {
        self.0.last().map_or("", String::as_str)
    }

    /// Case-insensitive identity used to find duplicates.
    // shortcut: per-char to_uppercase approximates the NTFS upcase table; switch to the volume's table if a real clash is missed
    pub fn key(&self) -> String {
        self.0
            .iter()
            .map(|s| s.to_uppercase())
            .collect::<Vec<_>>()
            .join("\\")
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join("\\"))
    }
}

fn check_segment(seg: &str) -> Result<(), &'static str> {
    if seg.is_empty() {
        return Err("empty segment");
    }
    if seg == "." || seg == ".." {
        return Err("dot segment");
    }
    if seg.contains(':') {
        return Err("colon (drive letter or alternate data stream)");
    }
    if seg.chars().any(|c| c < ' ' || "<>\"|?*".contains(c)) {
        return Err("invalid character");
    }
    if seg.ends_with([' ', '.']) {
        return Err("ends with space or dot");
    }
    if seg.encode_utf16().count() > 255 {
        return Err("segment longer than 255 UTF-16 units");
    }
    let stem = seg.split('.').next().unwrap_or("").trim_end_matches(' ');
    if RESERVED
        .iter()
        .any(|r| r.to_uppercase() == stem.to_uppercase())
    {
        return Err("reserved device name");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reason(s: &str) -> &'static str {
        RelPath::parse(s).unwrap_err().reason
    }

    #[test]
    fn accepts_normal_names() {
        for ok in ["a\\b.txt", "a/b.txt", "CONSOLE.txt", "a.b.c", "Résumé.pdf"] {
            assert!(RelPath::parse(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn rejects_each_rule() {
        assert_eq!(reason(""), "empty");
        assert_eq!(reason("\\\\server\\share"), "absolute or UNC path");
        assert_eq!(reason("/etc"), "absolute or UNC path");
        assert_eq!(reason("a\\\\b"), "empty segment");
        assert_eq!(reason("a\\..\\b"), "dot segment");
        assert_eq!(reason(".\\b"), "dot segment");
        assert_eq!(
            reason("C:x"),
            "colon (drive letter or alternate data stream)"
        );
        assert_eq!(
            reason("file.txt:stream"),
            "colon (drive letter or alternate data stream)"
        );
        assert_eq!(reason("a\u{1}b"), "invalid character");
        assert_eq!(reason("a?b"), "invalid character");
        assert_eq!(reason("name."), "ends with space or dot");
        assert_eq!(reason("name "), "ends with space or dot");
        assert_eq!(
            reason(&"x".repeat(256)),
            "segment longer than 255 UTF-16 units"
        );
        for r in ["CON.txt", "CON .txt", "conin$", "COM¹", "nul", "lpt9.log"] {
            assert_eq!(reason(r), "reserved device name", "{r}");
        }
    }

    #[test]
    fn helpers() {
        let p = RelPath::parse("a/b/c.txt").unwrap();
        assert_eq!(p.to_string(), "a\\b\\c.txt");
        let anc: Vec<String> = p.ancestors().map(|a| a.to_string()).collect();
        assert_eq!(anc, ["a", "a\\b"]);
        assert_eq!(p.with_suffix(".partial").to_string(), "a\\b\\c.txt.partial");
        assert_eq!(p.file_name(), "c.txt");
        assert_eq!(p.key(), RelPath::parse("A\\B\\C.TXT").unwrap().key());
        assert_eq!(
            p.to_path_under(Path::new("D:\\in")),
            Path::new("D:\\in\\a\\b\\c.txt")
        );
    }
}
