//! Plain data and rules shared by every crate. No Windows calls, no `unsafe`.

pub mod check;
pub mod machine;
pub mod relpath;

pub use check::{check, numbered, ConflictPolicy, Problem};
pub use relpath::{BadName, RelPath};

use std::time::SystemTime;

/// Largest single read, and the size of the one buffer per job.
pub const READ_MAX: usize = 1 << 20;
/// Buffer alignment and write granularity for unbuffered writes.
pub const ALIGN: usize = 4096;
/// Files above this size, or of unknown size, are written unbuffered.
pub const LARGE_FILE: u64 = 256 << 20;
/// Memory guard on the parsed clipboard list.
pub const MAX_ENTRIES: usize = 1_000_000;
/// Free space that must remain on the target volume.
pub const DISK_RESERVE: u64 = 1 << 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File { size: Option<u64> },
    Dir,
}

/// One item from the clipboard's file list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub rel_path: RelPath,
    pub kind: EntryKind,
    pub modified: Option<SystemTime>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    /// Counts `File` entries only; goes up after each final rename.
    pub files_done: u32,
    pub files_total: u32,
    pub bytes_done: u64,
    /// `None` if any size is unknown or the sum overflows.
    pub bytes_total: Option<u64>,
    pub current: Option<RelPath>,
}
