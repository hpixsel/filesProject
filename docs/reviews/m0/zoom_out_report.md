# Stage 5 — Initial Zoom Out Report

*   **Target:** `docs/designs/files_project_m0.md` (full file, lines 1–1017)
*   **Reviewer:** `egm-zoom-out` (fresh context; GoldfishContext: `CLAUDE.md`, `docs/designs/files_project.md`)
*   **Date:** 2026-11-11
*   **Verdict:** CHANGES REQUESTED. No blockers, 5 major findings, 8 nits.

Summary: the goal and the reason for it are clear, and the design mostly
targets the root problem. The main problems:

*   M0 builds a lot of code that may be thrown away.
*   The G2 cache metric measures the wrong kind of memory.
*   The Technical Plan gives function signatures that already disagree with
    the Detailed Plan.

The reviewer did not object to Alternatives A–H, the no-async choice,
`CopyFileExW`/`IFileOperation` reuse, or the handling of untrusted names.

## Major

### 1. Two engines are built in full, and then one is deleted (lines 192–194, 306–342, 914–932, 1002–1008)

*   **Problem**:
    *   The `own` engine is most of M0's code: conflict policies,
        `ContentReader`, `CoCancelCall`, flush logic, fake content types, and
        `paste_own.rs`.
    *   If `shell` wins, all of it is deleted.
    *   Risk 2 can rule out `shell` in a single manual run.
*   **Fix**:
    *   First, a throwaway `shell` spike (`drop_into` only, about one day),
        run once against real RDP.
    *   Build `own` only if the spike fails or finding 2 rules `shell` out.

### 2. The selection rule ignores the hard requirement in `CLAUDE.md` (lines 326–342)

*   **Problem**:
    *   "Fewer lines when both pass" always favours `shell`.
    *   But `shell` gives progress, conflicts and cancel to the Windows
        dialog.
    *   `CLAUDE.md` requires every operation to go through one job queue with
        progress and cancellation, and CUJ 4 needs progress inside the
        window.
    *   So `shell` could win M0 and then be unusable in M1.
*   **Fix**:
    *   Add a criterion: "progress and cancel go through `files-jobs`".
    *   If `shell` cannot meet it, mark it now as a diagnostic or fallback
        only, not a candidate.

### 3. The G2 cache metric contradicts the mechanism (lines 70–72, 293–301, 333, 1005)

*   **Problem**:
    *   Flushing turns Modified (dirty) pages into Standby pages, which
        Windows can reuse at once.
    *   A copy of 10–20 GB fills Standby up to the free RAM regardless of
        flushing.
    *   So "Modified + Standby ≤ 512 MB" fails by definition and triggers the
        `NO_BUFFERING` fallback for no reason.
    *   The likely cause of the freezes is Modified pages building up.
*   **Fix**:
    *   Either set the G2 limit on Modified pages only, with Standby recorded
        for information.
    *   Or plan `FILE_FLAG_NO_BUFFERING` for large files from the start.
    *   Either way, remove the "try flush, then maybe switch" branch.

### 4. The Technical Plan is at signature level and disagrees with the Detailed Plan (lines 199–263 vs. 683–986)

*   **Problem**: these pairs disagree:

    | Technical Plan | Detailed Plan |
    | :-- | :-- |
    | `spawn(name, f)` and `join() -> Result` | `spawn(name, stall_timeout, f)` and `wait() -> Outcome` |
    | `formats() -> Vec<String>` | `formats(&Sta) -> Result<…>` |
    | `open(i) -> impl Read` | `ContentReader` |
    | `drop_into(dir, data)` | `drop_into(&Sta, target, files)` |
    | `current: RelPath` | `current: Option<RelPath>` |
    | `create_partial`/`finish` with `\\?\` | no `long_path`, rely on `std::fs` |
    | `FlushFileBuffers` | `sync_data` |
    | exit codes 0–3 | exit codes 0–4 and 64 |

*   **Fix**:
    *   Keep the Technical Plan at component level: responsibilities,
        dependency direction, threading rules, and data flows.
    *   Keep signatures only in the Detailed Plan, and settle each conflict
        above once.

### 5. Tests depend on the system clipboard (lines 362–374, 856–870, 386, 614, 992)

*   **Problem**:
    *   `OleSetClipboard` in every test forces `--test-threads=1` across the
        whole workspace.
    *   It also makes CI depend on clipboard access on `windows-latest`,
        which has not been verified.
*   **Fix**:
    *   Make the main constructor `VirtualFiles::from_data_object(IDataObject)`,
        with `from_clipboard` as a thin wrapper around it.
    *   Pass the fake directly in tests.
    *   Keep one serial smoke test through the real clipboard, in its own
        test binary.
    *   Drop `--test-threads=1` everywhere else.

## Nits

6.  **`ConflictPolicy` answers Q5 too early** (lines 220–222, 718–730).
    *   Problem: it ships four policies and Rename tries up to 999 names,
        while Q5 is still open. Under `Fail`, a conflict at file 500 leaves
        499 files written. Names, by contrast, are all checked before any
        write.
    *   Fix: M0 ships only Fail and Overwrite. Consider checking conflicts
        on the top-level entries before writing anything.
7.  **Two ways to cancel a blocked read** (lines 230–233, 302–304, 795–798,
    898–900). Fix: keep the watchdog. Run `CoCancelCall` only as an
    experiment noted in HANDOFF.md, without a `CancelHandle` in the API.
8.  **Generic job runner in M0** (lines 199–235, 881–902). The four crates
    stay, as the owner decided. Fix: `files-jobs` in M0 holds only
    `CancelToken`, the progress sender, a thread and the watchdog. The
    general `spawn`/`Outcome` design waits for M1.
9.  **Five fake content variants** (lines 841–858). Fix: `Bytes`,
    `Generated`, and one `Faulty { inner, delay, fail_after, hang_after }`
    wrapper.
10. **Log file** (lines 261, 972–973). Fix: write one file through
    `tracing-subscriber` and drop `tracing-appender`. Make both sections say
    the same thing.
11. **Stale references** (lines 594–603, 674–681).
    *   Item 14 describes creating the file under review.
    *   `CLAUDE.md` line 8 points to the deleted `concepts_brief.md`.
    *   Fix: delete item 14. In item 6, replace that reference with
        `docs/designs/files_project.md`.
12. **Workspace lints** (lines 562–563, 688, 744).
    *   Problem: Cargo does not let a crate combine `lints.workspace = true`
        with its own overrides.
    *   Fix: `files-win` gets its own `[lints]` table. Remove the duplicate
        `#![forbid(unsafe_code)]`.
13. **Freeze baseline (Q1) is missing from M0's done-when list** (lines
    108–113, 1009–1013). Fix: add "Q1 baseline measured and recorded in
    HANDOFF.md".

## Main-context notes (Elephant)

*   Checked: `CLAUDE.md` line 8 does point to `concepts_brief.md`. That file
    was deleted while the design was being finalized. The pointer has now
    been changed to `docs/designs/files_project.md`. This is outside the
    target document, so the review is unaffected.
*   Checked: the Cargo limit on combining `lints.workspace = true` with a
    crate's own lint overrides (finding 12) is real.
*   Findings 1 and 2 reverse the owner's decision C (both engines) by
    argument, not by new data. Accepting them is the owner's call.
