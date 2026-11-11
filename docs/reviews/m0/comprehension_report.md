# Stage 6: Comprehension Test Report

*   **Target:** `docs/designs/files_project_m0.md` (FeatureRange 1–524, DetailedRange 525–1056)
*   **Reviewer:** `egm-comprehension`, in a fresh context. GoldfishContext: `CLAUDE.md` and `docs/designs/files_project.md`.
*   **Date:** 2026-11-11
*   **Verdict:** a reader can restate the whole document correctly. No blockers, 7 major findings, 6 nits.

## What this proposes

FilesProject is a Rust file explorer for Windows only, with an MIT licence
and one user. It replaces FilePilot and targets two problems:

*   the whole PC freezes during file operations;
*   files copied inside an RDP session cannot be pasted.

This document covers milestone M0 and the repository skeleton. It also sets
the architecture for M0–M6.

M0 is a command-line tool, `rdppaste <dir> [--overwrite]`. It works like
this:

*   It runs on its own thread in Windows background mode.
*   It reads the "virtual files" that the RDP client puts on the clipboard:
    a list of file descriptions, plus file contents served on request.
*   Before writing anything, it checks every remote file name and every
    conflict with existing files.
*   It copies each file in 4 MiB chunks into `<name>.partial`. Large files
    are flushed to disk every 64 MiB, so the Windows cache of unwritten data
    stays small.
*   When a file is complete, it renames it to the final name.
*   Ctrl+C cancels the paste. If no progress arrives within 30 s after
    cancel, the job is declared "stalled".

The code is split into four crates:

*   `files-core`: pure rules;
*   `files-win`: the only crate with `unsafe` code and Windows calls;
*   `files-jobs`: the job runner and the paste job;
*   `rdppaste`: the binary.

A fake `IDataObject` lets the code be tested without RDP. Paste through the
Windows drop target (`--diagnose-shell`) is kept only as a diagnostic,
because it bypasses the program's own progress, cancel, and job control.

## How the existing system works, per this document

There is no code yet. Today the author uses FilePilot, which has two
problems:

*   It freezes the PC during large operations. The cause has not been
    measured. Suspects are the Windows file cache, unlimited parallel I/O,
    and antivirus.
*   It cannot paste files copied in an RDP session.

When files are copied in an RDP session, the local client (`mstsc` or
`msrdc`) does not put file paths (`CF_HDROP`) on the clipboard. Instead it
puts `CFSTR_FILEDESCRIPTORW` and `CFSTR_FILECONTENTS`, and the file bytes
are pulled over the network through COM on demand. Windows Explorer can
read this; explorers that only read paths cannot.

## Findings

### Major

1.  **Open Questions** (lines 128–137, 363, 642).
    *   **Problem:**
        *   Q6 and Q7 sit under the "Resolved during design" heading, but
            they have no answers.
        *   Line 363 says Q7 stays open until M6.
        *   Item 8 asks to track "Q1–Q7", but Q3 does not exist.
        *   Nothing says what happens when the session drops or the
            clipboard changes during a paste.
    *   **Fix:**
        *   Give Q6 a short answer.
        *   Move Q7 back to the open list.
        *   Explain where Q3 went.
2.  **Stall without cancel** (lines 262–269, 65, 161–163, 916–921, 1021).
    *   **Problem:** the watchdog fires only after cancel. If a read hangs
        or the session drops and the user does nothing, the behaviour is
        undefined. The brief named exactly this case as the reason for the
        watchdog.
    *   **Fix:** state the behaviour. Either the user has to press Ctrl+C,
        or there is an idle timeout that works without cancel.
3.  **`.partial` after a stall** (lines 66, 287, 1021).
    *   **Problem:** G1 says cancel removes partial files, even when the
        watchdog ends the job. But after a stall the job thread is abandoned
        and the process exits, so nobody deletes the `.partial`.
    *   **Fix:** say who cleans up, for example the CLI deletes the known
        path, or the file is reported as left behind. Then align G1 and
        CUJ 2 with that.
4.  **`wait()` and `progress()`** (lines 912–921).
    *   **Problem:** stall detection needs the caller to drain the progress
        channel. If the caller is blocked in `wait()`, nobody drains it and
        `last_seen` never changes.
    *   **Fix:** describe the caller's loop in one sentence.
5.  **"Every problem reported up front" contradicts `BadName` on the first
    name** (lines 292–295 vs. 822).
    *   **Problem:** by the document's own rule the Detailed Plan wins, so
        the claim in the Technical Plan is false for names.
    *   **Fix:** either collect all bad names, or narrow the claim to
        conflicts.
6.  **`shell` is rejected for a requirement M0 also does not meet**
    (lines 50, 307–312, 505, 902–913).
    *   **Problem:** `own` in M0 also has no job queue; the queue arrives in
        M2.
    *   **Fix:** add one sentence. `own` runs as a job the program controls
        and joins the queue in M2 unchanged, while `shell` can never be put
        under the queue's control. M0 meets the progress and cancel parts
        now; the concurrency limit comes deliberately in M2.
7.  **The milestone contents exist only in the Polish brief** (lines 87–89,
    380–382, 653).
    *   **Problem:** G5 depends on what M3 contains, and an English-only
        reader cannot find that out.
    *   **Fix:** list M3–M6 in this document, in English.

### Nits

8.  Line 575 points to "item 16" for the `files-win` manifest; it should be
    "item 18".
9.  "Stage 5 review" and "EGM" are never explained (lines 205, 508, 707,
    1056). Add a date and a description.
10. The Overview (lines 22–24) says M0 "will settle the paste mechanism",
    but this document has already decided it. Reword.
11. "Draft:" in the Decided by column (line 207) is unclear. Say whether
    the owner confirmed it.
12. Item 6 (lines 616–617) lists a change that is already done and refers
    to the deleted `concepts_brief.md`. Remove that bullet.
13. FilePilot is never described or linked, and "ADR" is not expanded on
    first use (lines 14, 370).

## Main-context notes (Elephant)

*   **Finding 1** is an error I made while fixing the Stage 5 findings. I
    inserted the "Resolved during design" block after Q5, so Q6 and Q7
    ended up under it. The fix is mechanical.
*   **Finding 2**: the brief says "a dropped session may hang rather than
    return an error". The design gives no answer for the case where the
    user does not press Ctrl+C. This needs a decision, not just a wording
    change.
