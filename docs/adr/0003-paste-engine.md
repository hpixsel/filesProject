# ADR 0003: Own stream reader is the only paste engine

*   **Status:** accepted, 2026-11-11
*   **Decision:** `rdppaste` reads `FileGroupDescriptorW` and `FileContents`
    itself. When it fails, the diagnostic step is Ctrl+V into the same
    folder in Windows Explorer.
*   **Why:** with the Windows drop target, Windows' own copy engine
    controls progress, cancel, conflicts and buffering. That breaks the
    hard requirement in `CLAUDE.md` (one job queue with progress and
    cancel), CUJ 4, and G2.
*   **History:**
    1.  The owner first chose "build both engines, measure, keep one".
    2.  The first design review turned the drop target into a
        `--diagnose-shell` command only.
    3.  A later review removed that command too, because Explorer gives the
        same answer with no code.
