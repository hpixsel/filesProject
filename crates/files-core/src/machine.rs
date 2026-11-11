//! Line protocol between `rdppaste --machine` (child) and the GUI (parent).
//!
//! Child → parent on stdout, one event per line, fields separated by tabs:
//! *   `progress <files_done> <files_total> <bytes_done> <bytes_total|-> <current>`
//! *   `result <exit_code> <message>` (last line)
//!
//! Parent → child: closing stdin (or the parent dying) means cancel.
//! Tabs cannot occur in names (control characters are rejected) and are
//! replaced in messages.

use crate::{Progress, RelPath};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Progress(Progress),
    Result { code: i32, message: String },
}

pub fn progress_line(p: &Progress) -> String {
    format!(
        "progress\t{}\t{}\t{}\t{}\t{}",
        p.files_done,
        p.files_total,
        p.bytes_done,
        p.bytes_total.map_or("-".to_string(), |t| t.to_string()),
        p.current
            .as_ref()
            .map(|c| c.to_string())
            .unwrap_or_default()
    )
}

pub fn result_line(code: i32, message: &str) -> String {
    format!(
        "result\t{code}\t{}",
        message.replace(['\t', '\n', '\r'], " ")
    )
}

/// Parses one line; unknown or malformed lines give `None`.
pub fn parse_line(line: &str) -> Option<Event> {
    let mut f = line.trim_end_matches(['\r', '\n']).split('\t');
    match f.next()? {
        "progress" => {
            let files_done = f.next()?.parse().ok()?;
            let files_total = f.next()?.parse().ok()?;
            let bytes_done = f.next()?.parse().ok()?;
            let bytes_total = match f.next()? {
                "-" => None,
                t => Some(t.parse().ok()?),
            };
            let current = f
                .next()
                .filter(|c| !c.is_empty())
                .and_then(|c| RelPath::parse(c).ok());
            Some(Event::Progress(Progress {
                files_done,
                files_total,
                bytes_done,
                bytes_total,
                current,
            }))
        }
        "result" => Some(Event::Result {
            code: f.next()?.parse().ok()?,
            message: f.next().unwrap_or("").to_string(),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let p = Progress {
            files_done: 2,
            files_total: 5,
            bytes_done: 1024,
            bytes_total: None,
            current: Some(RelPath::parse("a\\b c.txt").unwrap()),
        };
        assert_eq!(parse_line(&progress_line(&p)), Some(Event::Progress(p)));
        let full = Progress {
            bytes_total: Some(9),
            ..Progress::default()
        };
        assert_eq!(
            parse_line(&progress_line(&full)),
            Some(Event::Progress(full))
        );
        assert_eq!(
            parse_line(&result_line(2, "cancelled;\t1/5 files")),
            Some(Event::Result {
                code: 2,
                message: "cancelled; 1/5 files".into()
            })
        );
        assert_eq!(parse_line("garbage"), None);
        assert_eq!(parse_line("progress\tx"), None);
    }
}
