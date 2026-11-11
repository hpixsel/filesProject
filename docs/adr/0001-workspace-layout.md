# ADR 0001: Four `files-*` crates from day one

*   **Status:** accepted, 2026-11-11 (owner)
*   **Decision:** a Cargo workspace with `files-core` (rules, no `unsafe`),
    `files-win` (the only `unsafe`, COM and Win32), `files-jobs` (runner and
    paste job) and the `rdppaste` binary.
*   **Why:** the compiler enforces the boundaries, above all "`unsafe` only
    in `files-win`" (goal G4).
*   **Rejected:** a single crate (boundaries kept only by discipline), and
    two crates (the rules/jobs boundary would not be enforced).
