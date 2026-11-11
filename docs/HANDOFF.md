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
*   **M1 conflicts and Q8:** merged in PRs #3 and #4.
    *   A paste that hits existing files asks Replace, Skip existing, or
        Keep both.
    *   `rdppaste` dies with the window through a job object.
*   **M2:** ADR 0005, tested by hand by the owner.
    *   **Folder sizes:** computed in the background by two workers, and
        cancelled on navigation.
    *   **Keys:** Ctrl+C, Ctrl+X, Ctrl+V, Delete, Shift+Delete and Enter.
        They work through `IFileOperation`, so pastes interoperate with
        Explorer.
    *   **Drag and drop on the window:** copies by default, Shift moves.
        It works with Explorer, NanaZip and 7-Zip.
    *   **NanaZip clipboard:** NanaZip's Ctrl+C puts only text on the
        clipboard, so drag and drop is the way in from NanaZip.
*   **Crates:**
    *   `files-core`: the rules, plus the `machine` line protocol;
    *   `files-win`: clipboard, I/O, `fileop` (IFileOperation), `drop`, `process` (job object), `shell` (context menu, open, dark
        menus, local time), and the test fake;
    *   `files-jobs`: the runner, the paste job, and folder sizes;
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

1.  **Done:**
    *   multiple selection (Ctrl+click, Shift+click, Ctrl+A);
    *   M3: a `Workspace` view with tabs, two panes, a sidebar of
        favourites and drives, and copy path (Ctrl+Alt+C or the toolbar
        button).
2.  **Layout lesson:** gpui-kit's `h_flex` centres children vertically.
    Use `flex().flex_row()` for rows that must stretch, plus `min_w_0` and
    `overflow_hidden` on panes.
3.  **M4 (branch `m4`), tested by hand by the owner:**
    *   **Settings:** `files_app::settings`, saved to
        `%APPDATA%\FilesProject\settings.txt` (accent, dark, density, lang).
    *   **Accent:** gpui-kit's colour picker in the title bar; hue and
        saturation drive the palette (`theme::apply`). The frame button
        follows the Windows accent colour (registry `DWM\AccentColor`),
        which Windows takes from the wallpaper when set to Automatic.
    *   **Density:** theme font size (16/14/13 px, scales rem paddings)
        plus the table's own sizes (Medium/Small/XSmall). `Size::Size(px)`
        drops cell text size and padding, so do not use it for the table.
    *   **Language:** `tr!("English", "Polski")` at each call site
        (`i18n.rs`); the default follows the Windows display language.
    *   **Title-bar lesson:** the title bar starts a window move on mouse
        down plus move, which swallowed button clicks. The button group
        stops mouse-down propagation.
    *   **No console window**, in debug builds too.
    *   **Backdrop:** GPUI's `Blurred` is the old
        `SetWindowCompositionAttribute` acrylic and lagged while dragging.
        The window now asks for `MicaBackdrop` and `files_win::shell::set_acrylic`
        switches it to Windows 11's DWM acrylic: smooth, and solid while the
        window is inactive (the owner is fine with that).
4.  **Next:** M5 (network drives), then M6 (CLI, updater, Explorer
    replacement). Later, a settings page with adjustable density steps.

## Open questions

These are from `docs/designs/files_project_m0.md`:

*   **Q1:** the freeze baseline in Explorer.
*   **Q2:** which RDP client and which server policy.
*   **Q4:** whether loopback RDP works.
*   **Q5:** answered in M1. The choices are replace, skip existing, or keep both (`name (2).ext`), through rdppaste exit code 5 and a bar in the window.
*   **Q6:** what happens when the session drops or the clipboard changes.
*   **Q7:** code signing.
*   **Q8:** answered in M1. The protocol is `files_core::machine` (tab-separated lines on stdout; closing stdin cancels). One paste runs at a time.
*   **Q8 job object:** done. `files_win::process::kill_with_parent` puts `rdppaste` in a kill-on-close job, so a crashed window ends the paste, and delete-on-close removes its `.partial`.

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
