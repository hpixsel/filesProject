Status: resolved (round 1, auto accept, 2026-11-11)

# Goldfish Annotated Review Report

**Target:** `docs/designs/files_project_m0.md` (lines 1–563, FeatureRange). Stage 7, round 1.
**Source:** `critic_report.md` (IDs C1–C15 and the nits).

Summary:

*   15 findings: PROPOSE FIX.
*   0 findings: CONTEST or ESCALATE.
*   Nits: some fixed, the rest DECLINE.
*   C2 and C6 are variant choices. The proposal below is what I would
    apply on "auto accept".

## Prioritized Findings & AI Suggestions

### [C1] [`structural`, `design`] - Severity: High (Blocker)
*   **Affected Scope:** whole FeatureRange.
*   **Discrepancy Description:** there is no "Risks & Mitigations"
    section. Several risks have no mitigation, and the brief's
    260-character limit was dropped.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** the standard requires this section, and a reader
    cannot confirm that the risks are covered.
*   ***Proposed Edit:*** a new `## Risks & Mitigations` section between
    Alternatives and the non-functional requirements. It is a table with
    these rows:

    | Risk | Mitigation |
    | :-- | :-- |
    | GPO blocks the clipboard (Q2) | Detect it with `--formats`; manual workaround `\\tsclient` (Alternative D); out of our control |
    | Antivirus causes the freezes (Q1) | Baseline measurement in M0; if confirmed, a documented exclusion for the target folder |
    | 260-character limit in the descriptor | A clear error; deeper paths are not supported |
    | Abandoned threads | See C2 |
    | Write performance | See C6 |
    | Delete-sharing assumption | Test in `paste_own.rs`; if it fails, the file is reported as left behind |
    | Fake object versus real cross-process COM | Manual RDP test is required before M0 is done |
    | SmartScreen | Q7 |
    | One person for the whole M0–M6 scope | Milestones are independent; `HANDOFF.md` is kept |

    Open Questions link to the rows.

### [C2] [`design`] - Severity: Medium
*   **Affected Scope:** Threading rules, "Why `shell`…", Later milestones M1.
*   **Discrepancy Description:** stall handling relies on the CLI exiting.
    In a GUI process, abandoned threads keep running and pile up.
*   ***AI Suggestion:*** PROPOSE FIX (variant A).
*   ***Justification:*** **variant A**. From M1 the GUI runs each paste as
    a child process, `rdppaste --json-progress`, and kills it on stall or
    cancel.
    *   Killing the process ends the COM call, the open file handles, and
        the thread in one step. This removes abandoned threads, the
        delete-while-open trick, and the thread limit.
    *   The M0 code stays the same apart from one output flag.
    *   Variant B (an "abandoned" flag checked after every `Read`, plus a
        thread limit inside one process) needs more code, and the problem
        remains in the long-running process.
*   ***Proposed Edit:***
    *   In Threading rules: "From M1, the GUI runs each paste as a child
        `rdppaste` process and kills it on cancel or stall."
    *   Remove "without changes" (line ~338).
    *   Add Alternative L: "In-process job with an abandon flag and a
        thread limit", rejected.
    *   In the M1 outline: "paste runs as a child process; progress as
        JSON lines on stdout".

### [C3] [`design`, `style`] - Severity: Medium
*   **Affected Scope:** G1, CUJ 2, Threading rules.
*   **Discrepancy Description:**
    *   cancel waits for the stall timeout;
    *   the timeout can end a slow but healthy transfer;
    *   the term is undefined.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Proposed Edit:***
    *   **Definition**: G1 defines "stall timeout: 60 s with no bytes
        received".
    *   **Cancel**: always finishes within 1 s. If a read is blocked, the
        thread or process is abandoned (in M1, killed) and the `.partial`
        file is removed.
    *   **Read size**: a single read requests at most 1 MiB. At a minimum
        supported speed of 64 KiB/s that takes 16 s, well below 60 s. The
        rule is recorded in the specification.
    *   **New CUJ 6**: "RDP session drops during a transfer, user does
        nothing". Then:
        *   the job ends as stalled within 60 s;
        *   finished files remain;
        *   no `.partial` file remains.

### [C4] [`design`] - Severity: Medium
*   **Affected Scope:** data flow, "Checks before writing".
*   **Discrepancy Description:** `.partial` can overwrite or delete a
    user's file, can exceed 255 characters, and `rename` overwrites a file
    even in Fail mode.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Proposed Edit:***
    *   The pre-write check also covers the `.partial` names, including the
        255-character limit per name and collisions inside the list.
    *   The temporary file is created with create-new and is never
        truncated.
    *   In Fail mode, the final rename uses no-replace (`MoveFileExW`
        without `REPLACE_EXISTING`). A collision becomes a job error.

### [C5] [`design`] - Severity: Medium
*   **Affected Scope:** Later milestones, M1.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Proposed Edit:*** M1 runs one paste at a time. A further Ctrl+V is
    queued in order (FIFO) and is visible in the window.

### [C6] [`design`, `structural`, `style`] - Severity: Medium
*   **Affected Scope:** G2, Flushing large files, NFR Performance.
*   **Discrepancy Description:**
    *   Standby pages are not "harmless": a large copy pushes other
        programs' data out of the cache.
    *   Background priority does not cover cached writes.
    *   Alternative write strategies are not compared.
    *   The metric is not measurable.
*   ***AI Suggestion:*** PROPOSE FIX (variant: `NO_BUFFERING` for large
    files).
*   ***Justification:***
    *   The hard requirement in `CLAUDE.md` says literally "large files do
        not clog the Windows cache", and that includes Standby.
    *   Only unbuffered writes meet it. The cost:
        *   a buffer aligned to 4096 bytes;
        *   the last chunk written padded to 4096;
        *   the file then truncated to its real size with `set_len`.
    *   That is about 30–50 lines (estimate).
    *   Periodic flushing is removed.
*   ***Proposed Edit:***
    *   **Write strategy**: files above `LARGE_FILE` are written with
        `FILE_FLAG_NO_BUFFERING`; smaller files are written normally.
    *   **G2**: the counters `Memory\Modified Page List Bytes` and
        `Memory\Standby Cache *` in Performance Monitor. Each must rise by
        at most 512 MB above the value measured before the copy.
    *   **Wording**: Windows repurposes Standby pages on demand.
    *   **NFR**: background mode covers the thread's own reads and writes.
        Unbuffered writes are performed by the thread itself.
    *   **Alternative K**, comparing four write strategies: periodic
        flush, `NO_BUFFERING` (chosen), `WRITE_THROUGH`, nothing.

### [C7] [`design`] - Severity: Medium
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Proposed Edit:*** the section becomes "Untrusted clipboard data"
    and adds these rules:
    *   a file is not allowed to grow beyond its declared size (error);
    *   at most 100,000 entries;
    *   before writing, free space ≥ the sum of declared sizes + 1 GiB;
    *   when sizes are unknown, the paste stops if free space falls below
        1 GiB;
    *   the same rules are added to the Security NFR.

### [C8] [`design`] - Severity: Medium
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Proposed Edit:***
    *   Q6 gets a new case: "the local clipboard is replaced during a
        transfer (for example Ctrl+C in a terminal)". The expected result
        is "failed" or "stalled", with finished files kept.
    *   `docs/testing-rdp.md` gets a test step for this case.
    *   CUJ 2 is changed to cancel through Ctrl+C in the console. That
        sends a signal and does not touch the clipboard while no text is
        selected, and this is now stated.

### [C9] [`structural`] - Severity: Medium
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Proposed Edit:*** add an edge `rdppaste → files-win` to the
    diagram, marked "diagnostics: --formats, --diagnose-shell".

### [C10] [`structural`, `design`, `style`] - Severity: Medium
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Proposed Edit:***
    *   Replace the rule with: "The Detailed Implementation Plan adds
        detail and must not contradict this section; any disagreement is a
        defect to fix in both."
    *   The same change in the Detailed Plan.
    *   Mark the numbers in Later milestones as "initial estimate".

### [C11] [`structural`, `design`] - Severity: Medium
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Proposed Edit:***
    *   add Alternative J, "`longPathAware` manifest", rejected because it
        needs a registry setting plus `build.rs`;
    *   add an "Alternative" column to the decision table;
    *   the long-path decision applies to `std::fs` in M0 only. M2 and
        later decide again for `CopyFileExW`, `IFileOperation` and
        `FindFirstFileExW`.

### [C12] [`structural`, `design`, `style`] - Severity: Medium
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Proposed Edit:***
    *   New goal **G0**: "The author has their own daily explorer, under
        their control, with the features in `CLAUDE.md`".
    *   Alternatives A and B are rejected against G0, with this wording:
        "OneCommander, Total Commander, Directory Opus and Files were not
        tested, by the owner's choice."
    *   The claims about Files are marked "(unverified)".

### [C13] [`style`] - Severity: Medium
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Proposed Edit:*** four corrections, as in `critic_report.md` C13:
    *   the scope of "most explorers";
    *   COM is local, and the bytes travel over the RDP channel;
    *   "from the repository contents alone";
    *   the exit code gives the status; the count is in the output and log.

### [C14] [`structural`, `style`, `design`] - Severity: Medium
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Proposed Edit:***
    *   remove the references to `zoom_out_report.md`;
    *   say once: "revised after the first design review (2026-11-11)";
    *   shorten Alternative I to one sentence of history.

### [C15] [`design`] - Severity: Low/Medium
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Proposed Edit:***
    *   **Folder creation**: the parent folder of every file is created
        before the file, whatever order the list arrives in.
    *   **Pre-write check**: it also catches duplicates in the list,
        compared case-insensitively, including a file and a folder with
        the same name.
    *   **Reserved names**: add `CONIN$`, `CONOUT$`, `COM¹–³` and `LPT¹–³`.
    *   **Name length**: at most 255 characters per name.
    *   **No attributes field**: an entry without it is treated as a file.
        If opening its contents fails, the paste ends with a clear error.

## Nits

*   **PROPOSE FIX** (cheap and valuable):
    *   title: "FilesProject: architecture and milestone M0 (RDP paste
        prototype)";
    *   `### Resolved` as a heading;
    *   "Proposed solution" moves to the Overview;
    *   4 GiB;
    *   the 50 ms limit applies from M1;
    *   one term: "background mode";
    *   "does not request elevation";
    *   G1: the last-write time is kept when the descriptor provides it;
    *   the log path and its content (it contains names of remote files)
        are defined;
    *   the FilePilot custom-command question is closed because of the
        decision to build.
*   **DECLINE**:
    *   regrouping the alternatives and a single template for them;
    *   purely wording nits ("Mature", "Familiar", "pile up", "on
        purpose");
    *   the clipboard test in CI (the plan already covers it: it is
        `#[ignore]` if it fails on the runner);
    *   renaming G4.
