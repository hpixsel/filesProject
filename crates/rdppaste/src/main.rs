//! `rdppaste`: paste files copied in an RDP session into a local folder.

use files_core::{ConflictPolicy, Problem, Progress};
use files_jobs::{paste, JobError, Outcome, STALL_TIMEOUT};
use files_win::{clipboard_object, formats, Error, Sta};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::info;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::prelude::*;

const USAGE: &str = "\
Paste files copied (Ctrl+C) in an RDP session into a local folder.

Usage:
  rdppaste <target-dir> [--overwrite]
  rdppaste --formats     list the formats currently on the clipboard
  rdppaste --help

Exit codes: 0 done, 1 failed, 2 cancelled, 3 stalled, 4 no virtual files, 64 usage.";

const PRINT_EVERY: Duration = Duration::from_millis(250);
// shortcut: fixed width; read the console width if narrow terminals still wrap
const LINE_WIDTH: usize = 79;
const MIB: f64 = (1u64 << 20) as f64;

fn main() {
    init_logging();
    // process::exit so a job thread blocked in COM cannot keep the process alive.
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args[..] {
        ["--help"] | ["-h"] => {
            println!("{USAGE}");
            0
        }
        ["--formats"] => list_formats(),
        [dir] if !dir.starts_with('-') => paste_into(dir, ConflictPolicy::Fail),
        [dir, "--overwrite"] | ["--overwrite", dir] => paste_into(dir, ConflictPolicy::Overwrite),
        _ => {
            eprintln!("{USAGE}");
            64
        }
    }
}

fn init_logging() {
    let file = std::env::var_os("LOCALAPPDATA")
        .map(|d| PathBuf::from(d).join("FilesProject").join("logs"))
        .and_then(|dir| {
            std::fs::create_dir_all(&dir).ok()?;
            let f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("rdppaste.log"));
            f.ok()
        });
    let opened = file.is_some();
    let file_layer = file.map(|f| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(Mutex::new(f))
            .with_filter(LevelFilter::INFO)
    });
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(LevelFilter::WARN);
    tracing_subscriber::registry()
        .with(file_layer)
        .with(stderr_layer)
        .init();
    if !opened {
        tracing::warn!("cannot open the log file; logging to the terminal only");
    }
}

fn list_formats() -> i32 {
    let result = Sta::new().and_then(|sta| formats(&clipboard_object(&sta)?));
    match result {
        Ok(names) => {
            for n in names {
                println!("{n}");
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn paste_into(dir: &str, policy: ConflictPolicy) -> i32 {
    info!(target = dir, ?policy, "paste started");
    let job = paste(PathBuf::from(dir), policy);
    let token = job.cancel_token();
    if let Err(e) = ctrlc::set_handler(move || token.cancel()) {
        tracing::warn!("Ctrl+C will not cancel cleanly: {e}");
    }

    let mut last = Progress::default();
    let mut printed: Option<Instant> = None;
    let outcome = job.wait(|p| {
        last = p.clone();
        if printed.is_none_or(|t| t.elapsed() >= PRINT_EVERY) {
            print_progress(p);
            printed = Some(Instant::now());
        }
    });
    if printed.is_some() {
        print_progress(&last);
        println!();
    }
    let completed = format!("{}/{} files completed", last.files_done, last.files_total);

    let (code, message) = match outcome {
        Outcome::Done(s) => {
            let text = format!(
                "Done: {} files, {} new folders, {}",
                s.files_written,
                s.folders_created,
                size(s.bytes)
            );
            println!("{text}");
            (0, text)
        }
        Outcome::Failed(JobError::Win(Error::NoVirtualFiles { formats })) => {
            let text = format!(
                "The clipboard holds no files from an RDP session. Formats found: {}",
                formats.join(", ")
            );
            eprintln!("{text}");
            (4, text)
        }
        Outcome::Failed(JobError::Problems(problems)) => {
            eprintln!("Nothing was written. Problems found:");
            for p in &problems {
                eprintln!("  {}", describe(p));
            }
            (1, format!("{} problem(s) before writing", problems.len()))
        }
        Outcome::Failed(e) => {
            eprintln!("Failed: {e}\n{completed}");
            (1, format!("failed: {e}; {completed}"))
        }
        Outcome::Cancelled => {
            eprintln!("Cancelled. {completed}");
            (2, format!("cancelled; {completed}"))
        }
        Outcome::Stalled => {
            let text = format!(
                "Stalled: no progress for {} s (RDP session lost?). {completed}",
                STALL_TIMEOUT.as_secs()
            );
            eprintln!("{text}");
            (3, text)
        }
    };
    info!(code, %message, "paste finished");
    code
}

/// KB below 1 MiB, so small pastes don't read as "0.0 MiB".
fn size(bytes: u64) -> String {
    if (bytes as f64) < MIB {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / MIB)
    }
}

fn print_progress(p: &Progress) {
    let pct = p
        .bytes_total
        .filter(|&t| t > 0)
        .map(|t| format!(" ({:.0}%)", p.bytes_done as f64 * 100.0 / t as f64))
        .unwrap_or_default();
    let head = format!(
        "{}/{} files, {}{pct}  ",
        p.files_done,
        p.files_total,
        size(p.bytes_done)
    );
    let name = p
        .current
        .as_ref()
        .map(|c| c.to_string())
        .unwrap_or_default();
    // Keep the line short enough not to wrap, or `\r` cannot rewrite it.
    let room = LINE_WIDTH.saturating_sub(head.chars().count());
    let chars: Vec<char> = name.chars().collect();
    let name: String = if chars.len() > room {
        let tail = &chars[chars.len() + 1 - room.max(1)..];
        format!("…{}", tail.iter().collect::<String>())
    } else {
        name
    };
    // `\x1b[2K` clears the previous, possibly longer, line.
    print!("\r\x1b[2K{head}{name}");
    let _ = std::io::stdout().flush();
}

fn describe(p: &Problem) -> String {
    match p {
        Problem::Duplicate(name) => format!("duplicate name in the copied list: {name}"),
        Problem::PartialNameTooLong(name) => format!("name too long: {name}"),
        Problem::PartialExists(path) => {
            format!("leftover temporary file exists: {}", path.display())
        }
        Problem::Conflict {
            path,
            overwritable: true,
        } => {
            format!("already exists (use --overwrite): {}", path.display())
        }
        Problem::Conflict {
            path,
            overwritable: false,
        } => {
            format!(
                "a file/folder of the other kind is in the way: {}",
                path.display()
            )
        }
        Problem::NotEnoughSpace { need, free } => format!(
            "not enough disk space: need {:.1} GiB, free {:.1} GiB",
            *need as f64 / (MIB * 1024.0),
            *free as f64 / (MIB * 1024.0)
        ),
    }
}
