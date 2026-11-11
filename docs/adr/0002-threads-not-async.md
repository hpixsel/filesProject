# ADR 0002: OS threads and bounded channels, no async runtime

*   **Status:** accepted, 2026-11-11
*   **Decision:** one `std::thread` per job, a bounded `sync_channel` for
    progress, and a shared `CancelToken`.
*   **Why:**
    *   COM single-threaded apartments need a dedicated OS thread anyway.
    *   File I/O in tokio runs on a blocking pool.
    *   tokio would add a large dependency and a second concurrency model
        for no gain.
*   **Rejected:** tokio.
