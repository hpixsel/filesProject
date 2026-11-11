# ADR 0005: Local copy, move and delete go through IFileOperation

*   **Status:** accepted, 2026-11-11
*   **Decision:** pasting regular files (`CF_HDROP`), dropped files, cut and
    delete all use Windows' own `IFileOperation`. It runs on its own OLE
    thread for clipboard pastes and deletes. For drops it runs inside
    `Drop`, because 7-Zip and NanaZip delete their temporary folder when the
    drop returns.
*   **Why:**
    *   Very little code (`files-win/src/fileop.rs`, `drop.rs`).
    *   Behaviour matches Explorer: progress dialog, conflict prompts,
        undo, Recycle Bin, long paths, elevation prompts.
    *   Windows writes large files unbuffered itself.
*   **Trade-off:**
    *   The progress and cancel UI for local operations is Windows' own
        dialog, not our job queue.
    *   This bends the hard requirement in `CLAUDE.md` ("one job queue with
        progress and cancel"). It is accepted because Windows' engine
        already gives progress, cancel and bounded I/O for local work.
    *   RDP pastes, where Windows' engine does not work for us, keep our
        own engine (ADR 0003).
*   **Job queue (M2):** not built. Concurrency is already bounded:
    *   one RDP paste at a time;
    *   two folder-size workers;
    *   each `IFileOperation` serialises its own work.

    Revisit if parallel local operations start to freeze the PC.
*   **Known limit:** one selected item per operation. Multiple selection is
    the next step.
