# Handoff

Update this file whenever a task finishes. It is the first thing a new
session reads.

## Current state (2026-11-11)

*   **M0 code:** P1–P4 are done on branch `m0-p1`. Nothing is pushed yet.
*   **Crates:**
    *   `files-core`: the rules (names, the check before writing);
    *   `files-win`: clipboard, I/O, and the test fake;
    *   `files-jobs`: the job runner and the paste job;
    *   `rdppaste`: the command-line tool.
*   **Tests:** `cargo test --workspace --all-features` runs 33 tests, all
    green. One more test is `#[ignore]`: the 5 GiB file.
*   **Manual checks done:** `--help`, `--formats`, CUJ 3 (text on the
    clipboard gives exit code 4), the usage error (exit code 64), and the
    log file.
*   **Not done yet:**
    *   the real RDP run (CUJ 1, 2, 6) and the Q6 cases;
    *   the Q1 freeze baseline;
    *   the G2 measurement.

    See `docs/testing-rdp.md`.
*   **UI toolkit for M1:** GPUI + gpui-kit, decided by the owner. The
    spike is described in `docs/research/ui-toolkit-2026-11.md`.

## Next step

1.  The owner tests `rdppaste` against a real RDP server, and locally by
    copying files out of a `.zip` in Explorer (see `docs/testing-rdp.md`).
2.  Record the results under "Experiments" below.
3.  Fix whatever the real client exposes.
4.  Then close M0 and start M1 with the GPUI spike.

## Open questions

These are from `docs/designs/files_project_m0.md`:

*   **Q1:** the freeze baseline in Explorer.
*   **Q2:** which RDP client and which server policy.
*   **Q4:** whether loopback RDP works.
*   **Q5:** the full conflict UI. M0 has only fail and `--overwrite`.
*   **Q6:** what happens when the session drops or the clipboard changes.
*   **Q7:** code signing.
*   **Q8:** the child-process protocol for M1.

## Experiments

*   **R5, delete-on-close together with `FILE_FLAG_NO_BUFFERING` (P2,
    2026-11-11): works.** The test is `io::tests::delete_on_close_with_unbuffered_writes`.
    *   A rename succeeds while our handle is still open.
    *   `create_new` refuses to overwrite an existing file.
*   **Clipboard busy (P2): `OleGetClipboard`, `QueryGetData` and `GetData`
    can fail with `CLIPBRD_E_CANT_OPEN` right after the clipboard
    changes**, because another process (such as clipboard history) holds
    it open. All three are retried for up to 1 s.
*   **Local virtual-file paste (owner, 2026-11-11): works.**
    *   **What was tested:** 3 files copied with Ctrl+C from inside a
        `.zip` in Windows Explorer. `--formats` showed `Shell IDList Array`,
        `FileGroupDescriptorW`, `FileContents` and `AsyncFlag`.
    *   **First paste:** 3 files, 5.8 MiB, exit code 0.
    *   **Second paste:** all three conflicts were listed, nothing was
        written, exit code 1.
    *   **Issue found:** the progress line wrapped in the terminal and
        updates piled up. Fixed in `7efab0d`.
    *   **Lesson:** this test needs Windows Explorer. FilePilot copies
        nothing to the clipboard from inside a zip.
*   **Q1 baseline:** to do.
*   **Q6 cases:** to do.

## Deviations from the design

*   **`windows` 0.58 needs the `Win32_Graphics_Gdi` feature** for
    `STGMEDIUM` and `ReleaseStgMedium`. The design did not list it.
*   **Rust is pinned to 1.99.1**, the stable version at P1.
*   **The fake's `Faulty` content returns `E_FAIL` once it is released
    from a hang.** That imitates a dropped session.

## Dependency log

| Crate | Version | Why |
| :-- | :-- | :-- |
| `windows`, `windows-core` | =0.58.0 | Win32/COM bindings; `windows-core` only for the test fake |
| `tracing` | =0.1.44 | log events |
| `tracing-subscriber` | =0.3.23 | log to a file and stderr |
| `ctrlc` | =3.5.2 | Ctrl+C becomes cancel |
