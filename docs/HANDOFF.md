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
4.  **M5 (branch `m5`):** the sidebar's Drives heading has + (Windows'
    "Map network drive" dialog, `WNetConnectionDialog`) and - (disconnect
    dialog). Network drives show a network icon and their share, muted
    (`WNetGetConnectionW`). The dialogs are modal, so they run in a spawned
    task outside GPUI's dispatch.
    *   **Owner check:** works, on the home network too (2026-11-11).
    *   **Known limit:** the drive list is read at start and after those
        dialogs, not when a drive appears elsewhere (USB stick, Explorer).
5.  **M6 CLI (branch `m6-cli`), tested by hand by the owner:**
    *   **Open terminal here:** toolbar button or Ctrl+Alt+T (Ctrl+` does
        not arrive on a Polish layout). Starts Windows Terminal by its full
        alias path (`%LOCALAPPDATA%\Microsoft\WindowsApps\wt.exe`; a search
        by name misses Store aliases), else `pwsh`, else `powershell`.
    *   **One window:** `files <folder>` (also `files .`) hands the folder
        to the running window through a named pipe
        (`files_win::instance`, local clients only) and exits. The folder
        opens as a new tab.
    *   **Focus lesson:** the Explorer shortcuts work only while focus is in
        a pane, so title-bar changes refocus the active pane.
6.  **M6 updates (design: `docs/designs/updates.md`):**
    *   **Releases:** pushing a `v*` tag runs `release.yml`: tests, release
        build, a minisign signature per file (secret `MINISIGN_KEY` in the
        `release` environment, `v*` tags only), checked against
        `minisign.pub` before publishing. v0.1.0 published 2026-11-11.
    *   **Install:** `files --install` copies both programs to
        `%LOCALAPPDATA%\Programs\FilesProject` and adds it to the user PATH.
    *   **Update:** the installed copy checks the latest tag at start
        (`curl.exe`), shows "Update to vX", downloads, verifies each file's
        signature (`minisign-verify`), renames the running files to `.old`,
        moves the new ones in, and restarts with `--after-update`.
    *   **To release:** bump `version` in `crates/files-ui-gpui/Cargo.toml`
        (and `rdppaste`), merge, tag `vX.Y.Z`, push the tag.
7.  **UI polish (PR #14):** app icon (`assets/app.ico` from
    `assets/make_icon.py`, resource 1 via `embed-resource`), the logo in
    the title bar, file-type icons (Lucide SVGs in `assets/icons`, served by
    `src/assets.rs` before gpui-kit's), mouse back/forward.
8.  **M6 Explorer replacement (design: `docs/designs/replace_explorer.md`):**
    `files --replace-explorer` backs up and sets `Directory`, `Drive` and the
    Win+E verb in HKCU through `reg.exe`; `files --restore-explorer` (or
    double-clicking `restore-explorer.reg`) undoes it. Needs the installed
    copy. Shipped in v0.3.0; owner test pending.
9.  **Places, tabs and mouse (v0.4.0, tested by hand by the owner):**
    *   **This PC** (`shell:MyComputerFolder`): drive tiles with a usage
        bar, local and network apart (`this_pc_view`). Up from a drive root
        goes there.
    *   **Recycle Bin** (`shell:RecycleBinFolder`): read from the `$I`/`$R`
        files (`files_app::places`), Restore moves `$R` back, Delete removes
        both for good, "Empty Recycle Bin" is Windows' own.
    *   **Middle click:** a folder opens in a new tab (and switches to it); a
        tab closes.
    *   **Tabs remembered:** panes and tabs saved on exit to
        `session.txt`, reopened at start; `restore_tabs=false` in
        `settings.txt` turns it off.
    *   **Crash log:** panics go to
        `%LOCALAPPDATA%\FilesProject\logs\files.log` with a backtrace.
    *   **Fixed:** a crash when a tab's close button closed the last tab (the
        click also reached the tab).
    *   `--replace-explorer` now also takes This PC and the Recycle Bin;
        whether the desktop icons use that verb is still to test.
10. **Settings panel and window (v0.5.0, tested by hand by the owner):**
    *   **Settings** are GPUI global state (`src/prefs.rs`): every change
        applies at once and is saved. The panel (`src/settings_page.rs`,
        gpui-kit's `Settings` in a sheet, the gear in the title bar) has
        mode, accent (picker, "from Windows / wallpaper"), density and a
        text size per density (`fonts` in `settings.txt`), language,
        remembered tabs, and the Explorer replacement switch (installed copy
        only; `replace_explorer::is_on` = the backup folder exists).
    *   **Title bar:** favourite, split, light/dark, settings.
    *   **Window position, size and maximized state** are saved with the tabs
        in `session.txt` and restored at start.
    *   **Lesson:** closing the window (X, Alt+F4, taskbar) does not quit the
        app, so `on_app_quit` never ran; saving now also happens in
        `on_window_should_close` and the title bar's close handler.
11. **Everyday interactions (v0.6.0, tested by hand by the owner):**
    *   **Panes:** a handle sets the two panes' widths (`h_resizable`); a
        tab dragged onto a pane's files moves there, or splits one pane.
    *   **Files:** dragged onto a folder row, a pane's list or a sidebar
        entry (GPUI drag and drop, `DraggedFiles`); same drive moves, else
        copies, Ctrl copies, Shift moves; `fileop::transfer` runs
        `IFileOperation`. Inside the window only; dragging out to other apps
        is not done.
    *   **F2** renames in the row (Enter / click away / Esc); the context
        menu's Rename (asked with `CMF_CANRENAME`, `shell::Picked::Rename`)
        starts the same edit. `.lnk`/`.url` keep their hidden extension.
    *   **Folder background menu** (New, Paste, Properties):
        `shell::folder_menu` (`CreateViewObject`). The gpui-kit table takes
        every right-click, so the list listens in the capture phase and opens
        it when no row took the click.
    *   **Keys:** sort by header (folders first, kept across folders), type
        to jump, Backspace back, Ctrl+L / Alt+D / F4 address, Ctrl+Shift+N
        new folder (then renamed in place), Esc clears, Alt+Enter Properties,
        Ctrl+H hidden files.
    *   **Hidden and system files** (desktop.ini) are hidden unless
        `show_hidden` (settings, Ctrl+H).
    *   **Shortcuts:** `.lnk` targets read with `IShellLinkW`; folder
        shortcuts open here, with their own icons; names without `.lnk`/`.url`.
    *   **Windows' icons** for programs and shortcuts and a **hover preview**
        (photos, videos, PDFs through PowerToys) via `IShellItemImageFactory`
        (`shell::shell_image`), loaded off the UI thread.
12. **Address crumbs (v0.7.0, tested by hand by the owner):** the address is
    clickable levels (This PC › C: › ... › folder), like FilePilot: click to
    go, middle-click for a tab, drop files on a level, a tooltip with the
    full path. Clicking the empty space (or Ctrl+L / Alt+D / F4) shows the
    path to type; Enter goes, Esc or clicking away returns. Long paths hide
    levels after "This PC" once the names pass about 90 characters.
13. **v0.7.0 (2026-11-11, PR #20, tested by hand by the owner):**
    *   **Thumbnail grid** (Ctrl+Shift+G): tiles with thumbnails; arrows move
        by tile and by row (context `Grid`), Shift+click ranges.
    *   **Sidebar groups** (`files_app::favourites`, `[Name]` sections and
        `[*drives]` in favourites.txt): drag items and whole groups, empty
        groups take drops, new / rename / remove groups (gpui-kit context
        menu), folders dropped on a group join it.
    *   **Folder watching** (`files_win::watch`, FindFirstChangeNotification):
        reload at most once a second, keeping the selection and a rename in
        progress; not while search results are shown.
    *   **Search** (Ctrl+F): filters as you type; Enter walks subfolders in
        the background (5,000 results at most) with a Location column.
    *   **History menu:** right-click Back / Forward.
    *   **Columns:** Type (`shell::type_name`, cached per extension), chosen
        in settings (`columns`), dragged widths kept (`widths`).
    *   **Preview pane** (Alt+P): picture or the start of a text file, and
        details.
    *   **Command palette** (Ctrl+Shift+P, `src/palette.rs`): commands and
        recent / favourite folders; letters in order, Polish marks optional.
    *   **Bulk rename** (F2 on several items, `files_app::bulk`): `#` number,
        `{name}`; all or nothing, through temporary names, undone on failure.
    *   **Drag out:** a GPUI drag that reaches the window edge becomes
        Windows' own drag (`files_win::drag`, DoDragDrop; GPUI's Windows
        platform has no external drag).
    *   **Operations bar** (`src/ops.rs`): copies, moves and deletes in
        progress with a bar and Cancel (`fileop::OpProgress`, an
        `IFileOperationProgressSink`; cancel stops before the next item; no
        pause, the API has none).
    *   **CI** now runs on pull requests and main only (it ran twice per
        change, and Windows runners queue).
14. **Also in v0.7.0:** the "Paste from RDP" button is gone; Ctrl+V pastes
    both kinds (real paths through Windows, virtual files through rdppaste).
15. **v0.7.1 / v0.7.2 (owner test at work):** copying into the same folder
    duplicates (`FOF_RENAMEONCOLLISION` only then); F2 selects the name without
    its extension; Enter in a rename does not also open the item (commit
    deferred one frame); USB sticks: the drive list is compared each second
    (an eject keeps the letter), empty removable drives are hidden, USB icon,
    Windows' drive menu (Eject) on drives.
16. **Next:** the remaining M0 checks (CUJ 6, Q6, G2 typeperf, Q1) and
    whatever the owner finds at work.

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
