# FilesProject: architecture and milestone M0 (RDP paste prototype)

*   **Author:** hpixsel (Kacper Adamus)
*   **Status:** Reviewed (EGM stages 5–9, 2026-11-11). Ready for implementation of M0.
*   **Date:** 2026-11-11
*   **Predecessor:** [`docs/designs/files_project.md`](files_project.md) (brainstorm brief, Polish)

--------------------------------------------------------------------------------

## Overview

FilesProject is a file explorer for Windows, written in Rust, built first for
one user (the author) and published as a public MIT-licensed repository. It
replaces FilePilot, a third-party file manager for Windows, in the author's
daily work.

This document covers two scopes:

*   **Whole product.** This covers every section above the Detailed
    Implementation Plan: the problem, goals, architecture, alternatives,
    and risks that milestones M0–M6 build on. Each CUJ is tagged with the
    milestone that delivers it.
*   **Milestone M0 and the repository skeleton** (Detailed Implementation
    Plan):
    *   a command-line prototype that pastes files from a Remote Desktop
        (RDP) clipboard;
    *   the workspace, CI, and documentation layout.

    Later milestones get their own design documents. Their details depend
    on what M0 learns (real RDP behaviour, the freeze baseline) and on the
    UI toolkit chosen after M0.

**Proposed solution:**

*   A Rust explorer whose file operations all run outside the UI thread,
    through one job queue with fixed limits on parallel work and memory.
*   It reads virtual files from the clipboard.
*   The first deliverable proves the RDP paste works before any UI is
    built.

## The Problem

The author uses FilePilot at work and hits two problems every day:

1.  **The whole PC freezes during file operations.** Disk and RAM use climb
    until the machine stops responding. The cause has not been measured.
    *   The likely candidates are:
        *   the Windows file cache filling with data from large copies;
        *   unbounded parallel I/O;
        *   antivirus scanning.
    *   The new program avoids the first two by design. The M0 baseline
        measurement (Q1) shows whether antivirus contributes.
2.  **Files copied inside an RDP session cannot be pasted.** The author
    presses Ctrl+C on files in Windows Explorer on a remote server, then
    Ctrl+V on the local machine. Windows Explorer accepts this paste;
    FilePilot does not.

**Why the paste fails in FilePilot and in any explorer that reads only
`CF_HDROP`:**

*   When files are copied in an RDP session, the local RDP client
    (`mstsc.exe`, or the Windows App / `msrdc`) puts them on the local
    clipboard as *virtual files*, not as paths.
*   The clipboard holds a list of file descriptors
    (`CFSTR_FILEDESCRIPTORW`).
*   It serves each file's bytes on request (`CFSTR_FILECONTENTS`) through
    a local COM `IDataObject`. The RDP client fetches those bytes from the
    server over the RDP connection.
*   A program that reads only file paths from the clipboard (`CF_HDROP`)
    sees nothing to paste.

## Goals

Each goal has a pass condition. The numbers are first estimates; the M0
baseline measurement (Q1) may adjust them.

*   **G1 — RDP paste.** Files and folders copied in Windows Explorer inside
    an RDP session paste into a local folder. In M0 this is done with the
    `rdppaste` command; from M1 with Ctrl+V.
    *   Folder structure is recreated.
    *   A file's last-write time is kept when the descriptor provides it.
    *   Progress is shown. When the size is unknown, the bytes copied so far
        are shown instead.
    *   Cancel stops the transfer within 1 second, even when a network read
        is blocked, and removes the unfinished `.partial` file.
    *   The **stall timeout** is 60 seconds with no bytes received. A
        transfer that receives no bytes for that long ends on its own as
        "stalled", without user action, and removes its `.partial` file.
    *   A single file larger than 4 GiB (2^32 bytes) transfers intact,
        checked by SHA-256.
*   **G2 — No system freeze.** Measured with Performance Monitor during a
    10–20 GB copy, against values taken just before the copy.
    *   The program's RAM stays below 150 MB.
    *   `Memory\Modified Page List Bytes` rises by at most 512 MB.
    *   The sum of the `Memory\Standby Cache *` counters rises by at most
        512 MB.
    *   During the copy the author types in Notepad and switches windows.
        No pause longer than 1 second is visible.
    *   From M1, the UI thread never blocks for more than 50 ms, as recorded
        by a message-loop latency log.
*   **G3 — Bounded background work (M2).**
    *   Folder size calculation on a tree with 100,000 files runs in
        background mode.
    *   It holds only running totals, not the tree, in memory.
    *   It stops within 200 ms of cancel.
*   **G4 — Maintainable code base.**
    *   `unsafe` code and Windows API calls live in one crate only.
    *   Every milestone ends with green CI (format, lint, tests).
    *   Every milestone also ends with an updated `docs/HANDOFF.md`, so
        work can resume from the repository contents alone.
*   **G5 — Own daily explorer.** From M3 on, the author uses FilesProject
    instead of FilePilot for a full work week.
    *   This is the reason the project builds an explorer under the
        author's own control, with the features listed in `CLAUDE.md`,
        rather than working around FilePilot's two problems.

## Non-Goals

*   **RDP copy in the other direction** (local machine → remote server). It
    requires the program to publish its own virtual files on the clipboard.
    Revisited after M2.
*   **Running FilesProject on the remote server.** Only the local machine runs
    it.
*   **Operating systems other than Windows 10/11 x64.** Linux, macOS, and
    ARM64 are excluded until revisited after M6.
*   **Shell virtual folders:** This PC, Recycle Bin, Control Panel, MTP
    phones, browsing inside zip files. These are handed to Windows Explorer.
*   **Replacing Open/Save file dialogs, the Desktop, or the taskbar.** Explorer
    replacement (M6) only covers opening folders.
*   **A custom copy engine for local files.** Local copy, move, and delete use
    Windows APIs (`CopyFileExW` or `IFileOperation`). Undo and Recycle Bin come
    from Windows.
*   **Keeping file attributes** (read-only, hidden) on pasted files in M0.
*   **Dragging files out of the program** before M6.
*   **Any UI in M0.**

## Open Questions

*   **Q1 — Freeze baseline.** Does Windows Explorer also freeze the PC on the
    same copy? Is the freeze local-disk-only, network-only, or both? Is
    antivirus involved? This does not block M0, but M0 is not done until it
    is measured. See risk R2.
*   **Q2 — RDP client and host policy.** Which client must work (`mstsc` or
    Windows App / `msrdc`)? Which Windows versions run on the remote servers?
    Does Group Policy on those servers allow file transfer through the
    clipboard? See risk R1.
*   **Q4 — Test rig.** Does loopback RDP to `localhost` work on the author's
    Windows 11 Pro machine? If not, use a Hyper-V VM.
*   **Q5 — Name conflicts.** When a pasted file already exists: overwrite,
    skip, rename, or ask, with an "apply to all" option?
    *   M0 supports only "fail" (the default) and "overwrite".
    *   All conflicts are checked before anything is written.
    *   The full answer is due in M1.
*   **Q6 — Session loss and clipboard change during paste.** Partly
    answered for M0:
    *   The job takes the clipboard object once, at the start.
    *   Three events can break the source during a paste:
        *   the session drops;
        *   the remote clipboard changes;
        *   the local clipboard is replaced, for example by Ctrl+C with text
            selected in a terminal.
    *   After any of them, the next read either fails, which ends the job
        as "failed", or hangs, which ends it as "stalled" after the stall
        timeout.
    *   Either way, finished files stay and the unfinished `.partial` file
        is removed.
    *   How `mstsc` behaves in each case is checked in the manual RDP run.
    *   Still open for M1: show which files completed and offer a retry
        that skips them; warn when the clipboard changes during a paste.
*   **Q7 — Code signing.** Accept SmartScreen warnings, or set up signing
    (SignPath Foundation or Azure Trusted Signing)? This affects the updater
    in M6. See risk R8.
*   **Q8 — Paste as a child process (M1).** The direction is set: the
    GUI runs each paste as a child `rdppaste` process (Alternative L). The
    M1 design must answer these before building it:
    *   **Cancel channel.** How does the GUI ask the child to cancel? A
        GUI without a console cannot send a console Ctrl+C. Candidate:
        the child treats end of stdin as cancel, which also covers a GUI
        crash.
    *   **Orphaned child.** How is the child stopped if the GUI dies?
        Candidate: a Windows job object that kills it with the GUI.
    *   **Progress protocol.** What format does the child use on stdout?
    *   **Queued pastes.** A queued paste reads the clipboard only when
        its child starts. If the author copies something else meanwhile,
        it pastes the new content. Candidate: the GUI reads the clipboard
        when Ctrl+V is pressed and refuses a second paste while one runs.

    M0 builds none of this.

### Resolved

*   **Q3 — Paste mechanism.** The program's own stream reader is the only
    engine. See "Why not the Windows drop target".
*   **Custom command in FilePilot** (from the brief). This is closed by G5:
    the project builds its own explorer.

## Critical User Journeys (CUJs)

### CUJ 1: Paste one large file from an RDP session (M0)

*   **Given**: The author is connected to a remote server with `mstsc` and
    clipboard redirection on. A 6 GB file exists on the server.
*   **When**: The author selects the file in Explorer on the server, presses
    Ctrl+C, and runs `rdppaste D:\incoming` locally.
*   **Then**:
    *   The file appears in `D:\incoming` with the same SHA-256 as on the
        server.
    *   The terminal shows progress.
    *   The G2 limits hold.
    *   No `.partial` file remains.

### CUJ 2: Paste a folder tree and cancel midway (M0)

*   **Given**: Same session. The author copies a folder with 2,000 files in
    nested subfolders.
*   **When**: The author runs `rdppaste D:\incoming` and presses Ctrl+C in the
    local terminal halfway through, with no text selected. Ctrl+C then sends
    a cancel signal and does not touch the clipboard.
*   **Then**:
    *   The transfer stops within 1 second.
    *   Fully copied files remain, and the unfinished `.partial` file is
        deleted.
    *   The exit code reports "cancelled". The terminal output and the log
        state how many files completed.

### CUJ 3: Clipboard has no virtual files (M0)

*   **Given**: The clipboard holds text, or local file paths.
*   **When**: The author runs `rdppaste D:\incoming`.
*   **Then**:
    *   The program exits with a message stating that the clipboard holds
        no virtual files.
    *   The message lists the clipboard formats it found.
    *   `rdppaste --formats` prints the same list.

### CUJ 4: Paste with Ctrl+V in the explorer window (M1)

*   **Given**: FilesProject is open on a local folder. Files are copied in an
    RDP session.
*   **When**: The author presses Ctrl+V. While that paste is running, the
    author presses Ctrl+V again in another folder.
*   **Then**:
    *   The first paste runs in the background with a progress indicator.
    *   The second paste waits in a visible queue and starts when the first
        ends.
    *   The file list stays scrollable, with no UI-thread block over 50 ms.
    *   New files appear in the list when they finish.

### CUJ 5: Folder sizes without freezing (M2)

*   **Given**: FilesProject shows a folder whose subfolders hold 100,000 files
    in total.
*   **When**: The author asks for folder sizes, then navigates away.
*   **Then**:
    *   Each folder's size updates as its subtree is scanned.
    *   Navigating away cancels the calculation within 200 ms.
    *   The G2 responsiveness check holds.

### CUJ 6: The RDP session drops during a paste (M0)

*   **Given**: A paste of several large files is running.
*   **When**: The RDP connection is cut (network unplugged or session
    disconnected), and the author does nothing.
*   **Then**:
    *   Within the stall timeout (60 s) plus 1 s, `rdppaste` exits with
        "stalled".
    *   Finished files remain, and no `.partial` file remains.

## Technical Plan (Architecture)

This section describes components, responsibilities, and data flow. The
Detailed Implementation Plan adds signatures, file lists, and exact values.
It must not contradict this section; any disagreement is a defect to fix in
both places.

### Decisions taken in this design

| Decision | Choice | Decided by | Alternatives |
| :-- | :-- | :-- | :-- |
| Paste engine | Own stream reader only. No drop-target code; Ctrl+V in Windows Explorer serves as the diagnostic | Owner, revised in design review (2026-11-11) | I |
| Running a paste from the GUI (M1+) | Direction only: child `rdppaste` process. Details are left to the M1 design (Q8) | Owner, during review (2026-11-11) | L |
| Removing unfinished `.partial` files | Delete-on-close flag on the file handle, cleared just before the final rename | Owner, during review (2026-11-11) | n/a |
| Writing large files | Unbuffered writes (`FILE_FLAG_NO_BUFFERING`) | Owner, during review (2026-11-11) | K |
| Code layout | Cargo workspace with four crates from day one | Owner | E |
| Concurrency model | Plain OS threads and bounded channels; no async runtime | Design author; owner agreed | F |
| Crate names | Prefixed `files-*`, because a crate named `core` would clash with Rust's built-in `core` | Design author (mechanical) | n/a |
| Long paths in M0 | No application manifest; `std::fs` long-path handling. M2 decides again for `CopyFileExW`, `IFileOperation`, and `FindFirstFileExW` | Owner | J |

### Crates and dependency direction

```text
            rdppaste (bin, M0)          files-app + files-ui-<toolkit> (M1+)
              |       |                              |
              |       +-------------+----------------+   (M1+: GUI starts rdppaste
              |                     v                     as a child process for pastes)
              |                files-jobs   ---->  files-core
              |                     |                  ^
              |                     v                  |
              +--diagnostics--> files-win  ------------+
                (--formats)
```

Arrows mean "depends on". `files-core` depends on nothing in the workspace.

*   **`files-core`: rules and plain data.**
    *   Validates the clipboard file list received from the remote machine.
    *   Checks targets for conflicts.
    *   Describes progress.
    *   It has no Windows calls and no `unsafe`, so it can be tested on any
        OS.
*   **`files-win`: everything that touches Windows.**
    *   It handles the COM apartment and reads the clipboard's virtual-file
        list and file contents.
    *   It provides background mode, unbuffered file writing, the
        delete-on-close flag, and the no-replace rename.
    *   It is the only crate allowed to use `unsafe`, and it exposes only
        safe Rust types.
    *   It reads virtual files from any `IDataObject`, so tests can pass a
        fake object directly. A separate function gets the `IDataObject`
        from the clipboard and passes it on.
*   **`files-jobs`: running work away from the caller's thread.**
    *   In M0 it holds a cancel flag, a bounded progress channel, one worker
        thread per job, and a stall watchdog. It also holds the paste job.
    *   In M2 it gains a general queue with concurrency limits, when a
        second kind of job (folder sizes) exists.
*   **`rdppaste`: command-line front end.**
    *   It turns arguments into a paste job and terminal Ctrl+C into cancel.
    *   It prints progress, writes a log file, and returns an exit code.
    *   From M1 it is also intended as the worker process that the GUI
        starts for each paste (Q8).

### Threading and process rules

*   A COM object stays on the thread that obtained it. The paste job thread
    initialises OLE and fetches the clipboard itself, so no object crosses
    threads.
*   While a call into the RDP client is in progress, COM pumps messages on
    that thread. The job thread therefore never holds a lock that another
    thread needs.
*   Job threads run in **background mode**
    (`THREAD_MODE_BACKGROUND_BEGIN`), which gives them low I/O and low
    memory priority. This covers the reads and writes the job thread makes
    itself. Because large files are written unbuffered (see "Writing large
    files"), those writes are made by the job thread, not later by Windows'
    cache writer.
*   **A blocked network read cannot be interrupted safely.** Two events end
    the wait for it:
    *   the user cancels: the caller waits at most 1 second;
    *   no bytes arrive for the stall timeout: the job is declared stalled.

    In both cases the caller stops waiting and the process exits.
    *   Process exit ends the blocked COM call.
    *   M0 is a command-line program that exits after one paste, so an
        abandoned thread never outlives the process.
*   **Unfinished files clean themselves up.**
    *   Right after a `.partial` file is created, its handle is marked
        delete-on-close.
    *   The mark is cleared only just before the final rename.
    *   Windows therefore deletes the file on every other exit path: error,
        cancel, stall, process exit, and process kill.
    *   No other code deletes `.partial` files, so the program can never
        delete a `.partial` that it did not create.
*   **From M1, the GUI does not run pastes in its own process.** The
    direction is a child `rdppaste` process that the GUI can kill. Killing
    it releases the COM objects, the file handles, and the blocked thread
    together. Details are in Q8.

### Paste data flow

```text
main thread (CLI)                   job thread (OLE initialised, background mode)
-----------------                   -----------------------------------------------
parse args                          get IDataObject from clipboard
start job  ---------------------->  read and validate the whole file list
                                    check targets, temp names, duplicates, free space
                                      -> any failure: stop, report all, nothing written
                                    for each entry (parents created first):
print progress  <--- progress ----     folder -> create it
Ctrl+C -> cancel  --------------->     file   -> create "<name>.partial" (create-new, delete-on-close)
                                                 read in reads of at most 1 MiB; write
                                                 stop if more bytes than declared
                                                 check cancel between reads
                                                 clear delete-on-close; rename to final name (no-replace in "fail" mode)
                                    on cancel or error: drop the handle -> Windows deletes ".partial"
wait / watchdog <---- result -----  (cancel + 1 s, or stall: process exits -> Windows deletes ".partial")
exit code
```

*   **Checks before writing.** The whole list is checked before the first
    byte is written, and all failures are collected and reported together.
    The checks cover:
    *   names;
    *   conflicts with existing files;
    *   `.partial` names;
    *   duplicates inside the list;
    *   free disk space.

    The paste does not stop partway because of a problem that existed when
    it started. A file that appears in the target folder after the check is
    still not overwritten in "fail" mode, because the final rename refuses
    to replace it.
*   **Read size and the stall timeout.** Each read asks for at most 1 MiB.
    The stall timeout must stay above the time one read takes at the
    slowest supported speed. Here that is 1 MiB at 64 KiB/s, which takes
    16 s, well under 60 s. If the read size or the timeout changes, this
    rule must still hold.

### Writing large files

Normal (buffered) writes put data into the Windows file cache first.
*   The data waits there as Modified pages until Windows writes it to disk.
    It then stays as Standby pages.
*   A 10–20 GB copy fills both lists. Windows reuses Standby pages when
    other programs need memory, but the copy has by then pushed those
    programs' own cached data out. Afterwards they read it from disk again
    and feel slow.

`CLAUDE.md` requires that large files do not clog the Windows cache.

*   **Large files** are therefore written with `FILE_FLAG_NO_BUFFERING`,
    which bypasses the cache. A file counts as large when it is above a
    fixed size (the value is in the Detailed Implementation Plan), or when
    its size is unknown, because then it could be any size. The alignment
    rules this needs are in item 22 of the Detailed Implementation Plan.
*   **Small files** are written normally. They cannot fill the cache.

### Why not the Windows drop target

When the folder's drop target does the paste, Windows' own copy engine
controls progress, cancel, conflict prompts, and buffering. That breaks the
`CLAUDE.md` hard requirement (every operation runs through the program's own
job queue with progress and cancellation) and CUJ 4 (progress inside the
window). It also gives the program no way to meet G2 if Windows' engine is
itself the cause of the freezes.

M0 has no job queue either, because the queue arrives in M1 for pastes and
in M2 for other jobs. The difference is what can change later.
*   The own engine already runs as a job the program controls, with
    progress and cancel, and is designed to run under the queue.
*   The drop target can never be put under the queue's control.

M0 meets the progress and cancel parts of the requirement now. The
concurrency limit arrives with the queue.

**Diagnosis without extra code.** When `rdppaste` fails, press Ctrl+V in
Windows Explorer into the same folder. Explorer uses the drop target, so
this separates two kinds of failure:

*   **Explorer works:** the data on the clipboard is fine, and the defect
    is in `rdppaste`.
*   **Explorer fails too:** the likely cause is the RDP client or the
    server policy (Q2), or a permission problem on the target folder.

`rdppaste --formats` shows which formats the clipboard holds.

### Untrusted clipboard data

The clipboard file list and the file contents come from the remote
machine. The checks below run before anything is written, unless marked
otherwise.

**Names.** A name is rejected if it has any of these:

*   empty, `.` and `..` segments;
*   absolute paths, drive letters, and UNC prefixes;
*   `:` anywhere, which blocks alternate data streams;
*   control characters and `<>"|?*`;
*   Windows reserved device names: `CON`, `PRN`, `AUX`, `NUL`, `CONIN$`,
    `CONOUT$`, `COM1`–`COM9`, `COM¹`–`COM³`, `LPT1`–`LPT9`, `LPT¹`–`LPT³`,
    with or without an extension;
*   segments ending in a space or a dot (and reserved names are checked
    after trimming trailing spaces and dots, so `CON .txt` counts as
    `CON`);
*   invalid UTF-16 (for example an unpaired surrogate). Names are never
    converted lossily;
*   a name with no terminating NUL inside the descriptor's 260-unit field
    (risk R3);
*   a segment longer than 255 UTF-16 units, counted with the `.partial`
    suffix for files.

**The list as a whole.**

*   **Declared count.** The count of entries stated in the descriptor
    block must fit inside the block's actual size. Otherwise the whole list
    is rejected before any entry is read.
*   **Memory guard.** The list may hold at most 1,000,000 entries. This
    caps the memory used by the parsed list; it is not a limit on real
    trees.
*   **Duplicates.** No two entries may map to the same target path.
    *   The comparison ignores case.
    *   Every folder implied by a path counts as a folder entry, even when
        the list does not name it.
    *   Two paths clash only when at least one of them is a file. Folders
        with the same name, listed or implied, merge. So a file `x` next to
        `x\y` is a clash, while `x\a` next to `x\b` is not.
    *   The `.partial` name of each file counts as a path too. A list with
        both `a` and `a.partial` is therefore a clash.
*   **Conflicts with the target folder.** Implied folders are also
    checked against what already exists there. A paste of `x\y` into a
    folder that already contains a file `x` is rejected before writing.
*   The parent folder of every file is created before the file, whatever
    order the list arrives in.
*   The descriptor format stores each path in a fixed 260-character field.
    Deeper trees cannot be represented and fail with a clear error (risk
    R3).

**Sizes and disk space.**

*   A file must match its declared size exactly. Extra bytes, and a stream
    that ends early, are both errors, and the `.partial` file is removed.
*   Before writing, free space must be at least the sum of declared sizes
    plus 1 GiB. The sum is computed with overflow checks, and an overflow
    counts as "not enough space".
*   When a size is unknown and the disk fills up during the copy, Windows
    returns "disk full", the paste fails, and the `.partial` file is
    removed.

**Entries.**

*   Folders are recognised only by the directory attribute, and only when
    the attributes field is marked as present.
*   An entry without the attributes field is treated as a file. If its
    contents cannot be opened, the paste ends with an error naming the
    entry.
*   When the size field is missing, the size is unknown, and progress
    shows bytes copied without a percentage.

### Testing without RDP

*   **`files-core`**: plain unit tests.
*   **`files-win` and `files-jobs`**: a test-only fake `IDataObject` serves a
    configurable file list and contents, and is passed to the code directly.
    It can simulate:
    *   both delivery forms (stream or memory block);
    *   a missing or a wrong size;
    *   a size over 4 GiB;
    *   slow reads, failing reads, and reads that hang.
*   **Clipboard smoke test**: one test puts the fake on the real system
    clipboard and reads it back. It lives in its own test binary, so no
    other test has to run single-threaded. It is unverified whether the CI
    runner has a usable clipboard.
*   **Manual RDP test**: described in `docs/testing-rdp.md`. It covers:
    *   CUJ 1, 2, 3 and 6;
    *   the Q6 cases;
    *   loopback to `localhost` if Windows allows it, otherwise a Hyper-V
        Windows 11 evaluation VM;
    *   one run against a real work server.

    The fake runs in the same process and cannot reproduce cross-process
    COM behaviour, so the manual test is required (risk R7).

### Toolchain and CI

*   Pinned stable Rust and target `x86_64-pc-windows-msvc`.
*   GitHub Actions on `windows-latest`: format check, clippy with warnings
    as errors, tests.
*   Release builds and signing are not part of M0. Q7 stays open until M6.

### Later milestones (outline only; each gets its own design document)

*   **M1**:
    *   new crates `files-app` (view state, no UI types) and
        `files-ui-<toolkit>` (thin rendering);
    *   UI toolkit chosen after M0 and recorded as an ADR (Architecture
        Decision Record, a short file in `docs/adr/`).
        *   GPUI + gpui-kit is the first candidate, because it looks the
            most modern and has ready tabs, docks, and a virtual table.
        *   eframe/egui is the fallback.
        *   A 1–2 day spike measures both on Windows, as described in
            `docs/research/ui-toolkit-2026-11.md`;
    *   a single-pane list;
    *   Ctrl+V starts a paste, one at a time. The direction is a child
        `rdppaste` process. Queueing and the details are decided in the M1
        design (Q8);
    *   interactive conflict handling (answers Q5).
*   **M2**:
    *   a job queue with concurrency limits in `files-jobs` for local jobs;
    *   local copy, move, and delete through Windows APIs;
    *   background folder sizes with `FindFirstFileExW`, skipping reparse
        points. Initial estimate: 2 workers per physical disk and 1 per
        network share.
*   **M3**: tabs, multiple panes, favourite folders, and "copy path"
    (copies the path without quotes, unlike Windows' Ctrl+Shift+C).
*   **M4**:
    *   themes: dark, light, wallpaper-based, and accent colour;
    *   density: normal, compact, very compact;
    *   UI languages: Polish and English.
*   **M5**: mapping network drives, and the native Windows context menu.
*   **M6**: these items are independent of each other, and each can be
    turned off:
    *   command-line integration;
    *   updates from GitHub releases;
    *   optional replacement of Windows Explorer as the folder handler,
        with a restore command that also works without the GUI
        (`--restore-explorer`).

## Alternatives Considered

### Alternative A: Windows Explorer plus a small paste helper

*   **What it is**: keep Windows Explorer (or FilePilot) as the daily
    explorer. Add a tray hotkey or a "Paste from RDP here" context-menu entry
    that runs a small paste tool.
*   **Pros**:
    *   About a day of work.
    *   No UI to build.
    *   Explorer already pastes RDP files, so the helper would only be needed
        inside FilePilot.
*   **Cons**:
    *   Does not address the freezes if they come from FilePilot.
    *   Does not meet G5: the author would still have no explorer of their
        own, with tabs, panes, themes, and folder sizes.
*   **Reason for Rejection**: fails G5. The `rdppaste` tool from M0 can
    still serve as this helper.

### Alternative B: Use an existing explorer (Directory Opus, Files, OneCommander, Total Commander)

*   **What it is**: switch to another product, or contribute an RDP paste
    fix to Files (open source, C#/WinUI).
*   **Pros**:
    *   Broad feature sets.
    *   Directory Opus is reported to handle virtual files from the
        clipboard (unverified).
*   **Cons**:
    *   Directory Opus is paid and closed source.
    *   Files is reported to use a lot of memory and to have had problems
        with virtual-file paste (both unverified).
    *   OneCommander and Total Commander were not evaluated.
    *   None of the four was tested for RDP paste or freezes, by the
        owner's choice.
*   **Reason for Rejection**: none of them meets G5 (an explorer under the
    author's own control). They remain a reference for features and
    behaviour.

### Alternative C: Paste only through `IFileOperation` / `SHFileOperation`

*   **What it is**: use the Windows file-operation API directly for the RDP
    paste.
*   **Pros**: Windows handles progress, conflicts and the Recycle Bin.
*   **Cons**:
    *   `SHFileOperation` accepts only file paths.
    *   `IFileOperation` is not expected to accept virtual files from a
        `FileGroupDescriptor` (unverified).
*   **Reason for Rejection**: it cannot consume the RDP clipboard format. It
    stays the planned engine for local copy, move and delete in M2.

### Alternative D: `\\tsclient` drive redirection

*   **What it is**: enable drive redirection in the RDP client. Then copy on
    the server into `\\tsclient\D\...`.
*   **Pros**: no code at all.
*   **Cons**:
    *   The copy is started on the remote side, not with Ctrl+V locally.
    *   Customer servers may block drive redirection by policy (Q2).
*   **Reason for Rejection**: it does not match the required workflow (Ctrl+C
    remote, Ctrl+V local). It remains a manual workaround (risk R1).

### Alternative E: Single crate, or two crates (`files-win` + `files`)

*   **What it is**: fewer crates for the prototype.
*   **Pros**: fewer `Cargo.toml` files and less setup for a prototype.
*   **Cons**:
    *   With a single crate, the "`unsafe` only in one place" rule (G4) is
        kept only by discipline.
    *   With two crates, the boundary between the job system and the rules
        is not enforced.
    *   Both need a split later.
*   **Reason for Rejection**: the owner chose four crates, so that the
    compiler enforces every boundary from day one.

### Alternative F: Async runtime (tokio) for jobs

*   **What it is**: run jobs as async tasks on tokio.
*   **Pros**:
    *   Ready-made cancellation (`CancellationToken`) and channels.
    *   Scales to many concurrent I/O tasks.
*   **Cons**:
    *   COM single-threaded apartments need a dedicated OS thread anyway.
    *   File I/O on Windows in tokio runs on a blocking thread pool.
    *   tokio adds a dependency tree and a second concurrency model.
*   **Reason for Rejection**: no benefit for this workload, and it adds code
    and dependencies. Plain threads plus bounded channels cover it.

### Alternative G: Tauri / WebView2 or WinUI 3 for the UI (M1)

*   **What it is**: a web-based or WinUI-based UI layer.
*   **Pros**:
    *   Tauri: UI written with web technologies.
    *   WinUI 3: native Windows 11 controls.
*   **Cons**:
    *   Tauri/WebView2: a WebView2 window adds at least 100 MB of RAM
        (estimate), which conflicts with G2. Its drag-and-drop handles file
        paths only, and the COM code is needed anyway.
    *   WinUI 3: no official Rust support.
*   **Reason for Rejection**: resource budget and support.
*   **Update (2026-11-11):** Microsoft now has an official preview,
    `windows-reactor` in windows-rs. It is still a preview with a baseline
    of about 100 MB, so it is not a candidate, but it is worth watching.
*   **M1 toolkit decision (owner, 2026-11-11):** GPUI + gpui-kit. The
    owner uses Zed daily and is satisfied with how it looks and performs.
    *   The 1–2 day spike in `docs/research/ui-toolkit-2026-11.md` still
        runs, but only to validate the open risks: RAM on Windows, OLE
        drop, and the context menu.
    *   eframe/egui is the fallback only if the spike fails its pass
        criteria.

### Alternative H: `jwalk` for folder size scanning (M2)

*   **What it is**: a parallel directory-walking crate.
*   **Pros**: parallel directory walking with little code.
*   **Cons**: it runs on Rayon's global thread pool, which makes it hard to
    cap parallel I/O per disk.
*   **Reason for Rejection**: it conflicts with G3, which needs bounded,
    cancellable scanning in background mode. Use `FindFirstFileExW` on a
    small fixed pool instead.

### Alternative I: Windows drop target as the paste engine (or as a built-in diagnostic)

*   **What it is**: hand the clipboard data object to the target folder's
    drop target, so that Windows' own copy engine does the paste.
*   **Pros**:
    *   About 50 lines of code.
    *   Windows handles progress, conflicts, long paths, and large files.
*   **Cons**:
    *   Progress, cancel, and conflict prompts happen in Windows' dialog,
        outside the program's job queue.
    *   The program cannot control buffering or I/O priority, so G2 cannot
        be enforced.
    *   It is unknown when the copy finishes; a command-line process could
        exit while it is still running.
*   **Reason for Rejection**: it breaks the `CLAUDE.md` hard requirement (one
    job queue with progress and cancellation) and CUJ 4.
    *   **History:** the owner first chose to build both engines. After
        the first design review the drop target was kept only as a
        `--diagnose-shell` command.
    *   **Why the diagnostic went too:** a later review dropped it, because
        Ctrl+V in Windows Explorer gives the same answer with no code.

### Alternative J: Application manifest with `longPathAware`

*   **What it is**: declare long-path support in the executable's manifest,
    so that all Win32 calls accept paths over 260 characters.
*   **Pros**: covers Win32 and shell calls, not only `std::fs`.
*   **Cons**:
    *   It works only when long paths are enabled in the registry
        (`LongPathsEnabled`), which is off by default.
    *   Embedding a manifest needs a `build.rs` or an extra crate.
*   **Reason for Rejection**: M0 writes only through `std::fs`, which
    handles long paths by itself. M2 decides again when `CopyFileExW` and
    `IFileOperation` arrive.

### Alternative K: Other strategies for writing large files

| Strategy | Effect on cache | Reason not chosen |
| :-- | :-- | :-- |
| Periodic flush (`FlushFileBuffers` every N MB) | Limits Modified pages; Standby still fills | Fails the `CLAUDE.md` requirement that large files do not clog the cache |
| `FILE_FLAG_WRITE_THROUGH` | Data still goes through the cache; Standby fills | Same as above, and slower |
| Do nothing (rely on Windows write throttling) | Both lists fill | This is the suspected cause of the original freezes |
| **`FILE_FLAG_NO_BUFFERING`** (chosen) | Bypasses the cache | Cost: aligned buffer, padded last chunk, about 30–50 lines (estimate) |

### Alternative L: Run the paste inside the GUI process (M1+)

*   **What it is**: the GUI runs paste jobs on its own threads. Each read
    is followed by an "abandoned" flag check. There is a limit on how many
    abandoned threads may exist.
*   **Pros**: no inter-process protocol.
*   **Cons**:
    *   A thread blocked in a COM read cannot be stopped. It can wake up
        later and must be guarded before every write and rename.
    *   Abandoned threads, COM objects, and file handles collect in the
        long-running process until it restarts.
*   **Reason for Rejection**: a child process ends all of this with one
    kill, and `rdppaste` already exists. This is a direction for M1 only;
    the protocol and cancel channel are open (Q8).

## Risks & Mitigations

| ID | Risk | Likelihood / impact | Mitigation | Milestone |
| :-- | :-- | :-- | :-- | :-- |
| R1 | Group Policy on a customer server blocks file transfer through the clipboard (Q2) | Medium / G1 impossible on that server | `--formats` shows the missing formats; documented manual workaround `\\tsclient` (Alternative D). Not fixable by this program | M0 |
| R2 | Antivirus, not the cache, causes the freezes (Q1) | Unknown / G2 not reachable by design alone | M0 baseline with Explorer; if confirmed, document an antivirus exclusion for the target folder | M0 |
| R3 | Descriptor paths are limited to 260 characters | Low / deep remote trees fail | Clear error naming the entry; copy deeper trees in parts | M0 |
| R4 | A blocked COM read cannot be interrupted | Medium / thread stuck | Process exit in M0; child process killed by the GUI from M1 (Alternative L) | M0, M1 |
| R5 | Delete-on-close does not work together with `FILE_FLAG_NO_BUFFERING` (unverified) | Low / leftover `.partial` files | Tested in P2 (item 24). If it fails, stop and redesign the cleanup before P3 | M0 |
| R6 | Unbuffered writes are slower on some disks | Medium / longer copies | Measured in M0; small files stay buffered | M0 |
| R7 | The in-process fake does not behave like the real cross-process RDP client | Medium / bugs found late | Manual RDP run is part of "M0 done" | M0 |
| R8 | Unsigned binary triggers SmartScreen warnings (Q7) | High / friction only | Accepted until M6 | M6 |
| R9 | One developer for M0–M6 | High / slow progress | Independent milestones; `docs/HANDOFF.md` kept current | All |
| R10 | A misbehaving server sends too many entries, too many bytes, or fills the disk | Low / disk full | Limits in "Untrusted clipboard data" | M0 |

## Non-Functional Requirements

*   **Security**:
    *   Everything on the clipboard is untrusted remote input (see
        "Untrusted clipboard data").
    *   The program does not request elevation. Pasting into a folder that
        needs administrator rights fails with an access-denied message.
    *   The log contains names of remote files. It stays on the local
        machine (path in the Detailed Implementation Plan) and is never
        uploaded.
*   **Performance**: G2 and G3 numbers apply. Every job thread runs in
    background mode, and large files bypass the cache.
*   **Rollout and rollback**:
    *   M0 is a standalone executable with no installer, no registry
        changes, and no settings file.
    *   Removing it means deleting the file and
        `%LOCALAPPDATA%\FilesProject\logs\`.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Scope: the repository skeleton and milestone M0, which is `rdppaste` with
the own paste engine and `--formats`. All paths are relative to the
repository root.

This section adds detail to the Technical Plan and must not contradict it.
Any disagreement is a defect to fix in both places.

### Phases

Each phase ends with a check that must pass before the next phase starts.
CI builds the crates under `crates/*`, so CI is green from P1, even though
later crates do not exist yet.

| Phase | Items | Exit check |
| :-- | :-- | :-- |
| P1: skeleton and rules | 1–5, 7, 16–18 | CI green; `cargo test -p files-core` passes |
| P2: Windows layer | 19–24 | `cargo test -p files-win --all-features` passes, including the R5 test (delete-on-close together with unbuffered writes) |
| P3: paste job | 25–28 | `cargo test -p files-jobs --all-features` passes |
| P4: CLI | 29–30 | `rdppaste --formats` run by hand on the author's machine lists clipboard formats |
| P5: documents and RDP | 6, 8–15 | Everything in "M0 done when" |

### Dependency rules

*   Third-party crates are limited to the list below. Adding another one
    needs a line in `docs/HANDOFF.md` saying why.
*   Each version is pinned exactly (`=x.y.z`) when it is added, and
    `Cargo.lock` is committed.
*   Tests use no extra crates:
    *   temporary folders are `std::env::temp_dir()/files-test-<test name>-<pid>`,
        removed at the start and at the end of the test;
    *   large content comes from `Generated` (item 23), and is verified by
        regenerating the same stream and comparing bytes, so no hash crate
        is needed.

| Crate | Used in | Why |
| :-- | :-- | :-- |
| `windows` `=0.58.0` | `files-win` | COM, OLE, shell, and Win32 bindings |
| `windows-core` `=0.58.0` | `files-win`, only with feature `fake` | needed by the `#[implement]` macro in the test fake |
| `tracing` | `files-jobs`, `files-win`, `rdppaste` | structured log events |
| `tracing-subscriber` | `rdppaste` | write log events to a file and to stderr |
| `ctrlc` | `rdppaste` | turn terminal Ctrl+C into cancel |

Arguments are parsed by hand with `std::env::args`, without `clap`.

### Fixed values

| Name | Value | Lives in | Used for |
| :-- | :-- | :-- | :-- |
| `READ_MAX` | 1 MiB | `files-core` | largest single read, and the size of the one buffer per job |
| `ALIGN` | 4096 bytes | `files-core` | buffer alignment and write granularity for unbuffered writes |
| `LARGE_FILE` | 256 MiB | `files-core` | files above this size, or of unknown size, are written unbuffered |
| `MAX_ENTRIES` | 1,000,000 | `files-core` | memory guard on the parsed list |
| `DISK_RESERVE` | 1 GiB | `files-core` | free space that must remain on the target volume |
| `STALL_TIMEOUT` | 60 s | `files-jobs` | `Limits::default().stall` |
| `CANCEL_GRACE` | 900 ms | `files-jobs` | `Limits::default().cancel_grace`; with the 100 ms `wait` tick, cancel returns within 1 s (G1) |
| `PROGRESS_CAP` | 16 | `files-jobs` | progress channel capacity; updates beyond it are dropped |
| `PRINT_EVERY` | 250 ms | `rdppaste` | CLI progress line refresh |

`READ_MAX` and `STALL_TIMEOUT` must satisfy the rule in "Read size and the
stall timeout" in the Technical Plan.

### Target Files to Create/Modify

#### Repository skeleton

#### 1. `Cargo.toml` (Create)

*   **Type / Scope**: workspace manifest.
*   **Proposed Changes**:
    *   `members = ["crates/*"]` and `resolver = "2"`.
    *   `[workspace.package]`:
        *   `edition = "2021"`, so that Rust 1.79 is enough;
        *   `license = "MIT"`;
        *   `repository = "https://github.com/hpixsel/filesProject"`.
    *   `[workspace.dependencies]` holds the pinned versions.
    *   `[workspace.lints.rust]`: `unsafe_code = "forbid"`.
    *   `[workspace.lints.clippy]`: `all = "warn"`.
    *   `[profile.release]`: `lto = "thin"`, `strip = true`.
    *   **Lints per crate**:
        *   Every crate except `files-win` uses `[lints] workspace = true`.
        *   Cargo does not allow combining that with overrides, so
            `files-win` has its own `[lints]` table (item 19).
        *   No crate repeats `#![forbid(unsafe_code)]` in source.

#### 2. `rust-toolchain.toml` (Create)

*   **Type / Scope**: toolchain pin.
*   **Proposed Changes**:
    *   `channel` is the exact stable version current in P1, at least
        `1.79.0` (for `std::path::absolute`).
    *   Components: `rustfmt`, `clippy`.
    *   Target: `x86_64-pc-windows-msvc`.

#### 3. `.gitignore` (Create)

*   **Type / Scope**: git configuration.
*   **Proposed Changes**: ignore `/target`, `*.partial`, and editor folders.

#### 4. `LICENSE` (Create)

*   **Type / Scope**: legal.
*   **Proposed Changes**: MIT text, `Copyright (c) 2026 Kacper Adamus`.

#### 5. `README.md` (Create)

*   **Type / Scope**: public entry point.
*   **Proposed Changes**:
    *   what the project is, in one paragraph;
    *   status ("M0: command-line RDP paste prototype");
    *   the build command;
    *   `rdppaste` usage and exit codes;
    *   the log location, and the fact that the log holds remote file
        names;
    *   the diagnosis step: if `rdppaste` fails, try Ctrl+V in Windows
        Explorer;
    *   links to `docs/ROADMAP.md` and `docs/ARCHITECTURE.md`.

#### 6. `CLAUDE.md` (Modify)

*   **Type / Scope**: agent instructions, written in Polish like the rest
    of the file.
*   **Proposed Changes**:
    *   Add a "Zacznij tutaj" section pointing to `docs/HANDOFF.md`,
        `docs/ARCHITECTURE.md`, `docs/ROADMAP.md`, and
        `docs/designs/files_project_m0.md`.
    *   Add the rule that `unsafe` and Windows calls belong only in
        `files-win`.
    *   Keep the "Narzędzia" section current.
    *   The feature and design lists stay unchanged.

#### 7. `.github/workflows/ci.yml` (Create)

*   **Type / Scope**: CI.
*   **Proposed Changes**:
    *   Trigger: every push and pull request.
    *   Runner: `windows-latest`.
    *   Steps, in order:
        1.  checkout;
        2.  `rustup show`, which installs the toolchain pinned in
            `rust-toolchain.toml`;
        3.  `Swatinem/rust-cache`;
        4.  `cargo fmt --all --check`;
        5.  `cargo clippy --workspace --all-targets --all-features -- -D warnings`;
        6.  `cargo test --workspace --all-features`.
    *   If the clipboard smoke test (item 24) fails because the runner has
        no usable clipboard, mark it `#[ignore]`, run it by hand, and
        record that in `docs/HANDOFF.md`.

#### Documentation for future sessions

#### 8. `docs/HANDOFF.md` (Create)

*   **Type / Scope**: session handoff.
*   **Proposed Changes**: fixed headings:
    *   "Current state";
    *   "Next step";
    *   "Open questions" (Q1, Q2, and Q4–Q8, kept current);
    *   "Experiments" (the Q1 baseline, the Q6 cases, the R5 test result);
    *   "Dependency log".

    Updating this file is part of finishing any task.

#### 9. `docs/ROADMAP.md` (Create)

*   **Type / Scope**: milestone list.
*   **Proposed Changes**:
    *   M0–M6 as checklists, copied from "Later milestones".
    *   M0 is split into phases P1–P5.

#### 10. `docs/ARCHITECTURE.md` (Create)

*   **Type / Scope**: crate map.
*   **Proposed Changes**:
    *   the crate diagram and the direction of dependencies;
    *   one paragraph per crate, saying what belongs there and what does
        not;
    *   the threading and process rules from the Technical Plan.

#### 11. `docs/testing-rdp.md` (Create)

*   **Type / Scope**: manual test procedure.
*   **Proposed Changes**:
    *   **Rig**:
        *   first try loopback RDP (`mstsc /v:localhost` or `127.0.0.2`)
            with clipboard redirection on;
        *   otherwise use a Hyper-V Windows 11 evaluation VM with Remote
            Desktop enabled, connected with `mstsc`.
    *   **G2 measurement**:
        *   Start
            `typeperf "\Memory\Modified Page List Bytes" "\Memory\Standby Cache Normal Priority Bytes" "\Memory\Standby Cache Reserve Bytes" "\Memory\Standby Cache Core Bytes" -si 1 -f CSV -o g2-system.csv`
            10 s before the copy.
        *   After `rdppaste` has started, run a second
            `typeperf "\Process(rdppaste)\Private Bytes" -si 1 -f CSV -o g2-proc.csv`.
        *   During the copy, type in Notepad and switch windows.
    *   **Checklist**:
        *   CUJ 1, 2, 3 and 6;
        *   verify the copy with SHA-256 using `Get-FileHash`;
        *   use a 5 GiB test file of random data;
        *   fill in the results table.
    *   **Q6 cases**: during a paste, change the remote clipboard, then
        repeat with the local clipboard replaced (copy text in Notepad).
        Record the outcome of each.
    *   **Q1 freeze baseline**: the same large copy done with Windows
        Explorer, measured the same way.
    *   **Host check**: `gpresult /h` and the policies "Do not allow
        Clipboard redirection" and "Restrict clipboard transfer".
    *   **Diagnosis**: when `rdppaste` fails, run `rdppaste --formats`, then
        press Ctrl+V into the same folder in Windows Explorer.

#### 12. `docs/adr/0001-workspace-layout.md` (Create)

*   **Type / Scope**: decision record.
*   **Proposed Changes**: four `files-*` crates (Alternative E).

#### 13. `docs/adr/0002-threads-not-async.md` (Create)

*   **Type / Scope**: decision record.
*   **Proposed Changes**: OS threads and bounded channels (Alternative F).

#### 14. `docs/adr/0003-paste-engine.md` and `docs/adr/0004-unbuffered-large-files.md` (Create)

*   **Type / Scope**: decision records.
*   **Proposed Changes**:
    *   **0003**:
        *   the own engine only;
        *   Ctrl+V in Explorer as the diagnostic (Alternative I);
        *   the history: "both engines", then a `--diagnose-shell` command,
            then removed.
    *   **0004**: `FILE_FLAG_NO_BUFFERING` for large files (Alternative
        K). Measuring buffered writes in background mode first was
        considered during review and not chosen.

#### 15. `docs/reviews/m0/` (Create, by moving files)

*   **Type / Scope**: review records.
*   **Proposed Changes**: move these files from the repository root,
    without changing them:
    *   `zoom_out_report.md`
    *   `comprehension_report.md`
    *   `critic_report.md`
    *   `goldfish_review_report.md`
    *   `intermediate_zoom_out_report.md`
    *   `implementation_readiness_report.md`

#### Crate `files-core`

#### 16. `crates/files-core/Cargo.toml` and `src/lib.rs` (Create)

*   **Type / Scope**: crate manifest and library root.
*   **Proposed Changes**:
    *   No dependencies; `[lints] workspace = true`.
    *   Declares the modules `relpath` and `check`.
    *   Holds the `files-core` constants from "Fixed values".
    *   Defines:
        *   `Entry { rel_path: RelPath, kind: EntryKind, modified: Option<SystemTime> }`;
        *   `EntryKind::{File { size: Option<u64> }, Dir}`;
        *   `Progress { files_done: u32, files_total: u32, bytes_done: u64, bytes_total: Option<u64>, current: Option<RelPath> }`.
    *   **`Progress` fields**:
        *   `files_total` counts `File` entries only;
        *   `files_done` goes up after each successful final rename;
        *   `bytes_total` is `None` if any size is unknown, or if the sum
            overflows;
        *   `current` is `None` before the first file and after the last.

#### 17. `crates/files-core/src/relpath.rs` (Create)

*   **Type / Scope**: one untrusted name.
*   **Proposed Changes**:
    *   `RelPath` is a newtype over validated segments.
        *   `parse(&str) -> Result<RelPath, BadName>`;
        *   `to_path_under(&Path) -> PathBuf`;
        *   `ancestors() -> impl Iterator<Item = RelPath>` (proper
            ancestors only, nearest last);
        *   `with_suffix(&str) -> RelPath`, used for `.partial`;
        *   `key() -> String`: each segment upper-cased per character,
            joined with `\`. It is used to find duplicates. Add the comment
            `// shortcut: per-char to_uppercase approximates the NTFS upcase table; switch to the volume's table if a real clash is missed`;
        *   `Display`.
    *   Both `\` and `/` are accepted as separators.
    *   **Rules applied**:
        *   all name rules from "Untrusted clipboard data" except three:
            UTF-16 validity and the NUL terminator (item 21 checks those),
            and the `.partial` length (item 18 checks that);
        *   segments longer than 255 UTF-16 units are rejected;
        *   **reserved names**: take the part of the segment before the
            first dot, trim its trailing spaces, then compare it
            case-insensitively with the reserved list. So `CON .txt` is
            rejected.
    *   `BadName { input: String, reason: &'static str }`.
    *   Inline tests: `a\b.txt`, `CONSOLE.txt` and `a.b.c` pass, and one
        failing case per rule:
        *   `a\..\b`, `C:x`, `file.txt:stream`;
        *   `CON.txt`, `CON .txt`, `conin$`, `COM¹`;
        *   `name.`, `\\server\share`;
        *   a segment of 256 UTF-16 units;
        *   an empty string.

#### 18. `crates/files-core/src/check.rs` (Create)

*   **Type / Scope**: whole-list check before writing.
*   **Proposed Changes**:
    *   `enum ConflictPolicy { Fail, Overwrite }`.
    *   `enum Problem` has these variants:
        *   `Duplicate(String)`, holding the original name of the second
            path:
            *   **Paths compared**: every listed entry, every implied
                ancestor folder (as `Dir`), and the `.partial` path of
                every file.
            *   **Rule**: two of these paths with the same `key()` are a
                duplicate only when at least one of them is a file or a
                `.partial` path.
            *   **Folders merge**: a folder against a folder, listed or
                implied, is never a duplicate.
        *   `PartialNameTooLong(String)`: a file's last segment plus
            `.partial` is longer than 255 UTF-16 units.
        *   `PartialExists(PathBuf)`: `<name>.partial` already exists in
            the target folder.
        *   `Conflict { path: PathBuf, overwritable: bool }`. Every listed
            entry and every implied ancestor is checked against
            `kind_of`:
            *   file on file is overwritable;
            *   folder on file is not overwritable;
            *   file on folder is not overwritable;
            *   folder on folder is not a conflict, because the folders
                merge.
        *   `NotEnoughSpace { need: u64, free: u64 }`, reported when
            `free < need`:
            *   `need` is the sum of the known sizes plus `DISK_RESERVE`,
                computed with `checked_add`;
            *   unknown sizes count as 0;
            *   on overflow, `need = u64::MAX`.
    *   `fn check(entries: &[Entry], target: &Path, policy: ConflictPolicy, kind_of: impl Fn(&Path) -> Option<EntryKind>, free_bytes: u64) -> Vec<Problem>`.
        *   `kind_of` receives absolute paths built with `to_path_under(target)`.
        *   Under `Fail`, every `Conflict` is reported. Under `Overwrite`,
            only the ones that are not overwritable.
        *   An empty result means "go".
    *   **Inline tests**:
        *   one per problem;
        *   each conflict kind under both policies;
        *   merging folders;
        *   a duplicate that differs only by case;
        *   a file `x` next to `x\y` (rejected);
        *   `x\a` next to `x\b` with no `x` entry (accepted);
        *   a listed folder `X` next to an implied `x` (accepted);
        *   `a` next to `a.partial` (rejected);
        *   one known size larger than the free space, plus one unknown
            size (`NotEnoughSpace`);
        *   an implied folder `x` while the target has a file `x`;
        *   two sizes of `u64::MAX / 2 + 1`.

#### Crate `files-win`

#### 19. `crates/files-win/Cargo.toml` and `src/lib.rs` (Create)

*   **Type / Scope**: crate manifest and library root.
*   **Proposed Changes**:
    *   **Dependencies**: `files-core`, `tracing`, and `windows` with these
        features:
        *   `Win32_Foundation`;
        *   `Win32_System_Com`, `Win32_System_Com_StructuredStorage`,
            `Win32_System_Ole`;
        *   `Win32_System_DataExchange`, `Win32_System_Memory`;
        *   `Win32_System_Threading`, `Win32_Storage_FileSystem`;
        *   `Win32_UI_Shell` and `Win32_UI_Shell_Common`, for
            `FILEDESCRIPTORW`, the `CFSTR_*` names, the `FD_*` flags, and
            the fake's `SHCreateStdEnumFmtEtc`.
    *   **Feature `fake`**: `fake = ["windows/implement", "dep:windows-core"]`.
        `windows-core` is an optional dependency.
    *   **Test targets**: `[[test]] name = "virtual_files"` and
        `name = "clipboard_smoke"`, both with `required-features = ["fake"]`.
    *   **Lints**: its own `[lints]` table copies the workspace clippy
        settings and sets `unsafe_code = "allow"`.
    *   **`lib.rs`** declares the modules `sta`, `clipboard`, `io`, and
        `fake` (behind the `fake` feature).
    *   **`Error` enum**: wraps `windows::core::Error` and `std::io::Error`.
        Its own variants:
        *   `NoVirtualFiles { formats: Vec<String> }`;
        *   `BadDescriptor(&'static str)`;
        *   `BadNames(Vec<BadName>)`;
        *   `TargetExists(PathBuf)`.
    *   Every `unsafe` block has a `// SAFETY:` comment.
    *   If a symbol is missing in windows-rs 0.58.0, add the feature that
        its documentation names for it.
    *   If `windows` 0.58.0 has no `implement` feature, use
        `fake = ["dep:windows-core"]` instead.

#### 20. `crates/files-win/src/sta.rs` (Create)

*   **Type / Scope**: COM apartment guard.
*   **Proposed Changes**:
    *   `Sta::new() -> Result<Sta>` calls `OleInitialize`. `Drop` calls
        `OleUninitialize`.
    *   `Sta` holds a `PhantomData<*const ()>`, which makes it `!Send`.
    *   There is no message pump: the job thread blocks only inside COM
        calls, and COM pumps messages there itself.

#### 21. `crates/files-win/src/clipboard.rs` (Create)

*   **Type / Scope**: reading virtual files.
*   **Proposed Changes**:
    *   `clipboard_object(&Sta) -> Result<IDataObject>` wraps
        `OleGetClipboard`.
    *   `formats(&IDataObject) -> Result<Vec<String>>` uses `EnumFormatEtc`.
        Each format is named as follows:
        *   IDs 1–17 by their `CF_*` names (`CF_TEXT` … `CF_DIBV5`);
        *   other IDs with `GetClipboardFormatNameW`;
        *   failing that, `#<id>`.
    *   **`VirtualFiles::from_data_object(sta: &Sta, obj: IDataObject) -> Result<VirtualFiles>`**:
        *   **Formats**: registers `CFSTR_FILEDESCRIPTORW` and
            `CFSTR_FILECONTENTS`.
            *   It calls `QueryGetData` for the descriptor format with
                `TYMED_HGLOBAL`. Any failure means `NoVirtualFiles`, with
                `formats(&obj)`.
            *   A later failure of `GetData` is an ordinary `Win` error.
        *   **Bounds**, checked before reading any entry:
            *   `GetData` the descriptor as an `HGLOBAL`, `GlobalLock` it,
                and set `size = GlobalSize`;
            *   if `size < 4`, return `BadDescriptor("short block")`;
            *   read `cItems`;
            *   if `cItems > MAX_ENTRIES`, return
                `BadDescriptor("too many entries")`;
            *   if `size < 4 + cItems * size_of::<FILEDESCRIPTORW>()`,
                return `BadDescriptor("count exceeds block")`.
        *   **Each entry**:
            *   **Name**:
                *   take `cFileName` up to the first NUL;
                *   no NUL within 260 units means a `BadName` with
                    "descriptor path limit";
                *   decode with `String::from_utf16`; an error means a
                    `BadName` with "invalid UTF-16";
                *   then `RelPath::parse`.
            *   **Size**: only if `FD_FILESIZE` is set,
                `(nFileSizeHigh << 32) | nFileSizeLow`.
            *   **Modified time**: only if `FD_WRITESTIME` is set.
                *   Convert the FILETIME to a `SystemTime` as
                    `UNIX_EPOCH ± (ft − 116_444_736_000_000_000) × 100 ns`.
                *   Use checked arithmetic. Overflow means `None`.
            *   **Kind**: `Dir` only if `FD_ATTRIBUTES` is set and the
                attributes include `FILE_ATTRIBUTE_DIRECTORY`; otherwise
                `File`.
        *   All name errors are collected and returned as one `BadNames`.
        *   The memory is unlocked and released on every path.
    *   `VirtualFiles::from_clipboard(sta: &Sta) -> Result<VirtualFiles>` is
        `from_data_object(sta, clipboard_object(sta)?)`.
    *   `VirtualFiles::entries(&self) -> &[Entry]`. Entries are in
        descriptor order, folders included. An entry's position in this
        slice is its descriptor index.
    *   **`VirtualFiles::open(&self, index: usize) -> Result<ContentReader>`**:
        *   `index` is the position in `entries()`, the same as the
            descriptor index. It is not a count of files only;
        *   calls `GetData` with
            `FORMATETC { cfFormat: FILECONTENTS, lindex: index as i32, tymed: (TYMED_ISTREAM.0 | TYMED_HGLOBAL.0) as u32 }`;
        *   `ContentReader` implements `std::io::Read`:
            *   **`IStream`**: `Read` returns `S_OK` or `S_FALSE`, and both
                mean success. End of stream is `cbRead == 0`.
            *   **`HGLOBAL`**: yields `min(GlobalSize, declared size)`
                bytes when a size is declared, and `GlobalSize` bytes
                otherwise. The second case gets the comment
                `// shortcut: GlobalSize may include allocation padding when no size is declared; reject HGLOBAL without FD_FILESIZE if this shows up`;
        *   the `STGMEDIUM` is released on drop.

#### 22. `crates/files-win/src/io.rs` (Create)

*   **Type / Scope**: file and thread helpers.
*   **Proposed Changes**:
    *   **`background_mode() -> Result<BackgroundGuard>`**:
        `SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN)`.
        `Drop` sets `THREAD_MODE_BACKGROUND_END`.
    *   **`long_path(&Path) -> io::Result<PathBuf>`**:
        *   `std::path::absolute`;
        *   a path that already starts with `\\?\` is returned unchanged;
        *   `\\server\share\…` becomes `\\?\UNC\server\share\…`;
        *   anything else gets the `\\?\` prefix.
    *   **`AlignedBuf::new(len)`**:
        *   `Layout::from_size_align(len, ALIGN)`, then `alloc_zeroed`;
        *   `handle_alloc_error` if the result is null;
        *   `dealloc` with the same layout in `Drop`;
        *   `Deref`/`DerefMut` to `[u8]`.
    *   **`create_partial(path, unbuffered: bool) -> Result<File>`**:
        *   `OpenOptions::new().write(true).create_new(true)`;
        *   `.access_mode(GENERIC_WRITE.0 | DELETE.0)`. The two constants
            are different newtypes, so their raw values are combined.
            DELETE is needed for the disposition flag;
        *   `.custom_flags(FILE_FLAG_NO_BUFFERING.0)` when `unbuffered`;
        *   then `set_delete_on_close(&file, true)`;
        *   if that call fails, drop the file, remove it by path, and
            return the error.
    *   **`set_delete_on_close(&File, bool) -> Result<()>`** wraps
        `SetFileInformationByHandle(FileDispositionInfo, FILE_DISPOSITION_INFO { DeleteFile })`.
    *   **`write_aligned(file: &mut File, buf: &mut [u8], len: usize, last: bool) -> io::Result<()>`**:
        *   precondition: `len % ALIGN == 0` unless `last`;
        *   when `last`, it zeroes `buf[len..round_up(len, ALIGN)]` and
            writes `round_up(len)` bytes;
        *   it uses `write_all`;
        *   the caller then calls `file.set_len(total_written)`;
        *   it carries the comment
            `// shortcut: assumes sector size <= 4096; query FileStorageInfo if a disk rejects unbuffered writes`.
    *   **`rename_no_replace(from, to) -> Result<()>`**:
        *   `MoveFileExW(long_path(from), long_path(to), MOVE_FILE_FLAGS(0))`;
        *   it works while a handle to `from` is still open, because std
            opens files with `FILE_SHARE_DELETE`;
        *   an error whose `e.code()` equals
            `ERROR_ALREADY_EXISTS.to_hresult()` (183) or
            `ERROR_FILE_EXISTS.to_hresult()` (80) becomes `TargetExists`.
    *   **`free_bytes(dir) -> Result<u64>`**:
        *   calls `GetDiskFreeSpaceExW` on `long_path(dir)`, adding a
            trailing `\` only if the path does not already end with one;
        *   returns `lpFreeBytesAvailableToCaller`.

#### 23. `crates/files-win/src/fake.rs` (Create, feature `fake`)

*   **Type / Scope**: test double.
*   **Proposed Changes**:
    *   **`FakeFiles`** is a builder. It is `Send`, holds no COM objects,
        and keeps entries in the order given.
        *   `new()`;
        *   `dir(name)`;
        *   `file(name, FakeFile)`;
        *   `extra_format(cf: u16)`;
        *   `without_descriptors()`;
        *   `raw_count(n: u32)`: overrides `cItems` in the descriptor
            block;
        *   `raw_name(units: [u16; 260])`: adds a file entry whose name is
            exactly these units, with empty content, no declared size, and
            `FD_ATTRIBUTES` set.
    *   **`FakeFile`**, built with `FakeFile::new(content)`:
        *   `content: FakeContent`;
        *   `size: DeclaredSize::{Real, None, Value(u64)}` (default `Real`,
            meaning the content length);
        *   `modified: Option<SystemTime>` (default `None`);
        *   `medium: Medium::{Stream, Memory}` (default `Stream`);
        *   `attributes: bool` (whether `FD_ATTRIBUTES` is set; default
            true).

        The setters are `.size(..)`, `.modified(..)`, `.memory()`, and
        `.no_attributes()`.
    *   **`FakeContent`**:
        *   `Bytes(Vec<u8>)`;
        *   `Generated { len: u64, seed: u64 }`:
            *   a fixed xorshift64 stream starting from `seed`;
            *   tests verify content by regenerating the same stream;
        *   `Faulty { inner: Box<FakeContent>, delay: Duration, fail_after: Option<u64>, hang_after: Option<u64>, release: Option<Arc<AtomicBool>> }`:
            *   valid only with `Medium::Stream`;
            *   `delay` is slept before every `IStream::Read`;
            *   `fail_after` makes `Read` return `E_FAIL`;
            *   `hang_after` blocks `Read` until `release` is set;
            *   `hang_after` without `release` is allowed only in the stall
                test.
    *   **`into_data_object(self) -> IDataObject`** is called on the job
        thread, inside the source closure.
        *   It is implemented with `#[implement(IDataObject)]` and
            `#[implement(IStream)]`.
        *   **`GetData(FILECONTENTS, lindex = i)`** serves entry `i` in
            builder order, with folders counted.
        *   **`QueryGetData`** returns `S_OK` for the descriptor format,
            unless `without_descriptors()` was used, and for each
            `extra_format`. It returns `DV_E_FORMATETC` otherwise.
        *   **`EnumFormatEtc`** returns `SHCreateStdEnumFmtEtc` of this
            list:
            *   the descriptor format with `TYMED_HGLOBAL` and `lindex -1`;
            *   the contents format with `TYMED_ISTREAM | TYMED_HGLOBAL`;
            *   the extra formats.
        *   **`DAdvise`, `DUnadvise` and `EnumDAdvise`** return
            `OLE_E_ADVISENOTSUPPORTED`.
        *   **Other `IDataObject`, `IStream` and `ISequentialStream`
            methods** that are not used return `E_NOTIMPL`.

#### 24. `crates/files-win/tests/virtual_files.rs`, `tests/clipboard_smoke.rs`, and tests in `io.rs` (Create)

*   **Type / Scope**: integration and unit tests.
*   **Proposed Changes**:
    *   **`virtual_files.rs`**: the fake is passed directly, so these tests
        can run in parallel. Cases:
        *   names, folders, unknown sizes, sizes over 4 GiB, and modified
            times are parsed from the descriptor;
        *   an entry without attributes becomes a file;
        *   two invalid names produce `BadNames` listing both;
        *   a name without a NUL produces `BadName` with "descriptor path
            limit";
        *   an unpaired surrogate produces "invalid UTF-16";
        *   a `raw_count` larger than the block produces
            `BadDescriptor("count exceeds block")`;
        *   a `raw_count` above `MAX_ENTRIES` produces
            `BadDescriptor("too many entries")`;
        *   a descriptor block shorter than 4 bytes produces
            `BadDescriptor("short block")`;
        *   `without_descriptors()` plus `extra_format(CF_UNICODETEXT)`
            produces `NoVirtualFiles` listing `CF_UNICODETEXT`;
        *   `open` reads both media;
        *   an `HGLOBAL` with a declared size yields exactly that many
            bytes.
    *   **`clipboard_smoke.rs`**: one test in its own binary.
        *   `OleSetClipboard(fake)`;
        *   `VirtualFiles::from_clipboard`;
        *   `OleSetClipboard(None)`.
    *   **`io.rs`**:
        *   `rename_no_replace` onto an existing file returns
            `TargetExists`;
        *   `long_path` handles drive, UNC, and already-prefixed paths;
        *   **R5 test**: a `create_partial(.., unbuffered = true)` file
            that is written and dropped without clearing the flag is gone,
            and one with the flag cleared survives. Record the result in
            `docs/HANDOFF.md`.

#### Crate `files-jobs`

#### 25. `crates/files-jobs/Cargo.toml` (Create)

*   **Type / Scope**: crate manifest.
*   **Proposed Changes**:
    *   Dependencies: `files-core`, `files-win`, `tracing`.
    *   Dev-dependency: `files-win` with feature `fake`.
    *   `[lints] workspace = true`.

#### 26. `crates/files-jobs/src/lib.rs` (Create)

*   **Type / Scope**: minimal job runner.
*   **Proposed Changes**:
    *   **`Limits { stall: Duration, cancel_grace: Duration }`**.
        `Default` gives `STALL_TIMEOUT` and `CANCEL_GRACE`. Tests pass
        their own.
    *   **`CancelToken(Arc<OnceLock<Instant>>)`**:
        *   `cancel()` records the time the first time it is called;
        *   `is_cancelled()` is `cancelled_at().is_some()`;
        *   `cancelled_at() -> Option<Instant>`.
    *   **`JobCtx { cancel: CancelToken, progress: SyncSender<Progress> }`**:
        *   `report(Progress)` uses `try_send`, and drops the update when
            the channel is full. It is used after reads;
        *   `report_done(Progress)` uses a blocking `send`, so the count of
            finished files always reaches the caller. It is used after
            each successful final rename.
            *   `wait` drains the channel every 100 ms, so the send blocks
                only briefly.
            *   If the receiver is gone, the send fails at once, and the
                error is ignored;
        *   `check_cancel() -> Result<(), JobError>` returns
            `Err(JobError::Cancelled)` when cancelled.
    *   **`start<T: Send + 'static>(name: &str, limits: Limits, f: impl FnOnce(JobCtx) -> Result<T, JobError> + Send + 'static) -> JobHandle<T>`**:
        runs `f` on one thread named `name`, with a channel of
        `PROGRESS_CAP`.
    *   **`JobHandle<T>`**:
        *   `cancel_token(&self) -> CancelToken`;
        *   `wait(self, on_progress: impl FnMut(&Progress)) -> Outcome<T>`.
    *   **`Outcome<T>`** is one of:
        *   `Done(T)`;
        *   `Failed(JobError)`;
        *   `Cancelled`;
        *   `Stalled`.
    *   **`wait` loop**:
        *   The stall clock starts when `wait` is called.
        *   `recv_timeout(100 ms)` on the channel. Each update goes to
            `on_progress` and restarts the stall clock.
        *   **Channel closed**: join the thread and map the result.
            *   `Ok(t)` gives `Done(t)`, even after a late cancel.
            *   `Err(JobError::Cancelled)` gives `Cancelled`.
            *   Any other `Err(e)` gives `Failed(e)`.
            *   A panic gives `Failed(JobError::Panicked)`.
        *   **Cancel grace**: if `cancelled_at()` is more than
            `limits.cancel_grace` ago, return `Cancelled` without joining.
        *   **Stall**: if no cancel has been requested and the stall clock
            exceeds `limits.stall`, call `cancel()` and return `Stalled`
            without joining. Once cancel has been requested, the stall
            check is skipped, so cancel always wins.
        *   Both bounds are therefore met within one 100 ms tick.
    *   **`JobError`** has these variants:
        *   `Cancelled`;
        *   `Panicked`;
        *   `Win(files_win::Error)`;
        *   `Io(std::io::Error)`;
        *   `Problems(Vec<Problem>)`;
        *   `SizeExceeded(RelPath)`;
        *   `SizeShort { path: RelPath, declared: u64, got: u64 }`;
        *   `TargetNotDir(PathBuf)`;
        *   `Entry { path: RelPath, source: Box<JobError> }`. Inside the
            file loop, every error from `open`, `read`, `write`,
            `create_partial`, or the rename is wrapped in it, so the
            message names the entry.

        `From<files_win::Error>` and `From<std::io::Error>` are implemented
        for `JobError`.

#### 27. `crates/files-jobs/src/paste.rs` (Create)

*   **Type / Scope**: the paste job.
*   **Proposed Changes**:
    *   `paste(target: PathBuf, policy: ConflictPolicy) -> JobHandle<Summary>`
        is `paste_from(target, policy, Limits::default(), VirtualFiles::from_clipboard)`.
    *   `paste_from(target: PathBuf, policy: ConflictPolicy, limits: Limits, source: impl FnOnce(&Sta) -> files_win::Result<VirtualFiles> + Send + 'static) -> JobHandle<Summary>`.
        The source runs on the job thread.
    *   `Summary { files_written: u32, folders_created: u32, bytes: u64 }`.
        `folders_created` counts folders that did not exist before,
        including implied ones.
    *   `fn kind_of_path(p: &Path) -> Option<EntryKind>`:
        `std::fs::symlink_metadata(p).ok()`. A folder gives `Dir`; anything
        else gives `File { size: Some(len) }`.
    *   **Job thread**, in pseudocode:
        ```text
        sta = Sta::new()?; _bg = background_mode()?
        if !target.is_dir() -> TargetNotDir
        files = source(&sta)?
        problems = check(files.entries(), &target, policy, kind_of_path, free_bytes(&target)?)
        if !problems.is_empty() -> Problems(problems)          // nothing written
        buf = AlignedBuf::new(READ_MAX)
        for each folder in (implied ancestors ∪ Dir entries), parents first:
            if not exists: create_dir; folders_created += 1
        for (i, entry) in files.entries().iter().enumerate():   // i = descriptor index (lindex)
            if entry is Dir: continue
            ctx.check_cancel()?
            // errors below are wrapped as JobError::Entry { path: entry.rel_path, .. }
            path = entry.rel_path.to_path_under(&target)
            large = size is None or size > LARGE_FILE
            file = create_partial(path + ".partial", unbuffered = large)?   // delete-on-close from here on
            reader = files.open(i)?
            filled = 0; written = 0
            loop:
                n = reader.read(&mut buf[filled..READ_MAX])?
                filled += n; written += n
                if declared size and written > size -> SizeExceeded      // dropping file deletes .partial
                eof = (n == 0)
                if large:
                    if eof:                       write_aligned(file, buf, filled, last = true); filled = 0
                    elif filled == READ_MAX:      write_aligned(file, buf, filled, last = false); filled = 0
                else if filled > 0:               file.write_all(&buf[..filled]); filled = 0
                ctx.report(progress); ctx.check_cancel()?
                if eof: break
            if declared size and written < size -> SizeShort
            if large: file.set_len(written)
            if entry.modified: file.set_modified(t)
            set_delete_on_close(&file, false)?
            r = match policy:                                     // handle still open
                    Fail      -> rename_no_replace(partial, path)
                    Overwrite -> std::fs::rename(partial, path)
            if r is Err: set_delete_on_close(&file, true); return r   // drop deletes .partial
            drop(file)
            files_done += 1; ctx.report_done(progress)
            info!(file = entry.rel_path, "done")
        ```
    *   **Why the loop is safe**:
        *   `filled` only moves forward. A write happens either at a full
            buffer, which is a multiple of `ALIGN`, or at end of stream.
            So every write except the last is aligned.
        *   Every early return drops `file` while delete-on-close is set.
            This includes a failed rename, which sets the flag again first.
            So Windows removes the `.partial`.
    *   **Accepted window**: if the process dies after the flag is cleared
        and before the rename finishes, a complete `.partial` is left
        behind. It can be renamed by hand.

#### 28. `crates/files-jobs/tests/paste_own.rs` (Create)

*   **Type / Scope**: end-to-end tests with the fake passed directly.
*   **Proposed Changes**: tests use
    `Limits { stall: 1 s, cancel_grace: 900 ms }`, and `Faulty` delays of
    50 ms (300 ms in the race test).
    *   **After `Cancelled` or `Stalled`**: `wait` does not join the thread,
        so tests first set `release` (if any), then poll for up to 2 s for
        the `.partial` to disappear.

    Cases, grouped:
    *   **Normal copies**:
        *   a nested tree whose descriptor list interleaves folders and
            files: structure and bytes match, which shows that `lindex` is
            the descriptor index;
        *   a file listed before its parent folder: still written;
        *   the modified time is kept;
        *   unknown size: `bytes_total` is `None`, and content and length
            match;
        *   a 300 MiB `Generated` `Stream` file (above `LARGE_FILE`, not a
            multiple of `ALIGN`): content and length match;
        *   long path: a final path over 300 characters, under both
            policies.
    *   **Conflicts**:
        *   `Fail` with two existing files: both reported, nothing written;
        *   `Overwrite` replaces existing files;
        *   a file where a folder should be: rejected under both policies;
        *   an existing `x.partial`: rejected, and the file is untouched;
        *   `A.txt` and `a.txt`: rejected as `Duplicate`;
        *   `x` and `x\y`: rejected as `Duplicate`;
        *   **TargetExists race** in `Fail` mode:
            *   a `Faulty` delay holds the first file;
            *   the `on_progress` callback creates the final file on the
                first update;
            *   the job fails with `Entry { source: TargetExists }`, the
                created file is untouched, and no `.partial` remains.
    *   **Size checks**:
        *   declared 10, content 20: `SizeExceeded`, and no `.partial`
            remains;
        *   declared 20, content 10: `SizeShort`, and no `.partial`
            remains;
        *   declared sizes larger than the free space: `NotEnoughSpace`.
    *   **Cancel, failure, and stall**:
        *   cancel during a slow `Faulty` read: `Cancelled` in under 1 s.
            No `.partial` remains, and finished files remain;
        *   cancel while `hang_after` blocks: `Cancelled` at least 900 ms
            and under 1.4 s after `cancel()`, never `Stalled`. No
            `.partial` remains after `release`;
        *   `fail_after`: `Failed(Entry { .. })` naming the file, and no
            `.partial` remains;
        *   a job closure that panics: `Failed(Panicked)`;
        *   `hang_after` without cancel: `Stalled` at least 1 s and under
            1.5 s after the last update. After `release`, no `.partial`
            remains.
    *   **Manual only** (`#[ignore]`): a `Generated` 5 GiB file, verified
        by regenerating the stream.

#### Crate `rdppaste`

#### 29. `crates/rdppaste/Cargo.toml` (Create)

*   **Type / Scope**: binary manifest.
*   **Proposed Changes**:
    *   Dependencies: `files-core`, `files-jobs`, `files-win`, `tracing`,
        `tracing-subscriber`, `ctrlc`.
    *   `[lints] workspace = true`.

#### 30. `crates/rdppaste/src/main.rs` (Create)

*   **Type / Scope**: command-line front end.
*   **Proposed Changes**:
    *   **Usage**:
        *   `rdppaste <target-dir> [--overwrite]`;
        *   `rdppaste --formats`;
        *   `rdppaste --help`, which prints the usage and exits with 0.
        *   Unknown arguments print the usage and exit with 64.
    *   **Logging**:
        *   **File layer**:
            *   INFO level, no ANSI;
            *   the file is
                `%LOCALAPPDATA%\FilesProject\logs\rdppaste.log`, opened for
                append inside a `Mutex<File>`;
            *   the folder is created with `create_dir_all`;
            *   if the file cannot be opened, the program continues with
                stderr only and prints one warning.
        *   **Terminal layer**: WARN level, written to stderr.
        *   **Log content**: where they apply, events include the HRESULT,
            the file index, sizes, and cancel points. The log contains
            remote file names and stays local.
    *   **`--formats`**: creates a `Sta` on the main thread, then prints
        `formats(&clipboard_object(&sta)?)`, one per line. Exits with 0, or
        with 1 on a COM error.
    *   **Paste**:
        *   start `paste(...)`, and install a `ctrlc` handler that calls
            `cancel()` on `cancel_token()`;
        *   call `wait` with a callback that prints one progress line to
            stdout, rewritten with `\r` at most every `PRINT_EVERY`. The
            line shows files `n/m`, MiB copied, `%` when the total is
            known, and the current name;
        *   keep the last `Progress` received;
        *   at the end, print one of these:
            *   the `Summary` on `Done`;
            *   the list of problems;
            *   on `Cancelled`, `Stalled`, and `Failed`, the line
                `files_done/files_total completed` from the last
                `Progress` (CUJ 2);
        *   log the outcome as one INFO event, including that count.
    *   **Exit codes**: always through `std::process::exit`, so the
        process ends even while the job thread is still blocked.
        *   0: `Done`;
        *   1: `Failed` (including `Problems`), except `NoVirtualFiles`;
        *   2: `Cancelled`;
        *   3: `Stalled`, with a message saying no progress was seen for
            `STALL_TIMEOUT`;
        *   4: `Failed(Win(NoVirtualFiles))`, printing the formats found;
        *   64: usage error.

### Verification Plan

*   **Per phase**: the exit checks in "Phases".
*   **Build**: `cargo build --workspace --release` with
    `RUSTFLAGS="-D warnings"`.
*   **Tests**: `cargo test --workspace --all-features`.
*   **Large file (manual)**: `cargo test -p files-jobs --all-features -- --ignored`.
*   **Lint**:
    *   `cargo fmt --all --check`;
    *   `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
*   **Coverage rule**: no percentage target. These must each have a test:
    *   every rule in "Untrusted clipboard data";
    *   every `Problem` variant;
    *   every `JobError` variant that the paste job can produce;
    *   every `Outcome` variant.
*   **Manual RDP run** (`docs/testing-rdp.md`):
    *   CUJ 1, 2, 3 and 6, and the Q6 cases;
    *   first on the test rig, then on one real work server;
    *   G2 recorded with `typeperf`.

### M0 done when

*   All five phases have passed their exit checks.
*   CUJ 1, 2, 3 and 6 pass on a real server.
*   G1 and all G2 limits are met.
*   `docs/HANDOFF.md` records:
    *   the Q1 freeze baseline (Explorer, same copy);
    *   the Q6 outcomes;
    *   the R5 test result.
*   `docs/HANDOFF.md` and `docs/ROADMAP.md` are updated.

--------------------------------------------------------------------------------

*Drafted with EGM (Elephant-Goldfish Model: a design process in which the
document is reviewed by reviewers that start from a fresh context).*
