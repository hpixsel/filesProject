//! The explorer pane: toolbar, address bar, file table, status bar.

use crate::theme;
use files_app::{format_size, list_dir, History, Item};
use files_core::machine::{self, Event};
use files_core::Progress;
use gpui_kit::component::button::*;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::table::*;
use gpui_kit::component::*;
use gpui_kit::*;
use std::io::BufRead;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

actions!(files, [GoBack, GoForward, GoUp, Refresh, PasteHere]);

const CONTEXT: &str = "Explorer";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("alt-left", GoBack, Some(CONTEXT)),
        KeyBinding::new("alt-right", GoForward, Some(CONTEXT)),
        KeyBinding::new("alt-up", GoUp, Some(CONTEXT)),
        KeyBinding::new("f5", Refresh, Some(CONTEXT)),
        KeyBinding::new("ctrl-v", PasteHere, Some(CONTEXT)),
    ]);
}

fn hwnd(window: &Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

/// Runs a shell call that may start a modal message loop (menus, dialogs)
/// from a task, outside GPUI's event dispatch: inside it, the nested loop
/// re-enters GPUI while its state is borrowed and the app panics.
fn outside_dispatch(cx: &mut App, f: impl FnOnce() + 'static) {
    cx.spawn(async move |_: &mut AsyncApp| f()).detach();
}

pub struct Listing {
    items: Vec<Item>,
    /// Set after a shell command that may have changed the folder.
    changed: Arc<AtomicBool>,
}

impl TableDelegate for Listing {
    fn columns_count(&self, _: &App) -> usize {
        3
    }
    fn rows_count(&self, _: &App) -> usize {
        self.items.len()
    }
    fn column(&self, col_ix: usize, _: &App) -> Column {
        match col_ix {
            0 => Column::new("name", "Name").width(px(440.)),
            1 => Column::new("size", "Size").width(px(110.)),
            _ => Column::new("modified", "Modified").width(px(160.)),
        }
    }
    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let path = self.items.get(row_ix).map(|i| i.path.clone());
        let changed = self.changed.clone();
        div()
            .id(("row", row_ix))
            .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                let (Some(h), Some(path)) = (hwnd(window), path.clone()) else {
                    return;
                };
                let changed = changed.clone();
                outside_dispatch(cx, move || {
                    if let Ok(true) = files_win::shell::context_menu(h, &path) {
                        changed.store(true, Ordering::SeqCst);
                    }
                });
            })
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
                let (icon, color) = if item.is_dir {
                    (IconName::Folder, cx.theme().primary)
                } else {
                    (IconName::File, muted)
                };
                div()
                    .h_flex()
                    .gap_2()
                    .child(Icon::new(icon).text_color(color))
                    .child(item.name.clone())
            }
            1 => div()
                .text_color(muted)
                .child(item.size.map(format_size).unwrap_or_default()),
            _ => div().text_color(muted).child(
                item.modified
                    .map(files_win::shell::local_time)
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
    /// Set when a paste stopped because files already exist; asks what to do.
    conflict: Option<SharedString>,
    changed: Arc<AtomicBool>,
    accent: usize,
    focus: FocusHandle,
}

impl Explorer {
    pub fn new(start: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let changed = Arc::new(AtomicBool::new(false));
        let listing = Listing {
            items: Vec::new(),
            changed: changed.clone(),
        };
        let table = cx.new(|cx| TableState::new(listing, window, cx));
        let address = cx.new(|cx| InputState::new(window, cx));

        cx.subscribe_in(&table, window, |this, _, ev: &TableEvent, window, cx| {
            if let TableEvent::DoubleClickedRow(ix) = ev {
                this.open_row(*ix, window, cx);
            }
        })
        .detach();
        cx.subscribe_in(
            &address,
            window,
            |this, input, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    let typed = PathBuf::from(input.read(cx).value().trim());
                    if typed.is_dir() {
                        this.history.go(typed);
                        this.load(window, cx);
                    } else {
                        this.status = format!("Not a folder: {}", typed.display()).into();
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
            conflict: None,
            changed,
            accent: 0,
            focus: cx.focus_handle(),
        };
        this.load(window, cx);
        this.focus.focus(window, cx);
        this
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.history.current().to_path_buf();
        let shown = dir.display().to_string();
        self.address
            .update(cx, |a, cx| a.set_value(shown, window, cx));
        self.status = "Loading…".into();
        cx.notify();
        let task = cx.background_spawn(async move { list_dir(&dir) });
        cx.spawn_in(window, async move |this, cx| {
            let listed = task.await;
            let _ = this.update(cx, |this, cx| {
                match listed {
                    Ok(items) => {
                        this.status = format!("{} items", items.len()).into();
                        this.table.update(cx, |t, cx| {
                            t.delegate_mut().items = items;
                            t.refresh(cx);
                        });
                    }
                    Err(e) => this.status = format!("Cannot open folder: {e}").into(),
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

    fn paste_here(&mut self, _: &PasteHere, _: &mut Window, cx: &mut Context<Self>) {
        self.start(None, cx);
    }

    /// `flag` is the conflict choice (`--overwrite`, `--skip`, `--rename`),
    /// or `None` for the first attempt, which stops if files already exist.
    fn start(&mut self, flag: Option<&'static str>, cx: &mut Context<Self>) {
        self.conflict = None;
        if self.paste.is_some() {
            self.notice = "A paste is already running.".into();
            cx.notify();
            return;
        }
        match start_paste(self.history.current(), flag) {
            Ok(state) => {
                self.paste = Some(state);
                self.notice = "Pasting…".into();
            }
            Err(e) => self.notice = format!("Cannot start rdppaste: {e}").into(),
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
                4 if message.contains("CF_HDROP") => "These are regular files, not files from an RDP session or a zip. Pasting regular files comes with local copy (M2).".into(),
                4 => "The clipboard holds no files. Copy files (Ctrl+C) in Explorer or in the RDP session first.".into(),
                // rdppaste exit code 5: files already exist, nothing written.
                5 => {
                    self.conflict = Some(message.into());
                    SharedString::default()
                }
                _ => format!("Paste failed: {message}").into(),
            };
            if code == 0 {
                self.load(window, cx);
            }
        }
        cx.notify();
    }

    fn cycle_accent(&mut self, cx: &mut Context<Self>) {
        self.accent = (self.accent + 1) % theme::ACCENTS.len();
        theme::apply(theme::ACCENTS[self.accent].1, cx);
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
        let dark = cx.theme().mode.is_dark();
        let hue = theme::ACCENTS[self.accent].1;
        let muted = cx.theme().muted_foreground;
        let progress = self
            .paste
            .as_ref()
            .map(|p| p.lock().unwrap().progress.clone());
        let (can_back, can_forward) = (self.history.can_back(), self.history.can_forward());

        let toolbar = div()
            .h_flex()
            .gap_1()
            .px_3()
            .pb_2()
            .child(
                Button::new("back")
                    .ghost()
                    .icon(IconName::ArrowLeft)
                    .disabled(!can_back)
                    .tooltip("Back (Alt+Left)")
                    .on_click(cx.listener(|this, _, w, cx| this.go_back(&GoBack, w, cx))),
            )
            .child(
                Button::new("forward")
                    .ghost()
                    .icon(IconName::ArrowRight)
                    .disabled(!can_forward)
                    .tooltip("Forward (Alt+Right)")
                    .on_click(cx.listener(|this, _, w, cx| this.go_forward(&GoForward, w, cx))),
            )
            .child(
                Button::new("up")
                    .ghost()
                    .icon(IconName::ArrowUp)
                    .tooltip("Up (Alt+Up)")
                    .on_click(cx.listener(|this, _, w, cx| this.go_up(&GoUp, w, cx))),
            )
            .child(
                Button::new("refresh")
                    .ghost()
                    .icon(IconName::RefreshCw)
                    .tooltip("Refresh (F5)")
                    .on_click(cx.listener(|this, _, w, cx| this.refresh(&Refresh, w, cx))),
            )
            .child(div().flex_1().child(Input::new(&self.address)))
            .child(
                Button::new("paste")
                    .primary()
                    .label("Paste from RDP")
                    .tooltip("Ctrl+V")
                    .on_click(cx.listener(|this, _, w, cx| this.paste_here(&PasteHere, w, cx))),
            )
            .child(
                Button::new("mode")
                    .ghost()
                    .icon(if dark { IconName::Sun } else { IconName::Moon })
                    .tooltip("Light / dark")
                    .on_click(move |_, window, cx| {
                        let mode = if dark {
                            ThemeMode::Light
                        } else {
                            ThemeMode::Dark
                        };
                        Theme::change(mode, Some(window), cx);
                        files_win::shell::set_menu_dark(!dark);
                        theme::apply(hue, cx); // change() resets the colours
                    }),
            )
            .child(
                Button::new("accent")
                    .ghost()
                    .icon(IconName::Palette)
                    .tooltip("Accent colour")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.cycle_accent(cx);
                        window.refresh();
                    })),
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
                    "{}/{} files, {}{}",
                    p.files_done,
                    p.files_total,
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
                        .ghost()
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, _, _| this.cancel_paste())),
                );
        }
        if let Some(message) = self.conflict.clone() {
            let choice = |id: &'static str, label: &'static str, flag: &'static str| {
                Button::new(id)
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| this.start(Some(flag), cx)))
            };
            status = status
                .child(
                    div()
                        .text_color(cx.theme().foreground)
                        .child(format!("{message}. What should happen to them?")),
                )
                .child(choice("replace", "Replace", "--overwrite").primary())
                .child(choice("skip", "Skip existing", "--skip").outline())
                .child(choice("both", "Keep both", "--rename").outline())
                .child(
                    Button::new("dismiss")
                        .ghost()
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.conflict = None;
                            this.notice = "Paste cancelled; nothing was written.".into();
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
            .text_color(cx.theme().foreground)
            .child(TitleBar::new().child(div().pl_2().child("FilesProject")))
            .child(toolbar)
            .child(
                div()
                    .flex_1()
                    .mx_3()
                    .rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(cx.theme().border)
                    .overflow_hidden()
                    .child(DataTable::new(&self.table)),
            )
            .child(div().pt_2().child(status))
    }
}
