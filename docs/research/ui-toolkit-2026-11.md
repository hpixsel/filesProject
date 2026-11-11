# UI toolkit research (2026-11-11)

Input for the M1 UI-toolkit ADR. Desk research only. Nothing was built or
run locally, and every RAM figure comes from macOS or Linux measurements.
The M1 spike below has to confirm the choice on Windows.

## Key finding

The Windows integration works the same way in every candidate that exposes
the window's HWND, so it does not decide the choice:

*   **Drag and drop:** remove the toolkit's own handler with
    `RevokeDragDrop`, then call `RegisterDragDrop` with our own
    `IDropTarget`.
*   **Context menu:** call `SetWindowSubclass`, then forward the menu
    messages to `IContextMenu3::HandleMenuMsg2`.

The choice therefore comes down to four things: the look, RAM use, the list
widget, and how stable the API is.

## Comparison

| Toolkit | Look / accent colour | RAM (non-Windows measurements) | 100k-row list | Licence | Main risk |
| :-- | :-- | :-- | :-- | :-- | :-- |
| **GPUI + gpui-kit** (longbridge, 0.9.0, 2026-11) | Best: 75+ shadcn-style components, Table, VirtualList, docks, tabs, themes with sizes | ~125 MiB peak at 100k rows | Yes (`uniform_list`, virtual Table) | Apache-2.0 | No stable API: gpui-kit pins a Zed snapshot, while crates.io `gpui` is at 0.2.2. Docs are mostly source code |
| **eframe/egui** (0.34+) | Looks like a dev tool by default. Accent colour and density are easy to set | ~122 MiB peak; ~72 MB idle in one app | Yes (`show_rows`, `egui_extras` Table) | MIT/Apache | A modern look needs custom styling |
| **Slint** (1.17+) | Fluent and Material styles, but the accent colour is read-only on the standard widgets | ~118 MiB (FemtoVG); ~30 MB with the software renderer | Yes (`ListView`) | GPLv3, or royalty-free with a visible "uses Slint" notice | Attribution requirement and weak accent support |
| **iced** (0.14) | Clean, but few ready components | ~143 MB idle | No built-in virtual list | MIT | Highest RAM, slow releases |
| **windows-reactor** (Microsoft, windows-rs, preview) | Native WinUI 3 / Fluent, system accent colour | ~110 MB for a tiny app | Native virtualized ListView | MIT/Apache | Preview, git-only, needs the Windows App SDK runtime |
| Xilem, Freya, Makepad, Dioxus Native/Blitz | — | — | — | — | Experimental, alpha, or one maintainer |

## Recommendation

1.  **GPUI + gpui-kit (first choice).**
    *   It fits "modern and fast" best.
    *   It ships ready-made the components M3 and M4 would otherwise need:
        tabs, docks, a virtual Table, and themes with sizes.
    *   Zed shows it works on Windows, with a stable release on DirectX 11.
2.  **eframe/egui (fallback and control in the spike).** It is the most
    mature and stable option and needs the least code. Its weak point is
    the look.

Not chosen:

*   **Slint:** the attribution requirement, and weak support for a custom
    accent colour.
*   **iced:** highest RAM, and no virtual list.
*   **windows-reactor:** to watch. It is the only route to a truly native
    WinUI look, but it is still a preview.

## M1 spike (1–2 days, branch `spike/ui-gpui`)

Build the same five checks in GPUI + gpui-kit (day 1), then in egui (half
of day 2):

1.  **List scrolling:** a 100k-row virtual table. Fast scrolling, PageUp
    and PageDown, Home and End, with frame times measured.
2.  **Theme changes at runtime:** accent colour from a hex value,
    dark/light, and two densities. A change must reach the whole UI within
    one frame.
3.  **OLE drop:** take the HWND, call `RevokeDragDrop`, and register our
    own `IDropTarget`. It must log both `CF_HDROP` and `FileGroupDescriptorW`
    from Explorer.
4.  **Native context menu on a row:** "Send to" and "Open with" fill in,
    and the UI stays intact after the menu closes.
5.  **Measurements:**
    *   Working Set and Private Bytes at 30 s, after scrolling, and after
        60 s idle;
    *   cold start time;
    *   binary size.

**Pass criteria:**

*   idle Private Bytes under 150 MB;
*   no stutter;
*   drag and drop and the context menu work without hacks inside the
    toolkit.

If GPUI fails on RAM or on COM integration, choose egui. Record the result
as an ADR.

## Not verified

*   RAM on Windows, for every toolkit.
*   GPUI's behaviour after `RevokeDragDrop`, and the modal loop of
    `TrackPopupMenuEx`.
*   Whether gpui-kit can recompute the accent colour at runtime, and
    whether it supports i18n.
*   iced's HWND API.

## Sources

*   gpui-kit: https://github.com/longbridge/gpui-kit
*   GPUI `HasWindowHandle`: https://git.secluded.site/zed/commit/4270f89956b573512984c6bcbfb9544879381461
*   Zed on Windows: https://zed.dev/blog/windows-progress-report
*   100k-row bake-off (macOS): https://github.com/zed-industries/zed/discussions/63832
*   egui: https://newreleases.io/project/github/emilk/egui/release/0.34.0
*   egui vs iced RAM: https://www.michaelblaess.de/en/blog/egui-vs-iced/
*   egui → Slint RAM: https://trystan-sarrade.com/article/rust-gui-135mb-to-30mb-egui-to-slint/
*   Slint 1.17: https://slint.dev/blog/slint-1.17-released
*   Slint licence: https://github.com/slint-ui/slint/blob/master/LICENSE.md
*   Slint Palette: https://docs.slint.dev/latest/docs/slint/reference/std-widgets/globals/palette/
*   iced 0.14: https://github.com/iced-rs/iced/releases/tag/0.14.0
*   windows-reactor: https://github.com/microsoft/windows-rs/pull/4479, https://gist.github.com/kennykerr/1b52f354496042b138abd835277268dd
