# Stage 7 — Critic Review Report (round 1)

*   **Target:** `docs/designs/files_project_m0.md`, FeatureRange lines 1–563
*   **Reviewers** (fresh context, all completed):
    *   `egm-design`: feature-spec standard;
    *   `egm-structural`: feature-spec standard;
    *   `egm-style`: style persona rules.
*   **GoldfishContext:** `CLAUDE.md`, `docs/designs/files_project.md`
*   **Date:** 2026-11-11
*   **Raw totals:**

    | Reviewer | Blocker | Major | Nit |
    | :-- | :-- | :-- | :-- |
    | design | 0 | 9 | 9 |
    | structural | 1 | 4 | 7 |
    | style | 0 | 9 | 27 |

*   **After deduplication:** 1 Blocker, 14 Major, and the Nits grouped
    into one list.

## Blocker

### C1. No "Risks & Mitigations" section
*   **Sources:** structural B1, design M7.
*   **Problem:** risks are scattered across Open Questions, Threading rules,
    and the Security non-functional requirement. Several have no mitigation:
    *   Q2: a server's Group Policy blocks the clipboard, and the product
        does not work on that server;
    *   Q1: antivirus is the cause of the freezes;
    *   a 260-character limit in the descriptor format (it was in the brief
        and was dropped);
    *   abandoned threads in M1;
    *   the cost of flushing;
    *   the assumption about delete sharing;
    *   the in-process fake versus real cross-process COM;
    *   SmartScreen;
    *   one person for the whole M0–M6 scope.
*   **Fix:** a table with the columns Risk | Likelihood/Impact |
    Mitigation | Milestone. Keep Open Questions for unknowns only.

## Major: architecture

### C2. Stall handling works only because the CLI exits; it breaks in M1
*   **Source:** design M1.
*   **Problem:** in a GUI process that keeps running, an abandoned thread
    can wake up later. It can then write to the deleted `.partial`, rename
    it, or move on to the next files. Abandoned threads also pile up. So
    "`own` goes under the M2 queue without changes" is not supported.
*   **Fix:**
    *   after every `Read`, check an "abandoned" flag before any write or
        rename;
    *   decide on a limit for abandoned threads;
    *   record an alternative: run the paste in a child process (`rdppaste`)
        that the GUI can kill;
    *   remove "without changes".

### C3. Cancel waits for the stall timeout; the timeout can kill a slow but healthy transfer
*   **Sources:** design M2, style M1.
*   **Problem:**
    *   After cancel, the thread could be abandoned after 1 s, not after 60.
    *   A single 4 MiB read on a slow link can take longer than the timeout.
    *   The term "stall timeout" is undefined in the specification.
*   **Fix:**
    *   cancel always completes within 1 s (abandon the thread, delete the
        `.partial`);
    *   define the timeout and its relation to chunk size and minimum
        bandwidth;
    *   add a CUJ for "session dropped during a transfer".

### C4. The `.partial` file can destroy a user's file
*   **Sources:** design M3, design N1.
*   **Problem:**
    *   An existing `x.partial`, or a remote list with both `x` and
        `x.partial`, is overwritten or deleted.
    *   The suffix can push a name past 255 characters.
    *   `std::fs::rename` overwrites a file created after the check, even
        in `Fail` mode.
*   **Fix:**
    *   a temporary name that cannot collide (`<name>.<jobid>.partial` or a
        hidden staging folder), opened with create-new;
    *   include temporary names in the conflict and length checks;
    *   in `Fail` mode, rename with `MoveFileExW` without
        `REPLACE_EXISTING`.

### C5. M1 has Ctrl+V without a queue, which breaks the hard requirement in `CLAUDE.md`
*   **Source:** design M4.
*   **Problem:** five presses of Ctrl+V give five unlimited parallel pastes.
*   **Fix:** in M1, one paste at a time; another Ctrl+V is queued (FIFO) or
    refused.

### C6. The cache strategy rests on unverified claims and has no comparison of alternatives
*   **Sources:** design M5, structural M2, style M2/M3.
*   **Problem:**
    *   Standby pages are not handed over "immediately". A large copy
        pushes other programs' data out of the cache, so they slow down
        afterwards.
    *   Background I/O priority does not cover cached writes, which Windows
        performs later on its own threads.
    *   Periodic flushing is not compared with `NO_BUFFERING`,
        `WRITE_THROUGH`, or doing nothing.
    *   "About 512 MB because of the copy" cannot be measured.
*   **Fix:**
    *   add Alternative K (write strategy) comparing the four options;
    *   decide whether Standby must be limited;
    *   correct the security/performance non-functional requirement;
    *   name the performance counters and tool for G2;
    *   correct the wording to "Windows repurposes Standby on demand".

### C7. Clipboard data other than names is untrusted; disk use has no limit
*   **Source:** design M6.
*   **Problem:**
    *   a declared size that is exceeded;
    *   millions of entries;
    *   no check of free space, so a 6 GB paste fails halfway on a full
        disk.
*   **Fix:**
    *   stop at the declared size;
    *   a limit on the number of entries;
    *   a free-space check before writing, plus a reserve when the size is
        unknown;
    *   the section becomes "Untrusted clipboard data".

### C8. Local clipboard changes during a paste
*   **Source:** design M8.
*   **Problem:** even Ctrl+C in the terminal replaces the clipboard (CUJ 2).
    It is not known whether `mstsc` keeps serving the data once it no
    longer owns the clipboard.
*   **Fix:** add this to Q6 and to the manual RDP test, and define the
    expected behaviour.

### C9. The dependency diagram does not match the description
*   **Source:** structural M4.
*   **Problem:** `rdppaste --formats` and `--diagnose-shell` use `files-win`
    directly, but the diagram has no such edge.
*   **Fix:** add the edge `rdppaste → files-win`, labelled "diagnostics
    only".

## Major: document structure and accuracy

### C10. "The Detailed Implementation Plan wins"
*   **Sources:** structural M1, design N4, style N17.
*   **Problem:**
    *   The specification makes itself subordinate to the plan.
    *   It says exact values live only in the plan, yet it contains them,
        for example "2 workers per disk".
*   **Fix:**
    *   "The parts must agree; a disagreement is a defect to fix."
    *   Mark values in the Technical Plan as initial estimates.

### C11. Decisions with no alternative recorded
*   **Source:** structural M2.
*   **Problem:**
    *   a `longPathAware` manifest (from the brief);
    *   unbuffered writes (covered by C6);
    *   the long-path decision is described as product-wide, but it holds
        only for `std::fs` (design N5).
*   **Fix:**
    *   Alternative J (manifest);
    *   an "Alternative" column in the decision table;
    *   limit the long-path decision to M0.

### C12. Alternatives A and B rejected only by "owner decision"
*   **Sources:** structural M3, design M9, style M9.
*   **Problem:**
    *   OneCommander and Total Commander have no cons listed.
    *   The claims about Files ("high memory", "mixed record") have no
        source.
    *   The real reason, "an own explorer for daily use", is not in Goals.
*   **Fix:**
    *   a goal G0 "own, controlled daily explorer", with A and B rejected
        against it;
    *   say explicitly that the products were not tested by the owner's
        choice;
    *   mark the claims about Files as "(unverified)".

### C13. Inaccurate statements
*   **Source:** style M4–M7.
*   **Problem and fix:**
    *   "most third-party explorers" → "FilePilot and explorers that read
        only `CF_HDROP`".
    *   "bytes pulled across the network through COM" → COM is the local
        interface; the bytes travel over the RDP channel.
    *   "without this conversation" → "from the repository contents alone".
    *   "the exit code states how many files completed" → the exit code
        gives the status; the count is in the output and the log.

### C14. Reference to an uncommitted `zoom_out_report.md`
*   **Sources:** structural N3, style M8, design N9.
*   **Problem:** the reference will break after publication. The same
    history is repeated three times.
*   **Fix:** shorten it to "revised after the first design review
    (2026-11-11)", and either commit the reports to `docs/reviews/` or
    remove the references.

### C15. Gaps in name validation and descriptors
*   **Sources:** design N2/N3.
*   **Problem:**
    *   order of entries (parents created before their files);
    *   duplicates that differ only by case;
    *   a file `x` and a folder `x\…` in the same list;
    *   `CONIN$`, `CONOUT$`, `COM¹–³` and `LPT¹–³`;
    *   the 255-character limit per name;
    *   a missing attributes field.
*   **Fix:** extend the rules and the pre-write check.

## Nits (grouped)

*   **Structure:**
    *   one template for Alternatives A–I, grouped by decision (structural
        N1/N2);
    *   the Overview's scope map is incomplete (N4);
    *   title versus the M0 scope (N5);
    *   `### Resolved` as a heading (N6);
    *   "Proposed solution" is in the Problem section (N7).
*   **Measurability:**
    *   50 ms in M0, which has no UI (style N4);
    *   the manual check in G2 needs a defined procedure (N5);
    *   "4 GB" should be 4 GiB (N3);
    *   CUJ 3, 4 and 5 use undefined terms: "clear message", "responsive",
        "progressively" (N14–N16).
*   **Consistency:** one name for background mode (N6); G4 named
    "Enforced code boundaries and handoff" (N7); the "Decided by" column
    holds only the decider (N18).
*   **Wording:** "for now", "v1", "actually", "the bug is ours", "pile
    up", "on purpose", "Polish and English UI", "easy restore", "Mature",
    "Familiar", "about 100 MB or more" (N9–N27).
*   **Other:**
    *   "never runs elevated" should be "does not request elevation"
        (design N6);
    *   the clipboard test in CI is unverified (N7);
    *   file metadata (modification time, attributes) is not addressed in
        G1 (N8);
    *   the log path and its content (names of remote files) are not
        defined (N9);
    *   the brief's question about custom commands in FilePilot was dropped
        (N9).

## Elephant assessment (main context)

*   **Accept without discussion:** C1, C3, C4, C5, C7, C8, C9, C10, C11,
    C13, C14, C15, and the nits. These are real gaps or errors, and the
    fixes are local.
*   **C2 needs a decision from you.** The child-process option
    (`rdppaste` as a worker that the GUI starts and can kill) removes
    abandoned threads and the delete-while-open trick altogether. It is
    attractive, because the binary exists already. The cost is a protocol
    between the processes (progress over stdout) from M1 onwards. The
    alternative is an "abandoned" flag plus a thread limit inside one
    process. The M0 code is the same in both cases, so the decision can be
    recorded now and implemented in M1.
*   **C6 needs a decision from you.**
    *   Limiting only Modified pages is the cheapest option, but a large
        copy still pushes other programs' data out of the Standby list.
    *   `FILE_FLAG_NO_BUFFERING` does not touch the Standby list at all.
        The costs are aligned buffers, a separate write for the final
        chunk, and roughly 30–50 extra lines (estimate).
    *   Because this concerns the original "the computer slows down"
        problem, the measurement in M0 should decide. Both variants can be
        described now, with one chosen as the default.
*   **C12:** adding goal G0 is honest and costs nothing. Running a test of
    Directory Opus or Files is up to you; I do not recommend making it a
    precondition.
