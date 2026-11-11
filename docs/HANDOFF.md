# Handoff

Update this file whenever a task finishes. It is the first thing a new
session reads.

## Current state (2026-11-11)

*   **M0:** merged to `main` in PR #1. Real RDP pastes work: small files,
    218 MiB, 1.1 GB through the unbuffered path, and cancel.
*   **M1 (branch `m1`):** the first window, `files.exe`
    (`crates/files-ui-gpui`), built with GPUI and gpui-kit.
    *   **Layout:** a single pane with back, forward, up and refresh, an
        address bar, and a table of files with folders first and icons.
    *   **Opening:** double-click opens a folder, or a file with its
        default app.
    *   **Context menu:** right-click shows Explorer's own menu, dark in
        dark mode.
    *   **RDP paste:** Ctrl+V or the "Paste from RDP" button runs
        `rdppaste --machine` as a child process, with progress and cancel.
    *   **Look:** a Blurred backdrop and a palette derived from one accent
        colour.
    *   **Owner check:** the owner tested it by hand and everything worked.
*   **Crates:**
    *   `files-core`: the rules, plus the `machine` line protocol;
    *   `files-win`: clipboard, I/O, `shell` (context menu, open, dark
        menus, local time), and the test fake;
    *   `files-jobs`: the runner and the paste job;
    *   `files-app`: folder listing and navigation history;
    *   `files-ui-gpui`: the window;
    *   `rdppaste`: the command-line tool, and the worker process for the
        GUI.
*   **Tests:** `cargo test --workspace --all-features` passes. The 5 GiB
    test stays `#[ignore]`.
*   **Lessons, see `docs/research/ui-toolkit-2026-11.md`:**
    *   modal Win32 loops (menus, `ShellExecute`) must run outside GPUI's
        event dispatch;
    *   after changing colours, recompute `theme.tokens`;
    *   icons need `with_assets(gpui_kit::assets::Assets)`.
*   **Still open for M0:** CUJ 6 (session drop), the Q6 cases, the G2
    `typeperf` measurement, and the Q1 baseline.

## Next step

1.  M1 is complete; conflict handling (Q5) was merged with branch
    `m1-conflicts`.
2.  M2: the job queue, plus local copy, move and delete. This includes
    pasting regular files (`CF_HDROP`), for example from NanaZip or
    7-Zip, or a plain Explorer copy. Today the window says these come with
    M2. Folder sizes are computed in the background.
3.  Q8 leftover: a job object, so `rdppaste` dies with the window.

## Open questions

These are from `docs/designs/files_project_m0.md`:

*   **Q1:** the freeze baseline in Explorer.
*   **Q2:** which RDP client and which server policy.
*   **Q4:** whether loopback RDP works.
*   **Q5:** answered in M1. The choices are replace, skip existing, or keep both (`name (2).ext`), through rdppaste exit code 5 and a bar in the window.
*   **Q6:** what happens when the session drops or the clipboard changes.
*   **Q7:** code signing.
*   **Q8:** answered in M1. The protocol is `files_core::machine` (tab-separated lines on stdout; closing stdin cancels). One paste runs at a time.
*   **Q8 still open:** a job object, so the child dies with the GUI. Today the child only cancels when stdin closes.

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
*   **Real RDP paste (owner, 2026-11-11): works.** This is the core M0
    goal.
    *   **What was tested:** 2 small files (a `.zip` and a `.ps1`, about
        29 KB) copied with Ctrl+C in Explorer on the server, pasted locally
        with `rdppaste`, exit code 0.
    *   **Client formats:** `FileGroupDescriptorW`, `FileContents`,
        `Preferred DropEffect`, `CanIncludeInClipboardHistory` and
        `CanUploadToCloudClipboard`. There is no `CF_HDROP` and no
        `Shell IDList Array`, so the client offers only virtual files.
    *   **Follow-up runs (owner, 2026-11-11):**
        *   **218 MiB file (buffered path):** done, exit code 0. Pasting
            again without `--overwrite` was refused.
        *   **Cancel during `--overwrite` at 8%:** exit code 2, "0/1 files
            completed". The original file was untouched (same size, the
            server's modified time kept) and no `.partial` was left.
        *   **1,118 MiB file (unbuffered `NO_BUFFERING` path, above
            `LARGE_FILE`):** done, exit code 0. The file checks out by
            eye.
        *   **Responsiveness:** the owner noticed no stutter while copying.
            G2 has not been measured with `typeperf` yet.
    *   **Still to test:**
        *   a session drop (CUJ 6);
        *   the Q6 cases;
        *   the G2 `typeperf` measurement;
        *   the Q1 baseline.
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
