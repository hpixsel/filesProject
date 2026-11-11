//! The explorer pane: toolbar, address bar, file table, status bar.

use files_app::places::{self, Place};
use files_app::{format_size, list_dir, History, Item};
use files_core::machine::{self, Event};
use files_core::Progress;
use files_jobs::sizes::{scan_sizes, SizeScan};
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
        OpenTerminal
    ]
);

const CONTEXT: &str = "Explorer";

/// A folder middle-clicked in the list: the workspace opens it as a
/// background tab.
pub struct OpenInNewTab(pub PathBuf);

impl EventEmitter<OpenInNewTab> for Explorer {}

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
fn list(dir: &Path) -> std::io::Result<Vec<Item>> {
    let drives = files_win::shell::drives;
    Ok(match Place::of(dir) {
        Place::Folder => return list_dir(dir),
        Place::RecycleBin => places::recycle_bin(&drives()),
        Place::ThisPc => Vec::new(), // drive tiles instead, see `this_pc`
    })
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
    /// Rows selected with Ctrl/Shift+click (or the single selected row).
    marked: BTreeSet<usize>,
    /// Where a Shift+click range starts.
    anchor: Option<usize>,
    /// This PC and the Recycle Bin show other columns.
    place: Place,
    /// The view that owns this list, which emits `OpenInNewTab`.
    explorer: WeakEntity<Explorer>,
}

impl Listing {
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
        match col_ix {
            0 => Column::new("name", tr!("Name", "Nazwa")).width(px(440.)),
            1 => Column::new("size", tr!("Size", "Rozmiar")).width(px(110.)),
            2 if self.place == Place::RecycleBin => {
                Column::new("modified", tr!("Deleted", "Usunięto")).width(px(160.))
            }
            2 => Column::new("modified", tr!("Modified", "Zmodyfikowano")).width(px(160.)),
            _ => Column::new("origin", tr!("Original location", "Oryginalna lokalizacja"))
                .width(px(320.)),
        }
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
        let explorer = self.explorer.clone();
        let middle = self
            .items
            .get(row_ix)
            .filter(|i| i.is_dir)
            .map(|i| i.path.clone());
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
                let changed = l.changed.clone();
                cx.notify();
                outside_dispatch(cx, move || {
                    if let Ok(true) = files_win::shell::context_menu(h, &paths) {
                        changed.store(true, Ordering::SeqCst);
                    }
                });
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
                div()
                    .h_flex()
                    .gap_2()
                    .child(icon.text_color(color))
                    .child(item.name.clone())
            }
            1 => {
                let size = match item.size {
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
    /// This PC's drive tiles.
    drives: Vec<Drive>,
    focus: FocusHandle,
}

impl Explorer {
    pub fn new(start: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let changed = Arc::new(AtomicBool::new(false));
        let listing = Listing {
            items: Vec::new(),
            changed: changed.clone(),
            sizes: Default::default(),
            marked: Default::default(),
            anchor: None,
            place: Place::Folder,
            explorer: cx.weak_entity(),
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
                if let InputEvent::PressEnter { .. } = ev {
                    let typed = PathBuf::from(input.read(cx).value().trim());
                    if typed.is_dir() || places::is_place(&typed) {
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
            drives: Vec::new(),
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
        let task = cx.background_spawn(async move { list(&dir) });
        cx.spawn_in(window, async move |this, cx| {
            let listed = task.await;
            let _ = this.update(cx, |this, cx| {
                match listed {
                    Ok(items) => {
                        this.status = tr!("{} items", "Elementów: {}", items.len()).into();
                        // Drives and Recycle Bin items: no folder-size scan.
                        let dirs = items
                            .iter()
                            .filter(|i| i.is_dir && i.size.is_none())
                            .map(|i| i.path.clone());
                        // Replacing the scan drops (cancels) the previous folder's one.
                        let scan = scan_sizes(dirs.collect());
                        let sizes = scan.results();
                        this.scan = Some(scan);
                        this.table.update(cx, |t, cx| {
                            t.delegate_mut().items = items;
                            t.delegate_mut().place = place;
                            t.delegate_mut().sizes = sizes;
                            t.delegate_mut().marked.clear();
                            t.delegate_mut().anchor = None;
                            t.refresh(cx);
                        });
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
            self.history.go(item.path);
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.polish != i18n::polish() {
            self.polish = i18n::polish();
            self.table.update(cx, |t, cx| t.refresh(cx)); // column headers
        }
        let muted = cx.theme().muted_foreground;
        let progress = self
            .paste
            .as_ref()
            .map(|p| p.lock().unwrap().progress.clone());
        let (can_back, can_forward) = (self.history.can_back(), self.history.can_forward());
        let in_bin = Place::of(self.history.current()) == Place::RecycleBin;

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
            .child(div().flex_1().child(Input::new(&self.address)))
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
                    .child(if Place::of(self.history.current()) == Place::ThisPc {
                        self.this_pc_view(cx).into_any_element()
                    } else {
                        DataTable::new(&self.table)
                            .with_size(theme::table_size())
                            .into_any_element()
                    }),
            )
            .child(div().pt_2().child(status))
    }
}

#[cfg(test)]
mod tests {
    use super::icon_kind;

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
