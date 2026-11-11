//! The explorer pane: toolbar, address bar, file table, status bar.

use files_app::places::{self, Place};
use files_app::{format_size, list_dir, History, Item, SortBy};
use files_core::machine::{self, Event};
use files_core::Progress;
use files_jobs::sizes::{scan_sizes, SizeScan};
use files_win::shell::Picked;
use gpui_kit::component::button::*;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::table::*;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::{BTreeSet, HashMap};
use std::io::BufRead;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::i18n::{self, tr};
use crate::theme;

actions!(
    files,
    [
        GoBack,
        GoForward,
        GoUp,
        Refresh,
        PasteHere,
        CopySel,
        CutSel,
        DeleteSel,
        DeleteForever,
        OpenSel,
        CopyPath,
        SelectAll,
        OpenTerminal,
        Rename,
        FocusAddress,
        NewFolder,
        ClearSelection,
        Properties,
        ToggleHidden,
        ToggleGrid
    ]
);

const CONTEXT: &str = "Explorer";

/// A folder middle-clicked in the list: the workspace opens it as a
/// new tab.
pub struct OpenInNewTab(pub PathBuf);

impl EventEmitter<OpenInNewTab> for Explorer {}

/// Files dragged inside the window, and their drag image.
#[derive(Clone)]
pub struct DraggedFiles {
    paths: Vec<PathBuf>,
    /// The source list, reloaded once a move has taken them away.
    changed: Arc<AtomicBool>,
}

impl DraggedFiles {
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }
}

impl Render for DraggedFiles {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = match &self.paths[..] {
            [one] => one
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            many => tr!("{} items", "Elementów: {}", many.len()),
        };
        div()
            .px_3()
            .py_1()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_color(cx.theme().foreground)
            .child(label)
    }
}

/// Drops dragged files into `dest` like Explorer: a move on the same
/// drive, a copy to another; Ctrl copies, Shift moves. Windows shows
/// progress, asks about conflicts, and Ctrl+Z undoes it.
pub fn drop_files(
    d: &DraggedFiles,
    dest: &Path,
    window: &Window,
    dest_list: Option<Arc<AtomicBool>>,
) {
    // Not into itself, its own subfolder, or the folder it is already in.
    let paths: Vec<PathBuf> = d
        .paths
        .iter()
        .filter(|p| !dest.starts_with(p) && p.parent() != Some(dest))
        .cloned()
        .collect();
    let (Some(first), Some(h)) = (paths.first(), hwnd(window)) else {
        return;
    };
    if places::is_place(dest) {
        return;
    }
    let drive = |p: &Path| {
        p.components()
            .next()
            .map(|c| c.as_os_str().to_ascii_uppercase())
    };
    let m = window.modifiers();
    let mv = !m.control && (m.shift || drive(first) == drive(dest));
    let source = d.changed.clone();
    files_win::fileop::transfer(h, paths, dest.to_path_buf(), mv, move |r| {
        if let Err(e) = r {
            eprintln!("drop: {e}");
        }
        source.store(true, Ordering::SeqCst);
        if let Some(c) = dest_list {
            c.store(true, Ordering::SeqCst);
        }
    });
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("alt-left", GoBack, Some(CONTEXT)),
        KeyBinding::new("alt-right", GoForward, Some(CONTEXT)),
        KeyBinding::new("alt-up", GoUp, Some(CONTEXT)),
        KeyBinding::new("f5", Refresh, Some(CONTEXT)),
        KeyBinding::new("ctrl-v", PasteHere, Some(CONTEXT)),
        KeyBinding::new("ctrl-c", CopySel, Some(CONTEXT)),
        KeyBinding::new("ctrl-x", CutSel, Some(CONTEXT)),
        KeyBinding::new("delete", DeleteSel, Some(CONTEXT)),
        KeyBinding::new("shift-delete", DeleteForever, Some(CONTEXT)),
        KeyBinding::new("enter", OpenSel, Some(CONTEXT)),
        KeyBinding::new("ctrl-a", SelectAll, Some(CONTEXT)),
        // Ctrl+Shift+C is often taken globally (e.g. by ShareX), so Ctrl+Alt+C.
        KeyBinding::new("ctrl-alt-c", CopyPath, Some(CONTEXT)),
        // Ctrl+` does not arrive on a Polish keyboard layout.
        KeyBinding::new("ctrl-alt-t", OpenTerminal, Some(CONTEXT)),
        KeyBinding::new("f2", Rename, Some(CONTEXT)),
        // Explorer's keys: Backspace goes back, Ctrl+L/Alt+D/F4 to the address.
        KeyBinding::new("backspace", GoBack, Some(CONTEXT)),
        KeyBinding::new("ctrl-l", FocusAddress, Some(CONTEXT)),
        KeyBinding::new("alt-d", FocusAddress, Some(CONTEXT)),
        KeyBinding::new("f4", FocusAddress, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-n", NewFolder, Some(CONTEXT)),
        KeyBinding::new("escape", ClearSelection, Some(CONTEXT)),
        KeyBinding::new("alt-enter", Properties, Some(CONTEXT)),
        KeyBinding::new("ctrl-h", ToggleHidden, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-g", ToggleGrid, Some(CONTEXT)),
    ]);
}

pub fn hwnd(window: &Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

/// Runs a shell call that may start a modal message loop (menus, dialogs)
/// from a task, outside GPUI's event dispatch: inside it, the nested loop
/// re-enters GPUI while its state is borrowed and the app panics.
/// The items at `dir`: a folder, or This PC (drives with free space), or
/// the Recycle Bin. Runs off the UI thread; drives can be slow to answer.
fn list(dir: &Path, show_hidden: bool) -> std::io::Result<Vec<Item>> {
    let drives = files_win::shell::drives;
    Ok(match Place::of(dir) {
        Place::Folder => return list_dir(dir, show_hidden).map(with_links),
        Place::RecycleBin => places::recycle_bin(&drives()),
        Place::ThisPc => Vec::new(), // drive tiles instead, see `this_pc`
    })
}

/// `.lnk` shortcuts: their target, the name without `.lnk`, and a folder
/// when the target is one (so it opens here, not in Explorer).
fn with_links(mut items: Vec<Item>) -> Vec<Item> {
    for i in items.iter_mut().filter(|i| is_shortcut(&i.path)) {
        if let Some(stem) = i.path.file_stem() {
            i.name = stem.to_string_lossy().into_owned();
        }
        if let Some(target) = files_win::shell::link_target(&i.path) {
            i.is_dir = target.is_dir();
            i.link = Some(target);
        }
    }
    items
}

/// `.lnk` and `.url` shortcuts, shown without their extension.
fn is_shortcut(path: &Path) -> bool {
    has_ext(path, &["lnk", "url"])
}

/// Files that show Windows' own icon (a game, a program) instead of ours.
fn has_own_icon(path: &Path) -> bool {
    has_ext(
        path,
        &["lnk", "url", "exe", "appref-ms", "msc", "cpl", "scr"],
    )
}

fn has_ext(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Windows' picture for `path` (icon or thumbnail) as a GPUI image.
fn shell_picture(path: &Path, size: i32, thumbnail: bool) -> Option<Arc<RenderImage>> {
    let (w, h, bgra) = files_win::shell::shell_image(path, size, thumbnail)?;
    // GPUI images are BGRA, the order Windows gives.
    let frame = image::Frame::new(image::RgbaImage::from_raw(w, h, bgra)?);
    Some(Arc::new(RenderImage::new(vec![frame])))
}

/// A file's preview shown by the pointer.
struct Preview {
    path: PathBuf,
    image: Arc<RenderImage>,
    at: Point<Pixels>,
}

/// A drive tile in This PC.
#[derive(Clone)]
struct Drive {
    root: PathBuf,
    name: String,
    network: bool,
    /// (free, total) bytes; `None` when the drive does not answer.
    space: Option<(u64, u64)>,
}

/// The drives for This PC, local ones first. Off the UI thread: a network
/// drive can take seconds to answer.
fn this_pc() -> Vec<Drive> {
    let mut drives: Vec<Drive> = files_win::shell::drives()
        .into_iter()
        .map(|root| {
            let share = files_win::shell::network_path(&root);
            let label = share
                .as_deref()
                .and_then(|s| s.rsplit('\\').next().map(str::to_string))
                .or_else(|| files_win::shell::volume_label(&root))
                .unwrap_or_else(|| tr!("Local Disk", "Dysk lokalny"));
            let letter = root.display().to_string();
            Drive {
                name: format!("{label} ({})", letter.trim_end_matches('\\')),
                network: share.is_some(),
                space: files_win::io::disk_space(&root).ok(),
                root,
            }
        })
        .collect();
    drives.sort_by_key(|d| d.network);
    drives
}

/// Why `name` cannot be a file name in Windows, or `None` when it can.
fn bad_name(name: &str) -> Option<String> {
    if name.is_empty() || name == "." || name == ".." {
        return Some(tr!(
            "The name cannot be empty.",
            "Nazwa nie może być pusta."
        ));
    }
    if let Some(c) = name
        .chars()
        .find(|c| r#"\/:*?"<>|"#.contains(*c) || c.is_control())
    {
        return Some(tr!(
            "A name cannot contain {c}",
            "Nazwa nie może zawierać {c}"
        ));
    }
    if name.ends_with(['.', ' ']) {
        return Some(tr!(
            "A name cannot end with a dot or a space.",
            "Nazwa nie może kończyć się kropką ani spacją."
        ));
    }
    None
}

/// Icon file (in `assets/icons`) and hue for a file or folder, from its
/// name. A `None` hue keeps the default colour (accent folder, muted file);
/// `None` overall is the plain folder or file icon.
fn icon_kind(name: &str, is_dir: bool) -> Option<(&'static str, Option<f32>)> {
    if is_dir {
        return match name.to_ascii_lowercase().as_str() {
            "downloads" | "pobrane" => Some(("folder-down", None)),
            ".git" => Some(("folder-git-2", Some(15.))),
            n if n.starts_with('.') => Some(("folder-dot", None)),
            _ => None,
        };
    }
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    let (icon, hue) = match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico" | "heic" | "tif"
        | "tiff" | "psd" | "afphoto" | "afdesign" => ("file-image", 150.),
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "webm" => ("file-play", 280.),
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "wma" => ("file-music", 330.),
        "zip" | "7z" | "rar" | "tar" | "gz" | "xz" | "bz2" | "zst" | "cab" | "iso" | "msi" => {
            ("file-archive", 35.)
        }
        "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "c" | "h" | "cpp" | "hpp" | "cs" | "java"
        | "go" | "kt" | "swift" | "html" | "css" | "scss" | "json" | "xml" | "yaml" | "yml"
        | "toml" | "sql" | "lua" | "php" | "rb" => ("file-code", 195.),
        "exe" | "bat" | "cmd" | "ps1" | "sh" | "com" | "msix" | "appx" => ("file-terminal", 50.),
        "xlsx" | "xls" | "xlsm" | "csv" | "ods" => ("file-spreadsheet", 140.),
        "pdf" => ("file-text", 0.),
        "txt" | "md" | "log" | "doc" | "docx" | "rtf" | "odt" | "ppt" | "pptx" => {
            ("file-text", 217.)
        }
        "ttf" | "otf" | "woff" | "woff2" => ("file-type", 260.),
        "pem" | "crt" | "cer" | "key" | "pfx" | "p12" | "pub" | "kdbx" => ("file-key", 45.),
        "ini" | "cfg" | "conf" | "config" | "reg" | "env" | "lock" => {
            return Some(("file-cog", None))
        }
        _ => return None,
    };
    Some((icon, Some(hue)))
}

/// The table's icon for `item` and its colour.
fn item_icon(item: &Item, accent: Hsla, muted: Hsla) -> (Icon, Hsla) {
    let plain = if item.is_dir { accent } else { muted };
    if item.link.is_some() {
        let name = if item.is_dir {
            "folder-symlink"
        } else {
            "file-symlink"
        };
        return (Icon::empty().path(format!("icons/{name}.svg")), plain);
    }
    match icon_kind(&item.name, item.is_dir) {
        Some((name, hue)) => (
            Icon::empty().path(format!("icons/{name}.svg")),
            hue.map_or(plain, |h| hsla(h / 360., 0.7, 0.6, 1.)),
        ),
        None if item.is_dir => (Icon::new(IconName::Folder), plain),
        None => (Icon::new(IconName::File), plain),
    }
}

fn outside_dispatch(cx: &mut App, f: impl FnOnce() + 'static) {
    cx.spawn(async move |_: &mut AsyncApp| f()).detach();
}

pub struct Listing {
    items: Vec<Item>,
    /// Set after a shell command that may have changed the folder.
    changed: Arc<AtomicBool>,
    /// Folder sizes filled in by the background scan.
    sizes: Arc<Mutex<HashMap<PathBuf, u64>>>,
    /// Windows' own icons for programs and shortcuts, filled in the background.
    icons: Arc<Mutex<HashMap<PathBuf, Arc<RenderImage>>>>,
    /// Rows selected with Ctrl/Shift+click (or the single selected row).
    marked: BTreeSet<usize>,
    /// Where a Shift+click range starts.
    anchor: Option<usize>,
    /// This PC and the Recycle Bin show other columns.
    place: Place,
    /// The view that owns this list, which emits `OpenInNewTab`.
    explorer: WeakEntity<Explorer>,
    /// The row being renamed (F2) and its name field.
    renaming: Option<(PathBuf, Entity<InputState>)>,
    /// Set when a right-click hit a row, so the folder's menu stays shut.
    row_menu: Arc<AtomicBool>,
    /// The column order chosen in the header (kept across folders).
    sort: (SortBy, bool),
}

impl Listing {
    fn apply_sort(&mut self) {
        let sizes = self.sizes.lock().unwrap();
        let (by, descending) = self.sort;
        files_app::sort_items(&mut self.items, by, descending, |i| {
            i.size.or_else(|| sizes.get(&i.path).copied())
        });
    }

    fn marked_paths(&self) -> Vec<PathBuf> {
        self.marked
            .iter()
            .filter_map(|&i| self.items.get(i))
            .map(|i| i.path.clone())
            .collect()
    }
}

impl TableDelegate for Listing {
    fn columns_count(&self, _: &App) -> usize {
        if self.place == Place::RecycleBin {
            4
        } else {
            3
        }
    }
    fn rows_count(&self, _: &App) -> usize {
        self.items.len()
    }
    fn column(&self, col_ix: usize, _: &App) -> Column {
        // The header shows the order in use (sorting is ours, see perform_sort).
        let sort = |by: SortBy| match self.sort {
            (b, false) if b == by => ColumnSort::Ascending,
            (b, true) if b == by => ColumnSort::Descending,
            _ => ColumnSort::Default,
        };
        match col_ix {
            0 => Column::new("name", tr!("Name", "Nazwa"))
                .width(px(440.))
                .sort(sort(SortBy::Name)),
            1 => Column::new("size", tr!("Size", "Rozmiar"))
                .width(px(110.))
                .sort(sort(SortBy::Size)),
            2 if self.place == Place::RecycleBin => {
                Column::new("modified", tr!("Deleted", "Usunięto")).width(px(160.))
            }
            2 => Column::new("modified", tr!("Modified", "Zmodyfikowano"))
                .width(px(160.))
                .sort(sort(SortBy::Modified)),
            _ => Column::new("origin", tr!("Original location", "Oryginalna lokalizacja"))
                .width(px(320.)),
        }
    }
    /// A header click: that column ascending, descending, then back to the
    /// name. Folders stay first.
    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        let by = [SortBy::Name, SortBy::Size, SortBy::Modified][col_ix.min(2)];
        self.sort = match sort {
            ColumnSort::Ascending => (by, false),
            ColumnSort::Descending => (by, true),
            ColumnSort::Default => (SortBy::Name, false),
        };
        self.apply_sort();
        self.marked.clear();
        self.anchor = None;
        cx.notify();
    }
    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let marked = self.marked.contains(&row_ix) && self.marked.len() > 1;
        let row = div().id(("row", row_ix)).cursor_pointer();
        let row = if marked {
            row.bg(cx.theme().table_active)
        } else {
            row
        };
        // Dragging a selected row takes the whole selection.
        let dragged = if self.marked.contains(&row_ix) {
            self.marked_paths()
        } else {
            self.items
                .get(row_ix)
                .map(|i| vec![i.path.clone()])
                .unwrap_or_default()
        };
        let row = row.on_drag(
            DraggedFiles {
                paths: dragged,
                changed: self.changed.clone(),
            },
            |d, _, _, cx| cx.new(|_| d.clone()),
        );
        // A folder row takes dropped files.
        let row = match self.items.get(row_ix).filter(|i| i.is_dir) {
            Some(dir) => {
                let dest = dir.link.clone().unwrap_or_else(|| dir.path.clone());
                let list = self.changed.clone();
                let over = cx.theme().drag_border.opacity(0.25);
                row.drag_over::<DraggedFiles>(move |s, _, _, _| s.bg(over))
                    .on_drop(move |d: &DraggedFiles, window, cx| {
                        drop_files(d, &dest, window, Some(list.clone()));
                        cx.stop_propagation(); // not into the folder around it too
                    })
            }
            None => row,
        };
        let row = match self.items.get(row_ix) {
            Some(item) => {
                let (me, path) = (self.explorer.clone(), item.path.clone());
                row.on_hover(move |hovered, window, cx| {
                    let path = path.clone();
                    let _ = me.update(cx, |e, cx| e.hover_row(path, *hovered, window, cx));
                })
            }
            None => row,
        };
        let explorer = self.explorer.clone();
        let middle = self
            .items
            .get(row_ix)
            .filter(|i| i.is_dir)
            .map(|i| i.link.clone().unwrap_or_else(|| i.path.clone()));
        row.on_mouse_down(MouseButton::Middle, move |_, _, cx| {
            if let Some(dir) = middle.clone() {
                let _ = explorer.update(cx, |_, cx| cx.emit(OpenInNewTab(dir)));
            }
        })
        // Ctrl+click toggles a row, Shift+click selects a range; plain
        // clicks and arrow keys arrive as TableEvent::SelectRow.
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |t, e: &MouseDownEvent, _, cx| {
                let l = t.delegate_mut();
                if e.modifiers.shift {
                    let from = l.anchor.unwrap_or(row_ix);
                    l.marked = (from.min(row_ix)..=from.max(row_ix)).collect();
                } else if e.modifiers.control {
                    if !l.marked.remove(&row_ix) {
                        l.marked.insert(row_ix);
                    }
                    l.anchor = Some(row_ix);
                } else {
                    return;
                }
                cx.notify();
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |t, _: &MouseDownEvent, window, cx| {
                let l = t.delegate_mut();
                // Like Explorer: right-clicking outside the selection selects that row.
                if !l.marked.contains(&row_ix) {
                    l.marked = [row_ix].into();
                    l.anchor = Some(row_ix);
                }
                let paths = l.marked_paths();
                let (Some(h), false) = (hwnd(window), paths.is_empty()) else {
                    return;
                };
                l.row_menu.store(true, Ordering::SeqCst);
                let (changed, me) = (l.changed.clone(), l.explorer.clone());
                let handle = window.window_handle();
                cx.stop_propagation(); // not the folder's own menu as well
                cx.notify();
                // The menu runs its own message loop: outside GPUI's dispatch.
                cx.spawn(
                    async move |_, cx| match files_win::shell::context_menu(h, &paths) {
                        Ok(Picked::Ran) => changed.store(true, Ordering::SeqCst),
                        Ok(Picked::Rename) => {
                            let _ = cx.update_window(handle, |_, window, cx| {
                                let _ = me.update(cx, |e, cx| e.rename(&Rename, window, cx));
                            });
                        }
                        _ => {}
                    },
                )
                .detach();
            }),
        )
    }
    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(item) = self.items.get(row_ix) else {
            return div();
        };
        let muted = cx.theme().muted_foreground;
        match col_ix {
            0 => {
                let (icon, color) = item_icon(item, cx.theme().primary, muted);
                let own = self.icons.lock().unwrap().get(&item.path).cloned();
                let cell = div().h_flex().gap_2().child(match own {
                    Some(picture) => img(ImageSource::Render(picture))
                        .size_4()
                        .into_any_element(),
                    None => icon.text_color(color).into_any_element(),
                });
                match &self.renaming {
                    Some((path, input)) if *path == item.path => {
                        let me = self.explorer.clone();
                        let input_size = match theme::table_size() {
                            gpui_kit::component::Size::XSmall => gpui_kit::component::Size::XSmall,
                            _ => gpui_kit::component::Size::Small,
                        };
                        cell.child(
                            div()
                                .flex_1()
                                .on_key_down(move |e: &KeyDownEvent, window, cx| {
                                    if e.keystroke.key == "escape" {
                                        let _ = me
                                            .update(cx, |this, cx| this.cancel_rename(window, cx));
                                        cx.stop_propagation();
                                    }
                                })
                                // The table's text size, and pulled left by the
                                // field's padding and border so the name stays put.
                                .ml(-(input_size.input_px() + px(1.)))
                                .child(Input::new(input).with_size(input_size)),
                        )
                    }
                    _ => cell.child(item.name.clone()),
                }
            }
            1 => {
                let size = match item.size {
                    _ if item.is_dir && item.link.is_some() => String::new(),
                    Some(n) => format_size(n),
                    None => match self.sizes.lock().unwrap().get(&item.path) {
                        Some(&n) => format_size(n),
                        None => "…".to_string(), // still being scanned
                    },
                };
                div().text_color(muted).child(size)
            }
            2 => div().text_color(muted).child(
                item.modified
                    .map(files_win::shell::local_time)
                    .unwrap_or_default(),
            ),
            _ => div().text_color(muted).child(
                item.origin
                    .as_ref()
                    .map(|o| o.display().to_string())
                    .unwrap_or_default(),
            ),
        }
    }
}

/// State shared with the thread that reads the child `rdppaste`.
#[derive(Default)]
struct Paste {
    progress: Progress,
    result: Option<(i32, String)>,
    /// Dropping it closes the child's stdin, which the protocol treats as cancel.
    stdin: Option<ChildStdin>,
}

pub struct Explorer {
    history: History,
    table: Entity<TableState<Listing>>,
    address: Entity<InputState>,
    status: SharedString,
    /// Result of the last paste; survives folder reloads.
    notice: SharedString,
    paste: Option<Arc<Mutex<Paste>>>,
    /// Background folder-size scan of the current folder.
    scan: Option<SizeScan>,
    /// Set when a paste stopped because files already exist; asks what to do.
    conflict: Option<SharedString>,
    changed: Arc<AtomicBool>,
    /// Outcome of a file operation finished on another thread.
    thread_notice: Arc<Mutex<Option<String>>>,
    /// Language the column headers were built in.
    polish: bool,
    /// Whether this listing shows hidden files; reloads when the setting changes.
    shown_hidden: bool,
    /// This PC's drive tiles.
    drives: Vec<Drive>,
    /// A folder just made with Ctrl+Shift+N, renamed once the list shows it.
    pending_rename: Option<PathBuf>,
    /// The address shows as clickable crumbs; this is true while it is typed in.
    editing_address: bool,
    /// Watches the folder on disk; `dirty` is set when it changes.
    watch: Option<files_win::watch::Watch>,
    dirty: Arc<AtomicBool>,
    last_reload: std::time::Instant,
    /// The thumbnail view (large pictures) instead of the list.
    grid: bool,
    /// Thumbnails for the grid, filled in the background per folder.
    thumbs: Arc<Mutex<HashMap<PathBuf, Arc<RenderImage>>>>,
    /// The hover preview on screen, and a counter that drops stale ones.
    preview: Option<Preview>,
    hover_gen: usize,
    /// Letters typed to jump to a name, and when the last one came.
    typed: (String, std::time::Instant),
    focus: FocusHandle,
}

impl Explorer {
    pub fn new(start: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let changed = Arc::new(AtomicBool::new(false));
        let listing = Listing {
            items: Vec::new(),
            changed: changed.clone(),
            sizes: Default::default(),
            icons: Default::default(),
            marked: Default::default(),
            anchor: None,
            place: Place::Folder,
            explorer: cx.weak_entity(),
            renaming: None,
            row_menu: Default::default(),
            sort: Default::default(),
        };
        // Ctrl/Shift+click is ours (Listing::marked), not the table's single row.
        let table =
            cx.new(|cx| TableState::new(listing, window, cx).select_on_modifier_click(false));
        let address = cx.new(|cx| InputState::new(window, cx));

        cx.subscribe_in(&table, window, |this, _, ev: &TableEvent, window, cx| {
            match ev {
                TableEvent::DoubleClickedRow(ix) => this.open_row(*ix, window, cx),
                // A plain click or arrow key: the selection becomes this row.
                TableEvent::SelectRow(ix) => this.table.update(cx, |t, cx| {
                    let l = t.delegate_mut();
                    l.marked = [*ix].into();
                    l.anchor = Some(*ix);
                    cx.notify();
                }),
                _ => {}
            }
        })
        .detach();
        cx.subscribe_in(
            &address,
            window,
            |this, input, ev: &InputEvent, window, cx| {
                if let InputEvent::Blur = ev {
                    this.editing_address = false;
                    cx.notify();
                }
                if let InputEvent::PressEnter { .. } = ev {
                    let typed = PathBuf::from(input.read(cx).value().trim());
                    if typed.is_dir() || places::is_place(&typed) {
                        this.editing_address = false;
                        this.focus.focus(window, cx);
                        this.history.go(typed);
                        this.load(window, cx);
                    } else {
                        this.status = tr!(
                            "Not a folder: {}",
                            "To nie jest folder: {}",
                            typed.display()
                        )
                        .into();
                        cx.notify();
                    }
                }
            },
        )
        .detach();
        // Background threads (child reader, shell commands) cannot reach GPUI,
        // so poll their shared state a few times a second.
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            if this
                .update_in(cx, |this, window, cx| this.poll(window, cx))
                .is_err()
            {
                break;
            }
        })
        .detach();

        let mut this = Explorer {
            history: History::new(start),
            table,
            address,
            status: SharedString::default(),
            notice: SharedString::default(),
            paste: None,
            scan: None,
            conflict: None,
            changed,
            thread_notice: Default::default(),
            polish: i18n::polish(),
            shown_hidden: false,
            drives: Vec::new(),
            pending_rename: None,
            editing_address: false,
            watch: None,
            dirty: Default::default(),
            last_reload: std::time::Instant::now(),
            grid: false,
            thumbs: Default::default(),
            preview: None,
            hover_gen: 0,
            typed: (String::new(), std::time::Instant::now()),
            focus: cx.focus_handle(),
        };
        this.load(window, cx);
        this
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.history.current().to_path_buf();
        let shown = dir.display().to_string();
        self.address
            .update(cx, |a, cx| a.set_value(shown, window, cx));
        self.status = tr!("Loading…", "Wczytywanie…").into();
        cx.notify();
        let place = Place::of(&dir);
        self.last_reload = std::time::Instant::now();
        // Reloads when the folder changes on disk (folders only, not places).
        self.watch = (place == Place::Folder)
            .then(|| {
                let dirty = self.dirty.clone();
                files_win::watch::watch(&dir, move || dirty.store(true, Ordering::SeqCst)).ok()
            })
            .flatten();
        if place == Place::ThisPc {
            // The hidden table must not keep the last folder's selection, or
            // Delete here would act on files nobody can see.
            self.scan = None;
            self.table.update(cx, |t, cx| {
                let l = t.delegate_mut();
                l.items.clear();
                l.marked.clear();
                l.anchor = None;
                l.place = place;
                t.refresh(cx);
            });
            let task = cx.background_spawn(async move { this_pc() });
            cx.spawn_in(window, async move |this, cx| {
                let drives = task.await;
                let _ = this.update(cx, |this, cx| {
                    this.status = tr!("{} items", "Elementów: {}", drives.len()).into();
                    this.drives = drives;
                    cx.notify();
                });
            })
            .detach();
            return;
        }
        self.shown_hidden = crate::prefs::get(cx).show_hidden;
        let show_hidden = self.shown_hidden;
        let task = cx.background_spawn(async move { list(&dir, show_hidden) });
        cx.spawn_in(window, async move |this, cx| {
            let listed = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match listed {
                    Ok(items) => {
                        this.status = tr!("{} items", "Elementów: {}", items.len()).into();
                        // Drives and Recycle Bin items: no folder-size scan.
                        let dirs = items
                            .iter()
                            .filter(|i| i.is_dir && i.size.is_none() && i.link.is_none())
                            .map(|i| i.path.clone());
                        // Replacing the scan drops (cancels) the previous folder's one.
                        let scan = scan_sizes(dirs.collect());
                        let sizes = scan.results();
                        this.scan = Some(scan);
                        let wants: Vec<PathBuf> = items
                            .iter()
                            .filter(|i| has_own_icon(&i.path))
                            .map(|i| i.path.clone())
                            .collect();
                        let icons: Arc<Mutex<HashMap<PathBuf, Arc<RenderImage>>>> =
                            Default::default();
                        this.load_icons(wants, icons.clone(), window, cx);
                        if this.grid {
                            let files: Vec<PathBuf> = items
                                .iter()
                                .filter(|i| !i.is_dir)
                                .map(|i| i.path.clone())
                                .collect();
                            this.load_thumbs(files, window, cx);
                        }
                        this.table.update(cx, |t, cx| {
                            // A reload of the same folder keeps the selection.
                            let l = t.delegate();
                            let kept: BTreeSet<PathBuf> = l.marked_paths().into_iter().collect();
                            let current = t
                                .selected_row()
                                .and_then(|ix| l.items.get(ix))
                                .map(|i| i.path.clone());
                            let l = t.delegate_mut();
                            l.items = items;
                            l.place = place;
                            l.sizes = sizes;
                            l.icons = icons;
                            // A rename in progress survives a reload while its file is there.
                            if let Some((p, _)) = &l.renaming {
                                if !l.items.iter().any(|i| &i.path == p) {
                                    l.renaming = None;
                                }
                            }
                            l.anchor = None;
                            l.apply_sort();
                            l.marked = (0..l.items.len())
                                .filter(|&i| kept.contains(&l.items[i].path))
                                .collect();
                            let row =
                                current.and_then(|p| l.items.iter().position(|i| i.path == p));
                            t.refresh(cx);
                            match row {
                                Some(ix) => t.set_selected_row(ix, cx),
                                None => t.clear_selection(cx),
                            }
                        });
                        // Ctrl+Shift+N: the new folder's name is edited right away.
                        if let Some(new) = this.pending_rename.take() {
                            let items = &this.table.read(cx).delegate().items;
                            if let Some(ix) = items.iter().position(|i| i.path == new) {
                                this.table.update(cx, |t, cx| {
                                    t.set_selected_row(ix, cx);
                                    t.scroll_to_row(ix, cx);
                                });
                                this.rename(&Rename, window, cx);
                            }
                        }
                    }
                    Err(e) => {
                        this.status =
                            tr!("Cannot open folder: {e}", "Nie można otworzyć folderu: {e}").into()
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.table.read(cx).delegate().items.get(ix).cloned() else {
            return;
        };
        if item.is_dir {
            self.history.go(item.link.unwrap_or(item.path));
            self.load(window, cx);
        } else if let Some(h) = hwnd(window) {
            outside_dispatch(cx, move || {
                let _ = files_win::shell::open(h, &item.path);
            });
        }
    }

    fn go_back(&mut self, _: &GoBack, window: &mut Window, cx: &mut Context<Self>) {
        if self.history.back() {
            self.load(window, cx);
        }
    }
    fn go_forward(&mut self, _: &GoForward, window: &mut Window, cx: &mut Context<Self>) {
        if self.history.forward() {
            self.load(window, cx);
        }
    }
    fn go_up(&mut self, _: &GoUp, window: &mut Window, cx: &mut Context<Self>) {
        if self.history.up() {
            self.load(window, cx);
        }
    }
    fn refresh(&mut self, _: &Refresh, window: &mut Window, cx: &mut Context<Self>) {
        self.load(window, cx);
    }

    /// Ctrl+V: real paths (Explorer copy, NanaZip, our Ctrl+C/X) go to
    /// Windows' file engine; virtual files (RDP, zip contents) to rdppaste.
    fn paste_here(&mut self, _: &PasteHere, window: &mut Window, cx: &mut Context<Self>) {
        if !files_win::fileop::clipboard_has_paths() {
            self.start(None, cx);
            return;
        }
        let Some(h) = hwnd(window) else { return };
        let dir = self.history.current().to_path_buf();
        let report = self.report_done(tr!("Paste", "Wklejanie"));
        files_win::fileop::paste_paths(h, dir, report);
    }

    /// The selected items: the marked rows, or the table's current row.
    fn selected(&self, cx: &App) -> Vec<PathBuf> {
        let table = self.table.read(cx);
        let marked = table.delegate().marked_paths();
        if !marked.is_empty() {
            return marked;
        }
        let row = table
            .selected_row()
            .and_then(|ix| table.delegate().items.get(ix));
        row.map(|i| vec![i.path.clone()]).unwrap_or_default()
    }

    fn copy_selected(&mut self, cut: bool, cx: &mut Context<Self>) {
        let paths = self.selected(cx);
        if paths.is_empty() {
            return;
        }
        let result = files_win::Sta::new()
            .and_then(|sta| files_win::fileop::set_clipboard(&sta, &paths, cut));
        let what = match &paths[..] {
            [one] => one
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            many => tr!("{} items", "elementy: {}", many.len()),
        };
        self.notice = match result {
            Ok(()) if cut => tr!("Cut {what}", "Wycięto {what}").into(),
            Ok(()) => tr!("Copied {what}", "Skopiowano {what}").into(),
            Err(e) => tr!("Cannot copy: {e}", "Nie można skopiować: {e}").into(),
        };
        cx.notify();
    }

    fn copy_sel(&mut self, _: &CopySel, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selected(false, cx);
    }
    fn cut_sel(&mut self, _: &CutSel, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selected(true, cx);
    }

    fn delete_selected(&mut self, permanent: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (paths, Some(h)) = (self.selected(cx), hwnd(window)) else {
            return;
        };
        if paths.is_empty() {
            return;
        }
        let report = self.report_done(tr!("Delete", "Usuwanie"));
        if Place::of(self.history.current()) == Place::RecycleBin {
            // Out of the bin is for good; the $I file goes with its content.
            let paths = paths.iter().flat_map(|p| places::bin_files(p)).collect();
            return files_win::fileop::delete(h, paths, true, report);
        }
        files_win::fileop::delete(h, paths, permanent, report);
    }

    fn delete_sel(&mut self, _: &DeleteSel, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_selected(false, window, cx);
    }
    fn delete_forever(&mut self, _: &DeleteForever, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_selected(true, window, cx);
    }

    /// F2: the name in its row becomes editable; Enter or clicking away
    /// renames, Esc cancels.
    fn rename(&mut self, _: &Rename, window: &mut Window, cx: &mut Context<Self>) {
        let [path] = &self.selected(cx)[..] else {
            return; // one item at a time
        };
        if Place::of(self.history.current()) != Place::Folder {
            return;
        }
        let path = path.clone();
        // A shortcut's name is edited without `.lnk`/`.url`, which stays.
        let shown = if is_shortcut(&path) {
            path.file_stem()
        } else {
            path.file_name()
        };
        let name = shown
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.commit_rename(window, cx);
            }
        })
        .detach();
        input.update(cx, |i, cx| {
            i.focus(window, cx);
            i.select_all(window, cx);
        });
        self.table.update(cx, |t, cx| {
            t.delegate_mut().renaming = Some((path, input));
            cx.notify();
        });
    }

    /// Explorer's menu for the folder itself: New folder or file, Paste,
    /// Properties.
    fn folder_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.history.current().to_path_buf();
        let (Some(h), Place::Folder) = (hwnd(window), Place::of(&dir)) else {
            return;
        };
        let changed = self.changed.clone();
        outside_dispatch(cx, move || {
            if let Ok(Picked::Ran) = files_win::shell::folder_menu(h, &dir) {
                changed.store(true, Ordering::SeqCst);
            }
        });
    }

    fn focus_address(&mut self, _: &FocusAddress, window: &mut Window, cx: &mut Context<Self>) {
        self.editing_address = true;
        cx.notify();
        self.address.update(cx, |a, cx| {
            a.focus(window, cx);
            a.select_all(window, cx);
        });
    }

    /// Ctrl+Shift+N: "New folder" (numbered if taken), then its name is
    /// edited in place.
    fn new_folder(&mut self, _: &NewFolder, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.history.current().to_path_buf();
        if Place::of(&dir) != Place::Folder {
            return;
        }
        let base = dir.join(tr!("New folder", "Nowy folder"));
        let task = cx.background_spawn(async move {
            let free = (1..1000)
                .map(|n| {
                    if n == 1 {
                        base.clone()
                    } else {
                        files_core::numbered(&base, n)
                    }
                })
                .find(|p| !p.exists())
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::AlreadyExists))?;
            std::fs::create_dir(&free).map(|()| free)
        });
        cx.spawn_in(window, async move |this, cx| {
            let made = task.await;
            let _ = this.update_in(cx, |this, window, cx| match made {
                Ok(path) => {
                    this.pending_rename = Some(path);
                    this.load(window, cx);
                }
                Err(e) => {
                    this.notice = tr!(
                        "Cannot create a folder: {e}",
                        "Nie można utworzyć folderu: {e}"
                    )
                    .into();
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Esc: no selection (or, while renaming, the old name stays).
    fn clear_selection(&mut self, _: &ClearSelection, window: &mut Window, cx: &mut Context<Self>) {
        if self.table.read(cx).delegate().renaming.is_some() {
            return self.cancel_rename(window, cx);
        }
        self.table.update(cx, |t, cx| {
            t.delegate_mut().marked.clear();
            t.delegate_mut().anchor = None;
            t.clear_selection(cx);
        });
    }

    /// Alt+Enter: Windows' Properties for the selection, or the folder.
    fn properties(&mut self, _: &Properties, window: &mut Window, cx: &mut Context<Self>) {
        let mut paths = self.selected(cx);
        if paths.is_empty() && Place::of(self.history.current()) == Place::Folder {
            paths.push(self.history.current().to_path_buf());
        }
        let (Some(path), Some(h)) = (paths.into_iter().next(), hwnd(window)) else {
            return;
        };
        outside_dispatch(cx, move || files_win::shell::properties(h, &path));
    }

    /// Typing letters in the list jumps to the first name starting with
    /// them, like Explorer; a pause of a second starts over.
    fn type_to_select(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = e.keystroke.modifiers;
        let Some(ch) = e.keystroke.key_char.clone() else {
            return;
        };
        let typing_elsewhere = self
            .address
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx)
            || self.table.read(cx).delegate().renaming.is_some();
        if m.control || m.alt || m.platform || typing_elsewhere || ch.trim().is_empty() {
            return;
        }
        let (typed, at) = &mut self.typed;
        if at.elapsed() > Duration::from_secs(1) {
            typed.clear();
        }
        typed.push_str(&ch.to_lowercase());
        *at = std::time::Instant::now();
        let prefix = typed.clone();
        self.table.update(cx, |t, cx| {
            let items = &t.delegate().items;
            if let Some(ix) = items
                .iter()
                .position(|i| i.name.to_lowercase().starts_with(&prefix))
            {
                t.set_selected_row(ix, cx);
                t.scroll_to_row(ix, cx);
            }
        });
    }

    /// Windows' icons for `paths`, a few at a time off the UI thread, each
    /// batch shown as it arrives. A newer folder has its own `icons`.
    /// The list or the thumbnail grid (for photos).
    fn toggle_grid(&mut self, _: &ToggleGrid, window: &mut Window, cx: &mut Context<Self>) {
        self.grid = !self.grid;
        if self.grid {
            let files: Vec<PathBuf> = self
                .table
                .read(cx)
                .delegate()
                .items
                .iter()
                .filter(|i| !i.is_dir)
                .map(|i| i.path.clone())
                .collect();
            self.load_thumbs(files, window, cx);
        }
        cx.notify();
    }

    /// Thumbnails (photos, videos, PDFs) for the grid, a few at a time off
    /// the UI thread, shown as they arrive. A newer folder starts a new map.
    fn load_thumbs(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let thumbs: Arc<Mutex<HashMap<PathBuf, Arc<RenderImage>>>> = Default::default();
        self.thumbs = thumbs.clone();
        cx.spawn_in(window, async move |this, cx| {
            for chunk in paths.chunks(8) {
                let chunk = chunk.to_vec();
                let found = cx
                    .background_spawn(async move {
                        chunk
                            .into_iter()
                            .filter_map(|p| {
                                let pic = shell_picture(&p, 256, true).or_else(|| {
                                    has_own_icon(&p)
                                        .then(|| shell_picture(&p, 96, false))
                                        .flatten()
                                });
                                pic.map(|i| (p, i))
                            })
                            .collect::<Vec<_>>()
                    })
                    .await;
                thumbs.lock().unwrap().extend(found);
                // Stop once another folder (or the list view) took over.
                let alive = this.update(cx, |this, cx| {
                    cx.notify();
                    this.grid && Arc::ptr_eq(&this.thumbs, &thumbs)
                });
                if !matches!(alive, Ok(true)) {
                    return;
                }
            }
        })
        .detach();
    }

    /// The folder as large tiles: thumbnails for photos and videos, icons
    /// for the rest. Click selects (Ctrl+click adds), double-click opens,
    /// right-click is Explorer's menu, middle-click opens a folder in a tab.
    fn grid_view(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        // shortcut: every tile is laid out (no virtual scrolling); fine for
        // photo folders, slow past a few thousand items. Virtualize then.
        let t = cx.theme();
        let (muted, hover, selected, accent, radius) = (
            t.muted_foreground,
            t.table_hover,
            t.table_active,
            t.primary,
            t.radius,
        );
        let table = self.table.read(cx);
        let l = table.delegate();
        let marked: BTreeSet<usize> = if l.marked.is_empty() {
            table.selected_row().into_iter().collect()
        } else {
            l.marked.clone()
        };
        let thumbs = self.thumbs.lock().unwrap();
        let icons = l.icons.lock().unwrap();
        let mut grid = div()
            .id("grid")
            .size_full()
            .overflow_y_scroll()
            .p_2()
            .flex()
            .flex_row()
            .flex_wrap()
            .content_start()
            .gap_2();
        for (ix, item) in l.items.iter().enumerate() {
            let picture = thumbs
                .get(&item.path)
                .or_else(|| icons.get(&item.path))
                .cloned();
            let (icon, color) = item_icon(item, accent, muted);
            let face = match picture {
                Some(p) => img(ImageSource::Render(p))
                    .max_w(px(128.))
                    .max_h(px(128.))
                    .object_fit(ObjectFit::Contain)
                    .into_any_element(),
                None => icon.size_16().text_color(color).into_any_element(),
            };
            let (open_dir, menu_paths) = (
                item.is_dir
                    .then(|| item.link.clone().unwrap_or_else(|| item.path.clone())),
                item.path.clone(),
            );
            grid = grid.child(
                div()
                    .id(("tile", ix))
                    .w(px(144.))
                    .p_2()
                    .v_flex()
                    .items_center()
                    .gap_1()
                    .rounded(radius)
                    .cursor_pointer()
                    .when(marked.contains(&ix), |d| d.bg(selected))
                    .hover(move |s| s.bg(hover))
                    .child(
                        div()
                            .size(px(128.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(face),
                    )
                    .child(
                        div()
                            .w_full()
                            .text_center()
                            .text_sm()
                            .truncate()
                            .child(item.name.clone()),
                    )
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        if e.click_count() >= 2 {
                            return this.open_row(ix, window, cx);
                        }
                        let ctrl = e.modifiers().control;
                        this.table.update(cx, |t, cx| {
                            let l = t.delegate_mut();
                            if ctrl {
                                if !l.marked.remove(&ix) {
                                    l.marked.insert(ix);
                                }
                            } else {
                                l.marked = [ix].into();
                            }
                            l.anchor = Some(ix);
                            t.set_selected_row(ix, cx);
                        });
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _, window, cx| {
                            let paths = {
                                let l = this.table.read(cx).delegate();
                                if l.marked.contains(&ix) {
                                    l.marked_paths()
                                } else {
                                    vec![menu_paths.clone()]
                                }
                            };
                            this.table.update(cx, |t, _| {
                                let l = t.delegate_mut();
                                if !l.marked.contains(&ix) {
                                    l.marked = [ix].into();
                                }
                                l.row_menu.store(true, Ordering::SeqCst);
                            });
                            let Some(h) = hwnd(window) else { return };
                            let changed = this.changed.clone();
                            outside_dispatch(cx, move || {
                                if let Ok(Picked::Ran) = files_win::shell::context_menu(h, &paths) {
                                    changed.store(true, Ordering::SeqCst);
                                }
                            });
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |_, _, _, cx| {
                            if let Some(dir) = open_dir.clone() {
                                cx.emit(OpenInNewTab(dir));
                            }
                        }),
                    ),
            );
        }
        grid
    }

    fn load_icons(
        &mut self,
        paths: Vec<PathBuf>,
        icons: Arc<Mutex<HashMap<PathBuf, Arc<RenderImage>>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            for chunk in paths.chunks(16) {
                let chunk = chunk.to_vec();
                let found = cx
                    .background_spawn(async move {
                        chunk
                            .into_iter()
                            .filter_map(|p| shell_picture(&p, 32, false).map(|i| (p, i)))
                            .collect::<Vec<_>>()
                    })
                    .await;
                icons.lock().unwrap().extend(found);
                let alive = this.update(cx, |this, cx| this.table.update(cx, |_, cx| cx.notify()));
                if alive.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    /// The pointer rests on a row: after a moment, Windows' preview of the
    /// file (a photo, a PDF page, a video frame) shows next to it.
    fn hover_row(
        &mut self,
        path: PathBuf,
        hovered: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hover_gen += 1;
        if !hovered {
            if self.preview.as_ref().is_some_and(|p| p.path == path) {
                self.preview = None;
                cx.notify();
            }
            return;
        }
        let file = self
            .table
            .read(cx)
            .delegate()
            .items
            .iter()
            .any(|i| i.path == path && !i.is_dir);
        if !file || is_shortcut(&path) || Place::of(self.history.current()) != Place::Folder {
            return;
        }
        let (gen, at) = (self.hover_gen, window.mouse_position());
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            let still = this
                .update(cx, |this, _| this.hover_gen == gen)
                .unwrap_or(false);
            if !still {
                return;
            }
            let p = path.clone();
            let picture = cx
                .background_spawn(async move { shell_picture(&p, 320, true) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let (true, Some(image)) = (this.hover_gen == gen, picture) {
                    this.preview = Some(Preview { path, image, at });
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The path as clickable levels, like FilePilot: This PC › C: › Users ›
    /// ... Click a level to go there, middle-click for a new tab, drop files
    /// on it; click the empty space on the right to type a path.
    fn crumbs(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = cx.theme();
        let (muted, fg, hover, border, radius) = (
            t.muted_foreground,
            t.foreground,
            t.table_hover,
            t.input,
            t.radius,
        );
        let mut levels: Vec<(String, PathBuf)> = Vec::new();
        let here = self.history.current();
        match Place::of(here) {
            Place::ThisPc => levels.push((tr!("This PC", "Ten komputer"), here.into())),
            Place::RecycleBin => levels.push((tr!("Recycle Bin", "Kosz"), here.into())),
            Place::Folder => {
                levels.push((tr!("This PC", "Ten komputer"), places::THIS_PC.into()));
                let mut dirs: Vec<&Path> = here.ancestors().collect();
                dirs.reverse();
                for d in dirs {
                    let name = match d.file_name() {
                        Some(n) => n.to_string_lossy().into_owned(),
                        // A drive (C:) or a share (\\server\share).
                        None => d.display().to_string().trim_end_matches('\\').to_string(),
                    };
                    levels.push((name, d.to_path_buf()));
                }
            }
        }
        // A long path hides levels after "This PC", oldest first, until the
        // rest fits about 90 characters (shortcut: a character budget, not
        // the bar's width; measure the text if it misjudges).
        let len =
            |l: &[(String, PathBuf)]| l.iter().map(|(n, _)| n.chars().count() + 3).sum::<usize>();
        let mut hidden = 0;
        while levels.len() - hidden > 3 && len(&levels[hidden + 1..]) + 5 > 90 {
            hidden += 1;
        }
        let last = levels.len() - 1;
        let mut bar = div()
            .id("crumbs")
            .flex_1()
            .h_8()
            .px_1()
            .flex()
            .flex_row()
            .items_center()
            .gap_0p5()
            .overflow_hidden()
            .rounded(radius)
            .border_1()
            .border_color(border)
            .cursor_text()
            .on_click(
                cx.listener(|this, _, window, cx| this.focus_address(&FocusAddress, window, cx)),
            );
        for (ix, (name, path)) in levels.into_iter().enumerate() {
            if ix > 0 && ix <= hidden {
                if ix == 1 {
                    bar = bar.child(div().px_1().text_color(muted).child("…"));
                }
                continue;
            }
            if ix > 0 {
                bar = bar.child(Icon::new(IconName::ChevronRight).xsmall().text_color(muted));
            }
            let (go, tab, drop_to) = (path.clone(), path.clone(), path.clone());
            bar = bar.child(
                div()
                    .id(("crumb", ix))
                    .flex_shrink_0()
                    .px_2()
                    .py_0p5()
                    .rounded(radius)
                    .cursor_pointer()
                    .text_color(if ix == last { fg } else { muted })
                    .hover(move |s| s.bg(hover).text_color(fg))
                    .tooltip({
                        let full = path.display().to_string();
                        move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(full.clone())
                                .build(window, cx)
                        }
                    })
                    .drag_over::<DraggedFiles>(move |s, _, _, _| s.bg(hover))
                    .on_drop(move |d: &DraggedFiles, window, _| {
                        drop_files(d, &drop_to, window, None)
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation(); // not the "type a path" click around it
                        this.go_to(go.clone(), window, cx);
                    }))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |_, _, _, cx| cx.emit(OpenInNewTab(tab.clone()))),
                    )
                    .child(name),
            );
        }
        bar
    }

    /// Esc while renaming: the old name stays.
    fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.table.update(cx, |t, cx| {
            t.delegate_mut().renaming = None;
            cx.notify();
        });
        self.focus.focus(window, cx);
    }

    /// Checks the new name, then renames off the UI thread (a network
    /// share can be slow) and reloads the folder. A bad name keeps the row
    /// editable and says why.
    fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((path, input)) = self
            .table
            .update(cx, |t, _| t.delegate_mut().renaming.take())
        else {
            return; // Enter, then the blur it causes: renamed once
        };
        let new = input.read(cx).value().trim().to_string();
        if let Some(why) = bad_name(&new) {
            self.notice = why.into();
            self.table.update(cx, |t, cx| {
                t.delegate_mut().renaming = Some((path, input.clone()));
                cx.notify();
            });
            input.update(cx, |i, cx| i.focus(window, cx));
            return;
        }
        self.table.update(cx, |_, cx| cx.notify());
        self.focus.focus(window, cx);
        // A shortcut's hidden extension (.lnk, .url) stays.
        let new = match path.extension().filter(|_| is_shortcut(&path)) {
            Some(ext) => format!("{new}.{}", ext.to_string_lossy()),
            None => new,
        };
        if path.file_name().is_some_and(|n| n.to_string_lossy() == new) {
            return;
        }
        let (from, to) = (path.clone(), path.with_file_name(&new));
        let task = cx.background_spawn(async move {
            // Renaming to another case of the same name is allowed.
            let same = from.to_string_lossy().to_lowercase() == to.to_string_lossy().to_lowercase();
            if to.exists() && !same {
                return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
            }
            std::fs::rename(&from, &to)
        });
        cx.spawn_in(window, async move |this, cx| {
            let r = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.notice = match r {
                    Ok(()) => tr!("Renamed to {new}", "Zmieniono nazwę na {new}").into(),
                    Err(e) => tr!("Cannot rename: {e}", "Nie można zmienić nazwy: {e}").into(),
                };
                this.load(window, cx);
            });
        })
        .detach();
    }

    /// This PC like Explorer's: drive tiles with a usage bar, local drives
    /// and network locations apart.
    fn this_pc_view(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = cx.theme();
        let (muted, primary, hover, track, radius) = (
            t.muted_foreground,
            t.primary,
            t.table_hover,
            t.muted,
            t.radius,
        );
        let danger = t.danger;
        let tile = |ix: usize, d: &Drive, cx: &mut Context<Self>| {
            let (root, middle) = (d.root.clone(), d.root.clone());
            let caption = match d.space {
                Some((free, total)) => tr!(
                    "{} free of {}",
                    "{} wolne z {}",
                    format_size(free),
                    format_size(total)
                ),
                None => tr!("Not available", "Niedostępny"),
            };
            let bar = d
                .space
                .filter(|&(_, total)| total > 0)
                .map(|(free, total)| {
                    let used = 1. - free as f32 / total as f32;
                    // Red when nearly full, like Explorer.
                    let fill = if used > 0.9 { danger } else { primary };
                    div()
                        .h(px(6.))
                        .w_full()
                        .rounded_full()
                        .bg(track)
                        .child(div().h_full().rounded_full().bg(fill).w(relative(used)))
                });
            let icon = if d.network {
                IconName::Network
            } else {
                IconName::HardDrive
            };
            div()
                .id(("drive", ix))
                .w(px(260.))
                .p_2()
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .rounded(radius)
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(
                    Icon::new(icon)
                        .with_size(gpui_kit::component::Size::Large)
                        .text_color(if d.space.is_some() { primary } else { muted }),
                )
                .child(
                    div()
                        .v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(div().truncate().child(d.name.clone()))
                        .children(bar)
                        .child(div().text_xs().text_color(muted).child(caption)),
                )
                .on_click(cx.listener(move |this, _, w, cx| this.go_to(root.clone(), w, cx)))
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(move |_, _, _, cx| cx.emit(OpenInNewTab(middle.clone()))),
                )
        };
        let mut view = div()
            .id("this-pc")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .v_flex()
            .gap_1();
        for (network, title) in [
            (false, tr!("Devices and drives", "Urządzenia i dyski")),
            (true, tr!("Network locations", "Lokalizacje sieciowe")),
        ] {
            let mut row = div().flex().flex_row().flex_wrap().gap_2();
            let mut any = false;
            for (ix, d) in self
                .drives
                .iter()
                .enumerate()
                .filter(|(_, d)| d.network == network)
            {
                row = row.child(tile(ix, d, cx));
                any = true;
            }
            if any {
                view = view
                    .child(div().pt_2().text_sm().text_color(muted).child(title))
                    .child(row);
            }
        }
        view
    }

    /// Recycle Bin: puts the selected items back where they were deleted from.
    fn restore_sel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let failed: Vec<String> = self
            .selected(cx)
            .iter()
            .filter_map(|p| places::restore(p).err().map(|e| e.to_string()))
            .collect();
        self.notice = match failed.first() {
            None => tr!("Restored", "Przywrócono").into(),
            Some(e) => tr!("Not restored: {e}", "Nie przywrócono: {e}").into(),
        };
        self.load(window, cx);
    }

    /// Windows' own "Empty Recycle Bin" (it asks first).
    fn empty_bin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(h) = hwnd(window) else {
            return;
        };
        let changed = self.changed.clone();
        outside_dispatch(cx, move || {
            if files_win::shell::empty_recycle_bin(h) {
                changed.store(true, Ordering::SeqCst);
            }
        });
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.table.update(cx, |t, cx| {
            let l = t.delegate_mut();
            l.marked = (0..l.items.len()).collect();
            cx.notify();
        });
    }

    fn open_sel(&mut self, _: &OpenSel, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.table.read(cx).selected_row() {
            self.open_row(ix, window, cx);
        }
    }

    /// Callback for a file operation running on its own thread: records the
    /// outcome and asks the next poll to reload the folder.
    pub fn report_done(&self, what: String) -> impl Fn(files_win::Result<()>) + Send + 'static {
        let (changed, notice) = (self.changed.clone(), self.thread_notice.clone());
        move |r| {
            // A cancelled Windows dialog returns an error too; say so plainly.
            let text = match r {
                Ok(()) => tr!("{what} done", "{what}: gotowe"),
                Err(e) => tr!("{what} did not finish: {e}", "{what}: nie ukończono: {e}"),
            };
            *notice.lock().unwrap() = Some(text);
            changed.store(true, Ordering::SeqCst);
        }
    }

    /// `flag` is the conflict choice (`--overwrite`, `--skip`, `--rename`),
    /// or `None` for the first attempt, which stops if files already exist.
    fn start(&mut self, flag: Option<&'static str>, cx: &mut Context<Self>) {
        self.conflict = None;
        if self.paste.is_some() {
            self.notice = tr!("A paste is already running.", "Wklejanie już trwa.").into();
            cx.notify();
            return;
        }
        match start_paste(self.history.current(), flag) {
            Ok(state) => {
                self.paste = Some(state);
                self.notice = tr!("Pasting…", "Wklejanie…").into();
            }
            Err(e) => {
                self.notice = tr!(
                    "Cannot start rdppaste: {e}",
                    "Nie można uruchomić rdppaste: {e}"
                )
                .into()
            }
        }
        cx.notify();
    }

    fn cancel_paste(&mut self) {
        if let Some(p) = &self.paste {
            p.lock().unwrap().stdin.take();
        }
    }

    fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.changed.swap(false, Ordering::SeqCst) {
            self.load(window, cx);
        }
        // The folder changed on disk: reload, at most once a second, so a
        // copy of thousands of files does not reload thousands of times.
        if self.last_reload.elapsed() > Duration::from_secs(1)
            && self.dirty.swap(false, Ordering::SeqCst)
        {
            self.load(window, cx);
        }
        if let Some(text) = self.thread_notice.lock().unwrap().take() {
            self.notice = text.into();
        }
        if self.scan.is_some() {
            // Repaint the size column as folder sizes arrive.
            self.table.update(cx, |_, cx| cx.notify());
        }
        let Some(paste) = self.paste.clone() else {
            return;
        };
        let done = paste.lock().unwrap().result.clone();
        if let Some((code, message)) = done {
            self.paste = None;
            // Kept in `notice`, which the folder reload below does not touch.
            self.notice = match code {
                0 => message.into(),
                // e.g. NanaZip/7-Zip or a plain Explorer copy put real paths
                // (CF_HDROP) on the clipboard, not virtual files.
                4 if message.contains("CF_HDROP") => tr!("These are regular files, not files from an RDP session or a zip.", "To zwykłe pliki, nie pliki z sesji RDP ani z archiwum zip.").into(),
                4 => tr!("The clipboard holds no files. Copy files (Ctrl+C) in Explorer or in the RDP session first.", "Schowek nie zawiera plików. Najpierw skopiuj pliki (Ctrl+C) w Eksploratorze lub w sesji RDP.").into(),
                // rdppaste exit code 5: files already exist, nothing written.
                5 => {
                    self.conflict = Some(message.into());
                    SharedString::default()
                }
                _ => tr!("Paste failed: {message}", "Wklejanie nie powiodło się: {message}").into(),
            };
            if code == 0 {
                self.load(window, cx);
            }
        }
        cx.notify();
    }

    /// Windows Terminal in this folder (its default profile), else
    /// PowerShell 7, else Windows PowerShell.
    fn open_terminal(&mut self, _: &OpenTerminal, _: &mut Window, cx: &mut Context<Self>) {
        const CREATE_NEW_CONSOLE: u32 = 0x10;
        let dir = self.history.current();
        // wt.exe is a Store app alias, which a search by name does not find.
        let wt = std::env::var_os("LOCALAPPDATA")
            .map(|d| PathBuf::from(d).join(r"Microsoft\WindowsApps\wt.exe"))
            .unwrap_or_else(|| "wt".into());
        let console = |exe: &str| {
            Command::new(exe)
                .current_dir(dir)
                .creation_flags(CREATE_NEW_CONSOLE)
                .spawn()
        };
        // `-d .` plus current_dir: wt misreads a quoted path ending in `\`, like `C:\`.
        let started = Command::new(wt)
            .args(["-d", "."])
            .current_dir(dir)
            .spawn()
            .or_else(|_| console("pwsh"))
            .or_else(|_| console("powershell"));
        if let Err(e) = started {
            self.notice = tr!(
                "Cannot open a terminal: {e}",
                "Nie można otworzyć terminala: {e}"
            )
            .into();
            cx.notify();
        }
    }

    pub fn current_dir(&self) -> &Path {
        self.history.current()
    }

    /// Tab title: the folder name, or the drive for a root.
    pub fn title(&self) -> String {
        let dir = self.history.current();
        match Place::of(dir) {
            Place::ThisPc => return tr!("This PC", "Ten komputer"),
            Place::RecycleBin => return tr!("Recycle Bin", "Kosz"),
            Place::Folder => {}
        }
        dir.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| dir.display().to_string())
    }

    pub fn go_to(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.history.go(dir);
        self.load(window, cx);
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// Where a drop's outcome goes: it reloads this tab and shows the result.
    pub fn drop_sink(&self) -> impl Fn(files_win::Result<()>) + 'static {
        self.report_done(tr!("Drop", "Upuszczanie"))
    }

    /// Ctrl+Alt+C: the selected paths (or the folder) as plain text, one
    /// per line, without the quotes Windows' "Copy as path" adds.
    fn copy_path(&mut self, _: &CopyPath, _: &mut Window, cx: &mut Context<Self>) {
        let mut paths = self.selected(cx);
        if paths.is_empty() {
            paths.push(self.history.current().to_path_buf());
        }
        let text: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        cx.write_to_clipboard(ClipboardItem::new_string(text.join("\r\n")));
        self.notice = match &paths[..] {
            [one] => tr!("Copied path {}", "Skopiowano ścieżkę {}", one.display()).into(),
            many => tr!("Copied {} paths", "Skopiowano ścieżki: {}", many.len()).into(),
        };
        cx.notify();
    }
}

/// Starts `rdppaste <dir> --machine` next to this executable and a thread
/// that turns its stdout lines into shared state.
fn start_paste(dir: &Path, flag: Option<&str>) -> std::io::Result<Arc<Mutex<Paste>>> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let exe = std::env::current_exe()?.with_file_name("rdppaste.exe");
    let mut child = Command::new(exe)
        .arg(dir)
        .args(flag)
        .arg("--machine")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    // If the window crashes, Windows ends the paste too (Q8).
    if let Err(e) = files_win::process::kill_with_parent(&child) {
        eprintln!("rdppaste will not stop with the window: {e}");
    }
    let state = Arc::new(Mutex::new(Paste {
        stdin: child.stdin.take(),
        ..Paste::default()
    }));
    let stdout = child.stdout.take().expect("piped stdout");
    let shared = state.clone();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            match machine::parse_line(&line) {
                Some(Event::Progress(p)) => shared.lock().unwrap().progress = p,
                Some(Event::Result { code, message }) => {
                    shared.lock().unwrap().result = Some((code, message))
                }
                None => {}
            }
        }
        let status = child.wait().ok().and_then(|s| s.code()).unwrap_or(1);
        let mut s = shared.lock().unwrap();
        if s.result.is_none() {
            s.result = Some((status, "rdppaste ended without a result".into()));
        }
    });
    Ok(state)
}

impl Render for Explorer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.polish != i18n::polish() {
            self.polish = i18n::polish();
            self.table.update(cx, |t, cx| t.refresh(cx)); // column headers
        }
        if self.shown_hidden != crate::prefs::get(cx).show_hidden {
            self.load(window, cx); // hidden files turned on or off
        }
        let muted = cx.theme().muted_foreground;
        let progress = self
            .paste
            .as_ref()
            .map(|p| p.lock().unwrap().progress.clone());
        let (can_back, can_forward) = (self.history.can_back(), self.history.can_forward());
        let in_bin = Place::of(self.history.current()) == Place::RecycleBin;
        let drop_border = cx.theme().drag_border;

        let toolbar = div()
            .h_flex()
            .gap_1()
            .px_3()
            .py_2() // the same gap under the tabs as above the file list
            .child(
                Button::new("back")
                    .cursor_pointer()
                    .ghost()
                    .icon(IconName::ArrowLeft)
                    .disabled(!can_back)
                    .tooltip(tr!("Back (Alt+Left)", "Wstecz (Alt+Left)"))
                    .on_click(cx.listener(|this, _, w, cx| this.go_back(&GoBack, w, cx))),
            )
            .child(
                Button::new("forward")
                    .cursor_pointer()
                    .ghost()
                    .icon(IconName::ArrowRight)
                    .disabled(!can_forward)
                    .tooltip(tr!("Forward (Alt+Right)", "Dalej (Alt+Right)"))
                    .on_click(cx.listener(|this, _, w, cx| this.go_forward(&GoForward, w, cx))),
            )
            .child(
                Button::new("up")
                    .cursor_pointer()
                    .ghost()
                    .icon(IconName::ArrowUp)
                    .tooltip(tr!("Up (Alt+Up)", "W górę (Alt+Up)"))
                    .on_click(cx.listener(|this, _, w, cx| this.go_up(&GoUp, w, cx))),
            )
            .child(
                Button::new("refresh")
                    .cursor_pointer()
                    .ghost()
                    .icon(IconName::RefreshCw)
                    .tooltip(tr!("Refresh (F5)", "Odśwież (F5)"))
                    .on_click(cx.listener(|this, _, w, cx| this.refresh(&Refresh, w, cx))),
            )
            .child(if self.editing_address {
                let me = cx.weak_entity();
                div()
                    .flex_1()
                    // Esc: back to the crumbs, the path unchanged.
                    .on_key_down(move |e: &KeyDownEvent, window, cx| {
                        if e.keystroke.key == "escape" {
                            let _ = me.update(cx, |this, cx| {
                                this.editing_address = false;
                                this.focus.focus(window, cx);
                                cx.notify();
                            });
                            cx.stop_propagation();
                        }
                    })
                    .child(Input::new(&self.address))
                    .into_any_element()
            } else {
                self.crumbs(cx).into_any_element()
            })
            .child(
                Button::new("copy-path")
                    .cursor_pointer()
                    .ghost()
                    .icon(IconName::Copy)
                    .tooltip(tr!(
                        "Copy path without quotes (Ctrl+Alt+C)",
                        "Kopiuj ścieżkę bez cudzysłowów (Ctrl+Alt+C)"
                    ))
                    .on_click(cx.listener(|this, _, w, cx| this.copy_path(&CopyPath, w, cx))),
            )
            .child(
                Button::new("grid")
                    .cursor_pointer()
                    .ghost()
                    .selected(self.grid)
                    .icon(IconName::LayoutDashboard)
                    .tooltip(tr!("Thumbnails (Ctrl+Shift+G)", "Miniatury (Ctrl+Shift+G)"))
                    .on_click(cx.listener(|this, _, w, cx| this.toggle_grid(&ToggleGrid, w, cx))),
            )
            .child(
                Button::new("terminal")
                    .cursor_pointer()
                    .ghost()
                    .icon(IconName::SquareTerminal)
                    .tooltip(tr!(
                        "Open terminal here (Ctrl+Alt+T)",
                        "Otwórz terminal tutaj (Ctrl+Alt+T)"
                    ))
                    .on_click(
                        cx.listener(|this, _, w, cx| this.open_terminal(&OpenTerminal, w, cx)),
                    ),
            )
            .when(in_bin, |bar| {
                bar.child(
                    Button::new("restore")
                        .cursor_pointer()
                        .outline()
                        .label(tr!("Restore", "Przywróć"))
                        .on_click(cx.listener(|this, _, w, cx| this.restore_sel(w, cx))),
                )
                .child(
                    Button::new("empty-bin")
                        .cursor_pointer()
                        .outline()
                        .label(tr!("Empty Recycle Bin", "Opróżnij kosz"))
                        .on_click(cx.listener(|this, _, w, cx| this.empty_bin(w, cx))),
                )
            })
            .child(
                Button::new("paste")
                    .cursor_pointer()
                    .primary()
                    .label(tr!("Paste from RDP", "Wklej z RDP"))
                    .tooltip("Ctrl+V")
                    .on_click(cx.listener(|this, _, w, cx| this.paste_here(&PasteHere, w, cx))),
            );

        let mut status = div()
            .h_flex()
            .gap_3()
            .px_3()
            .pb_2()
            .text_sm()
            .text_color(muted)
            .child(self.status.clone())
            .child(
                div()
                    .text_color(cx.theme().foreground)
                    .child(self.notice.clone()),
            );
        if let Some(p) = progress {
            let pct = p
                .bytes_total
                .filter(|&t| t > 0)
                .map(|t| p.bytes_done as f32 / t as f32);
            status = status
                .child(format!(
                    "{}/{} {}, {}{}",
                    p.files_done,
                    p.files_total,
                    tr!("files", "plików"),
                    format_size(p.bytes_done),
                    pct.map(|f| format!(" ({:.0}%)", f * 100.))
                        .unwrap_or_default()
                ))
                .child(
                    div()
                        .w(px(160.))
                        .h(px(6.))
                        .rounded_full()
                        .bg(cx.theme().muted)
                        .child(
                            div()
                                .h_full()
                                .rounded_full()
                                .bg(cx.theme().primary)
                                .w(relative(pct.unwrap_or(0.).clamp(0., 1.))),
                        ),
                )
                .child(
                    Button::new("cancel")
                        .cursor_pointer()
                        .ghost()
                        .label(tr!("Cancel", "Anuluj"))
                        .on_click(cx.listener(|this, _, _, _| this.cancel_paste())),
                );
        }
        if let Some(message) = self.conflict.clone() {
            // rdppaste says "<n> file(s) already exist".
            let count = message.split(' ').next().unwrap_or_default();
            let choice = |id: &'static str, label: String, flag: &'static str| {
                Button::new(id)
                    .cursor_pointer()
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| this.start(Some(flag), cx)))
            };
            status = status
                .child(div().text_color(cx.theme().foreground).child(tr!(
                    "{message}. What should happen to them?",
                    "Pliki już istnieją ({count}). Co z nimi zrobić?"
                )))
                .child(choice("replace", tr!("Replace", "Zastąp"), "--overwrite").primary())
                .child(choice("skip", tr!("Skip existing", "Pomiń istniejące"), "--skip").outline())
                .child(choice("both", tr!("Keep both", "Zachowaj oba"), "--rename").outline())
                .child(
                    Button::new("dismiss")
                        .cursor_pointer()
                        .ghost()
                        .label(tr!("Cancel", "Anuluj"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.conflict = None;
                            this.notice = tr!(
                                "Paste cancelled; nothing was written.",
                                "Wklejanie anulowane; nic nie zapisano."
                            )
                            .into();
                            cx.notify();
                        })),
                );
        }

        div()
            .v_flex()
            .size_full()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::go_back))
            .on_action(cx.listener(Self::go_forward))
            .on_action(cx.listener(Self::go_up))
            .on_action(cx.listener(Self::refresh))
            .on_action(cx.listener(Self::paste_here))
            .on_action(cx.listener(Self::copy_sel))
            .on_action(cx.listener(Self::cut_sel))
            .on_action(cx.listener(Self::delete_sel))
            .on_action(cx.listener(Self::delete_forever))
            .on_action(cx.listener(Self::open_sel))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy_path))
            .on_action(cx.listener(Self::open_terminal))
            .on_action(cx.listener(Self::rename))
            .on_action(cx.listener(Self::focus_address))
            .on_action(cx.listener(Self::new_folder))
            .on_action(cx.listener(Self::clear_selection))
            .on_action(cx.listener(Self::properties))
            .on_action(cx.listener(Self::toggle_grid))
            .on_action(cx.listener(|_, _: &ToggleHidden, _, cx| {
                crate::prefs::set(cx, |s| s.show_hidden = !s.show_hidden)
            }))
            .on_key_down(cx.listener(Self::type_to_select))
            // The mouse's side buttons.
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Back),
                cx.listener(|this, _, w, cx| this.go_back(&GoBack, w, cx)),
            )
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Forward),
                cx.listener(|this, _, w, cx| this.go_forward(&GoForward, w, cx)),
            )
            .text_color(cx.theme().foreground)
            .child(toolbar)
            .child(
                div()
                    .flex_1()
                    .mx_3()
                    .rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(cx.theme().border)
                    .overflow_hidden()
                    // Right-click on the empty space: the folder's own menu (New, Paste...).
                    // The table takes every right-click for its own menu, so this
                    // listens first (capture) and opens the folder's menu once
                    // the click is handled, unless a row took it.
                    .capture_any_mouse_down(cx.listener(|this, e: &MouseDownEvent, window, cx| {
                        if e.button != MouseButton::Right {
                            return;
                        }
                        let row = this.table.read(cx).delegate().row_menu.clone();
                        row.store(false, Ordering::SeqCst);
                        cx.spawn_in(window, async move |this, cx| {
                            if !row.load(Ordering::SeqCst) {
                                let _ = this
                                    .update_in(cx, |this, window, cx| this.folder_menu(window, cx));
                            }
                        })
                        .detach();
                    }))
                    // Files dropped on the list (not on a folder row) land here.
                    .drag_over::<DraggedFiles>(move |s, _, _, _| s.border_color(drop_border))
                    .on_drop(cx.listener(|this, d: &DraggedFiles, window, _| {
                        let here = this.history.current().to_path_buf();
                        drop_files(d, &here, window, Some(this.changed.clone()))
                    }))
                    .child(if Place::of(self.history.current()) == Place::ThisPc {
                        self.this_pc_view(cx).into_any_element()
                    } else if self.grid {
                        self.grid_view(cx).into_any_element()
                    } else {
                        DataTable::new(&self.table)
                            .with_size(theme::table_size())
                            .into_any_element()
                    }),
            )
            .child(div().pt_2().child(status))
            .children(self.preview.as_ref().map(|p| {
                // Beside the pointer, kept on screen.
                let scale = window.scale_factor();
                let size = p.image.size(0);
                // Fit in 320 x 320 logical px, keeping the shape.
                let fit = (320. * scale / size.width.0.max(size.height.0) as f32).min(1.) / scale;
                let (w, h) = (size.width.0 as f32 * fit, size.height.0 as f32 * fit);
                deferred(
                    anchored()
                        .position(p.at + point(px(16.), px(16.)))
                        .snap_to_window_with_margin(px(8.))
                        .child(
                            div()
                                .p_1()
                                .rounded(cx.theme().radius)
                                .border_1()
                                .border_color(cx.theme().border)
                                .bg(cx.theme().popover)
                                .shadow_lg()
                                .child(img(ImageSource::Render(p.image.clone())).w(px(w)).h(px(h))),
                        ),
                )
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::icon_kind;

    #[test]
    fn bad_names() {
        assert!(super::bad_name("ok name.txt").is_none());
        for bad in ["", "..", "a/b", r"a\b", "x:y", "what?", "trail.", "trail "] {
            assert!(super::bad_name(bad).is_some(), "{bad:?}");
        }
    }

    #[test]
    fn icons_by_name() {
        assert_eq!(
            icon_kind("Photo.JPG", false),
            Some(("file-image", Some(150.)))
        );
        assert_eq!(icon_kind("a.tar.gz", false).unwrap().0, "file-archive");
        assert_eq!(icon_kind("setup.ini", false), Some(("file-cog", None)));
        assert_eq!(icon_kind("README", false), None, "no extension");
        assert_eq!(icon_kind("odd.", false), None);
        assert_eq!(icon_kind("Downloads", true).unwrap().0, "folder-down");
        assert_eq!(icon_kind(".cargo", true).unwrap().0, "folder-dot");
        assert_eq!(icon_kind("photos.jpg", true), None, "folders by name only");
    }
}
