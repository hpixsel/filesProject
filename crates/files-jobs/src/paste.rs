//! The paste job: virtual files from the clipboard into a local folder.

use crate::{start, JobCtx, JobError, JobHandle, Limits};
use files_core::{
    check, numbered, ConflictPolicy, Entry, EntryKind, Progress, LARGE_FILE, READ_MAX,
};
use files_win::io::{
    background_mode, create_partial, free_bytes, rename_no_replace, set_delete_on_close,
    write_aligned, AlignedBuf,
};
use files_win::{Sta, VirtualFiles};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tracing::info;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub files_written: u32,
    /// Existing files left alone under `ConflictPolicy::Skip`.
    pub files_skipped: u32,
    /// Folders that did not exist before, including implied parents.
    pub folders_created: u32,
    pub bytes: u64,
}

pub fn paste(target: PathBuf, policy: ConflictPolicy) -> JobHandle<Summary> {
    paste_from(
        target,
        policy,
        Limits::default(),
        VirtualFiles::from_clipboard,
    )
}

/// `source` runs on the job thread, so the COM object never crosses threads.
pub fn paste_from(
    target: PathBuf,
    policy: ConflictPolicy,
    limits: Limits,
    source: impl FnOnce(&Sta) -> files_win::Result<VirtualFiles> + Send + 'static,
) -> JobHandle<Summary> {
    start("paste", limits, move |ctx| {
        run(&ctx, &target, policy, source)
    })
}

fn kind_of_path(p: &Path) -> Option<EntryKind> {
    std::fs::symlink_metadata(p).ok().map(|m| {
        if m.is_dir() {
            EntryKind::Dir
        } else {
            EntryKind::File {
                size: Some(m.len()),
            }
        }
    })
}

fn run(
    ctx: &JobCtx,
    target: &Path,
    policy: ConflictPolicy,
    source: impl FnOnce(&Sta) -> files_win::Result<VirtualFiles>,
) -> Result<Summary, JobError> {
    let sta = Sta::new()?;
    let _bg = background_mode()?;
    if !target.is_dir() {
        return Err(JobError::TargetNotDir(target.to_path_buf()));
    }
    let files = source(&sta)?;
    let entries = files.entries();
    let problems = check(entries, target, policy, kind_of_path, free_bytes(target)?);
    if !problems.is_empty() {
        return Err(JobError::Problems(problems));
    }

    let mut summary = Summary::default();
    let mut made = HashSet::new();
    for e in entries {
        let own = matches!(e.kind, EntryKind::Dir).then(|| e.rel_path.clone());
        for d in e.rel_path.ancestors().chain(own) {
            let p = d.to_path_under(target);
            if made.insert(d.key()) && !p.exists() {
                std::fs::create_dir(&p)?;
                summary.folders_created += 1;
            }
        }
    }

    let mut sizes = entries.iter().filter_map(|e| match e.kind {
        EntryKind::File { size } => Some(size),
        EntryKind::Dir => None,
    });
    let mut progress = Progress {
        files_total: sizes.clone().count() as u32,
        bytes_total: sizes.try_fold(0u64, |acc, s| acc.checked_add(s?)),
        ..Progress::default()
    };
    let mut buf = AlignedBuf::new(READ_MAX);
    for (i, e) in entries.iter().enumerate() {
        // `i` is the descriptor index (lindex), folders counted.
        let EntryKind::File { size } = e.kind else {
            continue;
        };
        ctx.check_cancel()?;
        progress.current = Some(e.rel_path.clone());
        if policy == ConflictPolicy::Skip && e.rel_path.to_path_under(target).exists() {
            progress.files_done += 1;
            progress.bytes_done += size.unwrap_or(0); // keep the percentage honest
            summary.files_skipped += 1;
            ctx.report_done(progress.clone());
            info!(file = %e.rel_path, "skipped, already exists");
            continue;
        }
        copy_one(
            ctx,
            &files,
            i,
            e,
            size,
            target,
            policy,
            &mut buf,
            &mut progress,
        )
        .map_err(|err| match err {
            JobError::Cancelled => JobError::Cancelled,
            other => JobError::Entry {
                path: e.rel_path.clone(),
                source: Box::new(other),
            },
        })?;
        progress.files_done += 1;
        summary.files_written += 1;
        ctx.report_done(progress.clone());
        info!(file = %e.rel_path, "done");
    }
    summary.bytes = progress.bytes_done;
    Ok(summary)
}

/// Renames to `path`, or to `name (2).ext`, `name (3).ext`… if taken. The
/// no-replace rename itself decides, so a file appearing meanwhile is safe.
fn rename_to_free_name(partial: &Path, path: &Path) -> files_win::Result<()> {
    let mut last = None;
    for n in 1..1000 {
        let candidate = if n == 1 {
            path.to_path_buf()
        } else {
            numbered(path, n)
        };
        match rename_no_replace(partial, &candidate) {
            Err(files_win::Error::TargetExists(p)) => last = Some(p),
            other => return other,
        }
    }
    Err(files_win::Error::TargetExists(
        last.unwrap_or_else(|| path.to_path_buf()),
    ))
}

#[allow(clippy::too_many_arguments)]
fn copy_one(
    ctx: &JobCtx,
    files: &VirtualFiles,
    index: usize,
    e: &Entry,
    size: Option<u64>,
    target: &Path,
    policy: ConflictPolicy,
    buf: &mut AlignedBuf,
    progress: &mut Progress,
) -> Result<(), JobError> {
    let path = e.rel_path.to_path_under(target);
    let partial = e.rel_path.with_suffix(".partial").to_path_under(target);
    let large = size.is_none_or(|s| s > LARGE_FILE);
    // From here on every early return drops `file` with delete-on-close set.
    let mut file = create_partial(&partial, large)?;
    let mut reader = files.open(index)?;
    let (mut filled, mut written) = (0usize, 0u64);
    loop {
        let n = reader.read(&mut buf[filled..READ_MAX])?;
        filled += n;
        written += n as u64;
        if size.is_some_and(|s| written > s) {
            return Err(JobError::SizeExceeded(e.rel_path.clone()));
        }
        let eof = n == 0;
        if large {
            // Only full buffers (multiples of ALIGN) or the final chunk are written.
            if eof || filled == READ_MAX {
                write_aligned(&mut file, buf, filled, eof)?;
                filled = 0;
            }
        } else if filled > 0 {
            file.write_all(&buf[..filled])?;
            filled = 0;
        }
        progress.bytes_done += n as u64;
        ctx.report(progress.clone());
        ctx.check_cancel()?;
        if eof {
            break;
        }
    }
    if let Some(declared) = size.filter(|&s| written < s) {
        return Err(JobError::SizeShort {
            path: e.rel_path.clone(),
            declared,
            got: written,
        });
    }
    if large {
        file.set_len(written)?;
    }
    if let Some(t) = e.modified {
        file.set_modified(t)?;
    }
    set_delete_on_close(&file, false)?;
    let renamed = match policy {
        ConflictPolicy::Fail | ConflictPolicy::Skip => rename_no_replace(&partial, &path),
        ConflictPolicy::Overwrite => std::fs::rename(&partial, &path).map_err(Into::into),
        ConflictPolicy::Rename => rename_to_free_name(&partial, &path),
    };
    if let Err(err) = renamed {
        set_delete_on_close(&file, true)?; // dropping `file` now deletes the .partial
        return Err(err.into());
    }
    Ok(())
}
