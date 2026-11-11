//! The only crate with `unsafe`, COM, OLE and Win32 calls. Exposes safe types.

pub mod clipboard;
pub mod drag;
pub mod drop;
#[cfg(feature = "fake")]
pub mod fake;
pub mod fileop;
pub mod instance;
pub mod io;
pub mod process;
pub mod shell;
pub mod sta;
pub mod watch;

pub use clipboard::{clipboard_object, formats, ContentReader, VirtualFiles};
pub use sta::Sta;

use files_core::BadName;
use std::fmt;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Win(windows::core::Error),
    Io(std::io::Error),
    NoVirtualFiles { formats: Vec<String> },
    BadDescriptor(&'static str),
    BadNames(Vec<BadName>),
    TargetExists(PathBuf),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Win(e) => write!(f, "Windows error: {e}"),
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::NoVirtualFiles { formats } => write!(
                f,
                "the clipboard holds no virtual files (formats found: {})",
                formats.join(", ")
            ),
            Error::BadDescriptor(why) => write!(f, "invalid clipboard file list: {why}"),
            Error::BadNames(names) => {
                write!(f, "invalid file names on the clipboard:")?;
                for n in names {
                    write!(f, " [{}: {}]", n.input, n.reason)?;
                }
                Ok(())
            }
            Error::TargetExists(p) => write!(f, "target already exists: {}", p.display()),
        }
    }
}

impl std::error::Error for Error {}

impl From<windows::core::Error> for Error {
    fn from(e: windows::core::Error) -> Self {
        Error::Win(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
