# ADR 0004: Unbuffered writes for large files

*   **Status:** accepted, 2026-11-11
*   **Decision:** files above 256 MiB, and files of unknown size, are
    written with `FILE_FLAG_NO_BUFFERING`. This needs:
    *   a buffer aligned to 4096 bytes;
    *   a zero-padded last chunk;
    *   a final `set_len`.
*   **Why:** `CLAUDE.md` requires that large files do not clog the Windows
    file cache. Only unbuffered writes avoid both the Modified and the
    Standby pages.
*   **Rejected:**
    *   periodic `FlushFileBuffers`, which still fills the Standby pages;
    *   `FILE_FLAG_WRITE_THROUGH`, which still goes through the cache;
    *   doing nothing, which is the suspected cause of the original
        freezes;
    *   measuring buffered writes in background mode first, which was
        considered during review and not chosen.
