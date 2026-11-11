# Stage 8: Intermediate Zoom Out Report

*   **Target:** `docs/designs/files_project_m0.md` (full file, about 1,447 lines)
*   **Reviewer:** `egm-zoom-out` (fresh context). It compared the document
    with `zoom_out_report.md` and did not repeat findings that were already
    fixed.
*   **Date:** 2026-11-11
*   **Verdict:** CHANGES REQUESTED. No blockers, 3 major findings, 5 nits.

## Additions the reviewer judged proportionate

These need no change:

*   CUJ 6;
*   the Risks table;
*   the whole-list check before writing;
*   the name rules;
*   Alternative J;
*   Alternative K, because the requirement in `CLAUDE.md` makes this
    decision necessary.

## Major

### Z1. `--diagnose-shell` duplicates Ctrl+V in Windows Explorer

*   **Problem:**
    *   The document itself says that Explorer accepts this paste.
    *   Pasting into the same folder in Explorer answers the same question
        ("is the clipboard data good, or is `rdppaste` broken?") with no
        code.
    *   The diagnostic costs:
        *   `shell.rs`;
        *   `pump_messages_until`;
        *   a stdin reader thread;
        *   the errors `DropRefused` and `PathTooLong`;
        *   `data_object()`;
        *   the `Win32_UI_Shell*` features;
        *   a CLI mode and a manual test step.
*   **Fix:**
    *   Remove all of the above.
    *   In `testing-rdp.md`, the diagnosis becomes: "run `--formats`, then
        press Ctrl+V in Explorer into the same folder".
    *   Alternative I becomes simply rejected.

### Z2. An M1 decision is fixed and partly built in M0

*   **Problem:**
    *   M0 already ships `--json-progress`, a hand-written JSON format, and
        ADR 0005, but nothing will read that output until M1.
    *   The child-process decision also has gaps:
        *   **Cancel channel:** the cancel request to the child is
            undefined. A GUI without a console cannot send console Ctrl+C.
        *   **Kill race:** the GUI kills the child after 1 s, and the
            child's own `CANCEL_GRACE` is also 1 s, so the `.partial` file
            can be left behind.
        *   **Late clipboard read:** a queued paste reads the clipboard
            only when it starts, so it can paste different content.
        *   **Orphaned child:** nothing stops the child if the GUI crashes.
*   **Fix:**
    *   Keep Alternative L only as a direction, in Later milestones and
        R4.
    *   Remove `--json-progress` and ADR 0005 from M0.
    *   Add an open question for M1 that covers all four gaps. A likely
        answer is that stdin EOF means cancel and a job object kills the
        child.

### Z3. Cleaning up `.partial` through the file system, not the caller

*   **Problem:**
    *   The design shares the current `.partial` path through
        `Arc<Mutex<Option<PathBuf>>>` and deletes the file from two places.
    *   It relies on delete sharing (R5), and it is exposed to the race in
        Z2.
*   **Fix:**
    *   Right after `create_partial`, mark the handle delete-on-close with
        `SetFileInformationByHandle(FileDispositionInfo)`.
    *   Clear the mark just before the final rename.
    *   Windows then deletes the file on every exit path: error, cancel,
        stall, process exit, and process kill.
    *   This removes `set_partial`, the shared path, the deletions in
        `wait`, R5, and the race.
    *   It is unverified whether this works together with `NO_BUFFERING`.

## Nits

4.  **Re-checking free space during the copy** (`SPACE_CHECK_EVERY`,
    `DiskReserve`) is not needed. A full disk already returns
    `ERROR_DISK_FULL`, and the `.partial` file is then deleted. Keep only
    the check before writing.
5.  **`MAX_ENTRIES = 100,000` is arbitrary.** It would reject a real tree,
    while the brief tests 100k files. The bound that matters is missing:
    `cItems` must fit in `GlobalSize` of the `HGLOBAL`, otherwise `unsafe`
    code reads out of bounds, driven by a count from the remote side.
    *   **Fix:** add the `GlobalSize` check, and remove `MAX_ENTRIES` or
        keep it only as a memory guard.
6.  **The `CoCancelCall` experiment** no longer feeds any decision (exit in
    M0, child kill in M1). Remove it from "M0 done when".
7.  **Unbuffered writes were chosen without measuring the cheaper option.**
    *   `THREAD_MODE_BACKGROUND_BEGIN` may also lower the priority of the
        cache pages the thread creates (unverified). Such Standby pages are
        reused first and do not push out other programs' data.
    *   The sum of the Standby counters in G2 cannot tell these pages
        apart.
    *   **Fix:** first measure one buffered copy in background mode, with
        Standby recorded per priority. The owner keeps the decision on K.
8.  **G0 has no pass condition** and repeats G5. Move it to the Overview
    or merge it into G5.
9.  **Duplicates and misplaced detail:**
    *   The rule linking read size to the stall timeout appears twice.
    *   The 4096-byte alignment and padding details belong in item 22, not
        in the Technical Plan.

## Elephant assessment (main context)

*   **Z1, Z2 and Z3 are good cuts.** Together they remove about one fifth
    of the M0 plan and fix the real race between the GUI and the child
    process. Z3 replaces two mechanisms with one flag that the file system
    enforces. I recommend accepting all three.
*   **Nits 4, 5, 6, 8 and 9:** accept. Nit 5 also uncovered a real bug at
    the trust boundary (reading out of bounds in the `unsafe` code).
*   **Nit 7 needs your decision.** The `NO_BUFFERING` decision was taken
    through "auto accept" in stage 7. A one-off `typeperf` measurement of
    a buffered copy in background mode could show that `AlignedBuf`,
    padding and `set_len` (about 30–50 lines) are not needed. The risk is
    a "measure, then maybe switch" branch, which stage 5 criticised.
    *   **Middle ground:** make the measurement the first step of M0,
        before any write code exists, and record the result as ADR 0004.
        Then one version is written.
