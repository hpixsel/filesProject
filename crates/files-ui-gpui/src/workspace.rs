//! The window: title bar, sidebar (favourites, drives), one or two panes,
//! each pane with its own tabs. Every tab is an `Explorer`.

use crate::explorer::{self, DraggedFiles, Explorer, OpenInNewTab};
use crate::i18n::tr;
use crate::{prefs, theme};
use files_app::favourites::{self, Group};
use files_app::places;
use files_app::session;
use files_app::update;
use gpui_kit::component::button::*;
use gpui_kit::component::color_picker::{ColorPickerEvent, ColorPickerState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::*;
use gpui_kit::*;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

actions!(
    workspace,
    [
        NewTab,
        CloseTab,
        NextTab,
        PrevTab,
        ToggleSplit,
        OtherPane,
        ToggleFavourite,
        OpenPalette,
        OpenSettings,
        ToggleDark
    ]
);

const CONTEXT: &str = "Workspace";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-t", NewTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-w", CloseTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-tab", NextTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-tab", PrevTab, Some(CONTEXT)),
        KeyBinding::new("f6", ToggleSplit, Some(CONTEXT)),
        KeyBinding::new("ctrl-f6", OtherPane, Some(CONTEXT)),
        KeyBinding::new("ctrl-d", ToggleFavourite, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-p", OpenPalette, Some(CONTEXT)),
    ]);
}

struct Pane {
    tabs: Vec<Entity<Explorer>>,
    active: usize,
}

impl Pane {
    fn current(&self) -> &Entity<Explorer> {
        &self.tabs[self.active]
    }
}

type DropSink = Box<dyn Fn(files_win::Result<()>)>;

/// The update offered in the title bar; written by background threads.
#[derive(Clone, Default, PartialEq)]
enum Update {
    #[default]
    None,
    Available(String),
    Working,
    Failed(String),
    /// Installed; the new version is starting, so this window closes.
    Restart,
}

pub struct Workspace {
    panes: Vec<Pane>,
    active_pane: usize,
    /// The sidebar's groups of favourites, and where the drives sit.
    layout: favourites::Layout,
    /// A group whose name is being typed.
    editing_group: Option<(usize, Entity<InputState>)>,
    /// Drive roots, with the share for mapped network drives.
    drives: Vec<(PathBuf, Option<String>)>,
    picker: Entity<ColorPickerState>,
    /// Read by the window's drop target at drop time: the active tab's folder…
    drop_dir: Arc<Mutex<PathBuf>>,
    /// …and where that drop's outcome goes (the active tab).
    drop_sink: Arc<Mutex<Option<DropSink>>>,
    focus: FocusHandle,
    update: Arc<Mutex<Update>>,
    /// Kept up to date, saved with the tabs on exit.
    bounds: WindowBounds,
}

impl Workspace {
    /// `explicit`: `start` was asked for (`files <folder>`); with restored
    /// tabs it opens as a new one, otherwise it is the only tab.
    pub fn new(
        start: PathBuf,
        explicit: bool,
        incoming: std::sync::mpsc::Receiver<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let saved = prefs::get(cx)
            .restore_tabs
            .then(|| session::file().and_then(|f| session::load(&f)))
            .flatten();
        let (panes, active_pane) = match &saved {
            Some(s) => (
                s.panes
                    .iter()
                    .map(|(dirs, active)| Pane {
                        tabs: dirs
                            .iter()
                            .map(|d| Self::explorer(d.clone(), window, cx))
                            .collect(),
                        active: *active,
                    })
                    .collect(),
                s.active_pane,
            ),
            None => (
                vec![Pane {
                    tabs: vec![Self::explorer(start.clone(), window, cx)],
                    active: 0,
                }],
                0,
            ),
        };
        cx.observe_window_bounds(window, |this: &mut Self, window, _| {
            this.bounds = window.window_bounds();
        })
        .detach();
        // Tabs and window are saved when the window closes (its X button,
        // Alt+F4, the taskbar) and when the app quits (the updater restart);
        // closing the window does not quit the app.
        cx.on_app_quit(|this: &mut Self, cx| {
            this.save_session(cx);
            async {}
        })
        .detach();
        let me = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            let _ = me.update(cx, |this, cx| this.save_session(cx));
            true
        });
        let accent = theme::accent(prefs::get(cx));
        let picker = cx.new(|cx| ColorPickerState::new(window, cx).default_value(accent));
        cx.subscribe_in(&picker, window, |_, _, e: &ColorPickerEvent, _, cx| {
            if let ColorPickerEvent::Change(Some(c)) = e {
                prefs::set(cx, |s| s.accent = Some((c.h * 360., c.s)));
            }
        })
        .detach();
        // `files <folder>` from another process: a new tab, window to the front.
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(200))
                .await;
            let alive = this.update_in(cx, |this, window, cx| {
                for dir in incoming.try_iter() {
                    this.open_tab(dir, window, cx);
                    window.activate_window();
                }
                if crate::ops::tidy(cx) {
                    cx.notify(); // progress bars move
                }
                let update = this.update.lock().unwrap().clone();
                match update {
                    Update::Restart => cx.quit(),
                    Update::None => {}
                    _ => cx.notify(), // the title-bar button follows the state
                }
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();
        let mut this = Workspace {
            panes,
            active_pane,
            layout: favourites::file()
                .map(|f| favourites::load(&f))
                .unwrap_or_default(),
            editing_group: None,
            drives: drives(),
            picker,
            drop_dir: Default::default(),
            drop_sink: Default::default(),
            focus: cx.focus_handle(),
            update: Default::default(),
            bounds: window.window_bounds(),
        };
        // Only the installed copy updates itself; one quiet check per start.
        if update::is_installed() {
            let state = this.update.clone();
            std::thread::spawn(move || match update::latest_tag() {
                Ok(tag) if update::is_newer(&tag, env!("CARGO_PKG_VERSION")) => {
                    *state.lock().unwrap() = Update::Available(tag)
                }
                Ok(_) => {}
                Err(e) => eprintln!("update check: {e}"),
            });
        }
        if let Some(h) = explorer::hwnd(window) {
            files_win::shell::set_acrylic(h);
            let sink = this.drop_sink.clone();
            let on_drop = move |r: files_win::Result<()>| {
                if let Some(f) = sink.lock().unwrap().as_ref() {
                    f(r);
                }
            };
            if let Err(e) = files_win::drop::register(h, this.drop_dir.clone(), on_drop) {
                eprintln!("drag and drop is off: {e}");
            }
        }
        if saved.is_some() && explicit {
            this.open_tab(start, window, cx);
        }
        this.focus_active(window, cx);
        this
    }

    fn save_session(&self, cx: &App) {
        let s = session::Session {
            panes: self
                .panes
                .iter()
                .map(|p| {
                    let dirs = p
                        .tabs
                        .iter()
                        .map(|t| t.read(cx).current_dir().to_path_buf());
                    (dirs.collect(), p.active)
                })
                .collect(),
            active_pane: self.active_pane,
            window: Some(match self.bounds {
                WindowBounds::Windowed(b) => (b, false),
                WindowBounds::Maximized(b) | WindowBounds::Fullscreen(b) => (b, true),
            })
            .map(|(b, max)| {
                let (o, sz) = (b.origin, b.size);
                (
                    o.x.into(),
                    o.y.into(),
                    sz.width.into(),
                    sz.height.into(),
                    max,
                )
            }),
        };
        if let Some(f) = session::file() {
            if let Err(e) = session::save(&f, &s) {
                eprintln!("cannot save tabs: {e}");
            }
        }
    }

    fn active(&self) -> &Entity<Explorer> {
        self.panes[self.active_pane].current()
    }

    fn focus_active(&self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.active().read(cx).focus_handle();
        focus.focus(window, cx);
        cx.notify();
    }

    /// A new tab's view; its middle-clicked folders open as new tabs next
    /// to it.
    fn explorer(dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Entity<Explorer> {
        let tab = cx.new(|cx| Explorer::new(dir, window, cx));
        cx.subscribe_in(&tab, window, |this, tab, e: &OpenInNewTab, window, cx| {
            let pane = this.panes.iter().position(|p| p.tabs.contains(tab));
            this.add_tab(e.0.clone(), pane.unwrap_or(this.active_pane), window, cx);
        })
        .detach();
        tab
    }

    /// Opens `dir` in a new tab right after the current one of `pane_ix`
    /// and switches to it.
    fn add_tab(
        &mut self,
        dir: PathBuf,
        pane_ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = Self::explorer(dir, window, cx);
        let pane = &mut self.panes[pane_ix];
        pane.tabs.insert(pane.active + 1, tab);
        pane.active += 1;
        self.active_pane = pane_ix;
        self.focus_active(window, cx);
    }

    fn open_tab(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.add_tab(dir, self.active_pane, window, cx);
    }

    fn new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.active().read(cx).current_dir().to_path_buf();
        self.open_tab(dir, window, cx);
    }

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.close(
            self.active_pane,
            self.panes[self.active_pane].active,
            window,
            cx,
        );
    }

    /// Closes a tab; the last tab of the second pane closes that pane, and
    /// the last tab of the only pane stays (there is always one).
    fn close(
        &mut self,
        pane_ix: usize,
        tab_ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane = &mut self.panes[pane_ix];
        if pane.tabs.len() > 1 {
            pane.tabs.remove(tab_ix);
            pane.active = pane.active.min(pane.tabs.len() - 1);
        } else if self.panes.len() > 1 {
            self.panes.remove(pane_ix);
            self.active_pane = 0;
        }
        self.focus_active(window, cx);
    }

    fn step_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let pane = &mut self.panes[self.active_pane];
        let n = pane.tabs.len() as isize;
        pane.active = (pane.active as isize + delta).rem_euclid(n) as usize;
        self.focus_active(window, cx);
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.step_tab(1, window, cx);
    }
    fn prev_tab(&mut self, _: &PrevTab, window: &mut Window, cx: &mut Context<Self>) {
        self.step_tab(-1, window, cx);
    }

    /// F6: a second pane on the right (same folder), or back to one pane.
    fn toggle_split(&mut self, _: &ToggleSplit, window: &mut Window, cx: &mut Context<Self>) {
        if self.panes.len() > 1 {
            self.panes.truncate(1);
            self.active_pane = 0;
        } else {
            let dir = self.active().read(cx).current_dir().to_path_buf();
            let tab = Self::explorer(dir, window, cx);
            self.panes.push(Pane {
                tabs: vec![tab],
                active: 0,
            });
            self.active_pane = 1;
        }
        self.focus_active(window, cx);
    }

    fn other_pane(&mut self, _: &OtherPane, window: &mut Window, cx: &mut Context<Self>) {
        self.active_pane = (self.active_pane + 1) % self.panes.len();
        self.focus_active(window, cx);
    }

    fn toggle_favourite(&mut self, _: &ToggleFavourite, _: &mut Window, cx: &mut Context<Self>) {
        let dir = self.active().read(cx).current_dir().to_path_buf();
        self.layout.toggle(&dir);
        self.save_favourites();
        cx.notify();
    }

    /// "Update to vX" while a newer release exists, then its progress.
    fn update_button(&self, cx: &mut Context<Self>) -> Option<Button> {
        let state = self.update.lock().unwrap().clone();
        let button = Button::new("update").cursor_pointer().xsmall();
        Some(match state {
            Update::None | Update::Restart => return None,
            Update::Available(tag) => button
                .primary()
                .label(tr!("Update to {tag}", "Aktualizuj do {tag}"))
                .on_click(cx.listener(move |this, _, _, cx| this.start_update(tag.clone(), cx))),
            Update::Working => button
                .ghost()
                .loading(true)
                .label(tr!("Updating…", "Aktualizowanie…")),
            Update::Failed(e) => button
                .ghost()
                .label(tr!("Update failed", "Aktualizacja nie powiodła się"))
                .tooltip(e)
                .on_click(cx.listener(|this, _, _, cx| {
                    *this.update.lock().unwrap() = Update::None;
                    cx.notify();
                })),
        })
    }

    /// Downloads and checks the release, swaps the files, and starts the new
    /// version in this folder; the poll loop then closes this window.
    fn start_update(&mut self, tag: String, cx: &mut Context<Self>) {
        let state = self.update.clone();
        let dir = self.active().read(cx).current_dir().to_path_buf();
        *state.lock().unwrap() = Update::Working;
        cx.notify();
        std::thread::spawn(move || {
            let run = || -> Result<(), String> {
                let home = update::install_dir().ok_or("LOCALAPPDATA is not set")?;
                let staging = home.join("update");
                update::download(&tag, &staging)?;
                update::apply(&staging, &home)?;
                std::process::Command::new(home.join("files.exe"))
                    .arg("--after-update")
                    .arg(&dir)
                    .spawn()
                    .map_err(|e| e.to_string())?;
                Ok(())
            };
            *state.lock().unwrap() = match run() {
                Ok(()) => Update::Restart,
                Err(e) => Update::Failed(e),
            };
        });
    }

    /// A tab dropped on a pane's files: it moves to that pane. Dropped on
    /// its own pane while there is one pane, it becomes the second pane (a
    /// copy of its folder when it is the only tab).
    fn move_tab(
        &mut self,
        from: usize,
        ix: usize,
        to: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.panes.get(from).and_then(|p| p.tabs.get(ix)).cloned() else {
            return;
        };
        if from == to {
            if self.panes.len() > 1 {
                return; // already there
            }
            let moved = if self.panes[from].tabs.len() > 1 {
                self.take_tab(from, ix);
                tab
            } else {
                let dir = tab.read(cx).current_dir().to_path_buf();
                Self::explorer(dir, window, cx)
            };
            self.panes.push(Pane {
                tabs: vec![moved],
                active: 0,
            });
            self.active_pane = self.panes.len() - 1;
        } else {
            self.take_tab(from, ix);
            let pane = &mut self.panes[to];
            pane.tabs.push(tab);
            pane.active = pane.tabs.len() - 1;
            self.active_pane = to;
            if self.panes[from].tabs.is_empty() {
                self.panes.remove(from);
                self.active_pane = 0;
            }
        }
        self.focus_active(window, cx);
    }

    /// Removes a tab from its pane, keeping that pane's active index valid.
    fn take_tab(&mut self, pane_ix: usize, ix: usize) {
        let pane = &mut self.panes[pane_ix];
        pane.tabs.remove(ix);
        pane.active = pane.active.min(pane.tabs.len().saturating_sub(1));
    }

    /// Ctrl+Shift+P: every command, then recent and favourite folders.
    fn open_palette(&mut self, _: &OpenPalette, window: &mut Window, cx: &mut Context<Self>) {
        use crate::explorer as e;
        use crate::palette::{Entry, Palette, Run};
        let cmd = |label: String, hint: &str, action: Box<dyn Action>| Entry {
            label,
            hint: hint.into(),
            run: Run::Action(action),
        };
        let mut entries = vec![
            cmd(tr!("New tab", "Nowa karta"), "Ctrl+T", Box::new(NewTab)),
            cmd(
                tr!("Close tab", "Zamknij kartę"),
                "Ctrl+W",
                Box::new(CloseTab),
            ),
            cmd(
                tr!("Split panes", "Podziel panele"),
                "F6",
                Box::new(ToggleSplit),
            ),
            cmd(
                tr!("Other pane", "Drugi panel"),
                "Ctrl+F6",
                Box::new(OtherPane),
            ),
            cmd(
                tr!("Favourite this folder", "Dodaj folder do ulubionych"),
                "Ctrl+D",
                Box::new(ToggleFavourite),
            ),
            cmd(
                tr!("New folder", "Nowy folder"),
                "Ctrl+Shift+N",
                Box::new(e::NewFolder),
            ),
            cmd(tr!("Rename", "Zmień nazwę"), "F2", Box::new(e::Rename)),
            cmd(tr!("Search", "Szukaj"), "Ctrl+F", Box::new(e::Search)),
            cmd(
                tr!("Thumbnails", "Miniatury"),
                "Ctrl+Shift+G",
                Box::new(e::ToggleGrid),
            ),
            cmd(
                tr!("Preview pane", "Panel podglądu"),
                "Alt+P",
                Box::new(e::TogglePane),
            ),
            cmd(
                tr!("Show hidden files", "Pokaż ukryte pliki"),
                "Ctrl+H",
                Box::new(e::ToggleHidden),
            ),
            cmd(
                tr!("Open terminal here", "Otwórz terminal tutaj"),
                "Ctrl+Alt+T",
                Box::new(e::OpenTerminal),
            ),
            cmd(
                tr!("Copy path", "Kopiuj ścieżkę"),
                "Ctrl+Alt+C",
                Box::new(e::CopyPath),
            ),
            cmd(tr!("Paste", "Wklej"), "Ctrl+V", Box::new(e::PasteHere)),
            cmd(
                tr!("Properties", "Właściwości"),
                "Alt+Enter",
                Box::new(e::Properties),
            ),
            cmd(tr!("Refresh", "Odśwież"), "F5", Box::new(e::Refresh)),
            cmd(tr!("Up", "W górę"), "Alt+Up", Box::new(e::GoUp)),
            cmd(
                tr!("Select all", "Zaznacz wszystko"),
                "Ctrl+A",
                Box::new(e::SelectAll),
            ),
            cmd(tr!("Settings", "Ustawienia"), "", Box::new(OpenSettings)),
            cmd(
                tr!("Light / dark", "Jasny / ciemny"),
                "",
                Box::new(ToggleDark),
            ),
        ];
        // Folders: where the tabs are and have been, then the favourites.
        let mut seen = std::collections::HashSet::new();
        let tabs = self.panes.iter().flat_map(|p| p.tabs.iter());
        let recent: Vec<PathBuf> = tabs.flat_map(|t| t.read(cx).recent()).collect();
        let favourites = self.layout.0.iter().flat_map(|g| match g {
            Group::Folders { paths, .. } => paths.clone(),
            Group::Drives => Vec::new(),
        });
        for dir in recent.into_iter().chain(favourites) {
            if seen.insert(dir.clone()) {
                let label = dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| dir.display().to_string());
                entries.push(Entry {
                    label,
                    hint: dir.display().to_string(),
                    run: Run::Go(dir),
                });
            }
        }
        let me = cx.weak_entity();
        let choose = move |run: &Run, window: &mut Window, cx: &mut App| {
            let _ = me.update(cx, |this, cx| {
                this.focus_active(window, cx);
                match run {
                    Run::Action(a) => window.dispatch_action(a.boxed_clone(), cx),
                    Run::Go(dir) => this.go_to(dir.clone(), window, cx),
                }
            });
        };
        let palette = cx.new(|cx| Palette::new(entries, choose, window, cx));
        let shown = palette.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            dialog.w(px(600.)).child(shown.clone())
        });
        palette.update(cx, |p, cx| p.focus(window, cx));
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::settings_page::open(self.picker.clone(), window, cx);
    }

    fn save_favourites(&self) {
        if let Some(f) = favourites::file() {
            if let Err(e) = favourites::save(&f, &self.layout) {
                eprintln!("cannot save favourites: {e}");
            }
        }
    }

    fn go_to(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.active().update(cx, |e, cx| e.go_to(dir, window, cx));
        self.focus_active(window, cx);
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let muted = cx.theme().muted_foreground;
        let (hover, icon_color, radius) = (
            cx.theme().table_hover,
            cx.theme().primary,
            cx.theme().radius,
        );
        let drop_line = cx.theme().drag_border;
        let entry = move |id: SharedString,
                          label: String,
                          icon: Icon,
                          path: PathBuf,
                          cx: &mut Context<Self>| {
            let (middle, drop_dest) = (path.clone(), path.clone());
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .w_full()
                .px_2()
                .py_1()
                .rounded(radius)
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .drag_over::<DraggedFiles>(move |s, _, _, _| s.bg(hover))
                .on_drop(move |d: &DraggedFiles, window, cx| {
                    explorer::drop_files(d, &drop_dest, window, None, cx)
                })
                .child(icon.text_color(icon_color))
                .child(div().truncate().child(label))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.go_to(path.clone(), window, cx)),
                )
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(move |this, _, window, cx| {
                        let pane = this.active_pane;
                        this.add_tab(middle.clone(), pane, window, cx)
                    }),
                )
        };
        // A group's header: its name (or the rename field), draggable to move
        // the group; items and groups dropped on it land there.
        let header = move |g: usize, name: String, cx: &mut Context<Self>| {
            div()
                .id(("group", g))
                .flex()
                .flex_row()
                .items_end()
                .px_2()
                .pt_3()
                .pb_1()
                .rounded(radius)
                .text_xs()
                .text_color(muted)
                .cursor_grab()
                .on_drag(SidebarDrag::Group(g, name.clone()), |d, _, _, cx| {
                    cx.new(|_| d.clone())
                })
                .drag_over::<SidebarDrag>(move |s, _, _, _| s.border_t_2().border_color(drop_line))
                .on_drop(cx.listener(move |this, d: &SidebarDrag, _, cx| {
                    match d {
                        SidebarDrag::Group(from, _) => this.layout.move_group(*from, g),
                        SidebarDrag::Item(fg, fi, _) => {
                            let end = usize::MAX;
                            this.layout.move_item((*fg, *fi), (g, end))
                        }
                    }
                    this.save_favourites();
                    cx.notify();
                }))
        };
        let me = cx.weak_entity();
        let mut side = div()
            .id("sidebar")
            .v_flex()
            .w(px(200.))
            .flex_shrink_0()
            .px_2()
            .overflow_y_scroll();
        for (g, group) in self.layout.0.iter().enumerate() {
            match group {
                Group::Folders { name, paths } => {
                    let title: AnyElement = match &self.editing_group {
                        Some((eg, input)) if *eg == g => {
                            Input::new(input).xsmall().into_any_element()
                        }
                        _ => div()
                            .flex_1()
                            .truncate()
                            .child(name.clone())
                            .into_any_element(),
                    };
                    let (rename, remove) = (me.clone(), me.clone());
                    let (rename_label, remove_label) = (
                        tr!("Rename group", "Zmień nazwę grupy"),
                        tr!("Remove group", "Usuń grupę"),
                    );
                    side = side.child(
                        header(g, name.clone(), cx)
                            .child(title)
                            // Folders dragged from the list onto the name join the group.
                            .drag_over::<DraggedFiles>(move |s, _, _, _| s.bg(hover))
                            .on_drop(cx.listener(move |this, d: &DraggedFiles, _, cx| {
                                for p in d.paths().iter().filter(|p| p.is_dir()) {
                                    this.layout.add_to(g, p.clone());
                                }
                                this.save_favourites();
                                cx.notify();
                            }))
                            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                                if e.click_count() >= 2 {
                                    this.start_group_rename(g, window, cx);
                                }
                            }))
                            .context_menu(move |menu, _, _| {
                                let (rename, remove) = (rename.clone(), remove.clone());
                                menu.item(PopupMenuItem::new(rename_label.clone()).on_click(
                                    move |_, window, cx| {
                                        let _ = rename.update(cx, |this, cx| {
                                            this.start_group_rename(g, window, cx)
                                        });
                                    },
                                ))
                                .item(
                                    PopupMenuItem::new(remove_label.clone()).on_click(
                                        move |_, _, cx| {
                                            let _ = remove.update(cx, |this, cx| {
                                                this.layout.remove_group(g);
                                                this.save_favourites();
                                                cx.notify();
                                            });
                                        },
                                    ),
                                )
                            }),
                    );
                    if paths.is_empty() {
                        // An empty group's hint takes drops too: favourites from
                        // other groups, and folders from the list.
                        side = side.child(
                            div()
                                .id(("empty-group", g))
                                .px_2()
                                .py_1()
                                .rounded(radius)
                                .text_sm()
                                .text_color(muted)
                                .drag_over::<SidebarDrag>(move |s, _, _, _| s.bg(hover))
                                .drag_over::<DraggedFiles>(move |s, _, _, _| s.bg(hover))
                                .on_drop(cx.listener(move |this, d: &SidebarDrag, _, cx| {
                                    match d {
                                        SidebarDrag::Item(fg, fi, _) => {
                                            this.layout.move_item((*fg, *fi), (g, 0))
                                        }
                                        SidebarDrag::Group(from, _) => {
                                            this.layout.move_group(*from, g)
                                        }
                                    }
                                    this.save_favourites();
                                    cx.notify();
                                }))
                                .on_drop(cx.listener(move |this, d: &DraggedFiles, _, cx| {
                                    for p in d.paths().iter().filter(|p| p.is_dir()) {
                                        this.layout.add_to(g, p.clone());
                                    }
                                    this.save_favourites();
                                    cx.notify();
                                }))
                                .child(tr!(
                                    "Ctrl+D or drop a folder here",
                                    "Ctrl+D albo upuść tu folder"
                                )),
                        );
                    }
                    for (i, p) in paths.iter().enumerate() {
                        let label = p
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| p.display().to_string());
                        let remove = me.clone();
                        let remove_label = tr!("Remove from favourites", "Usuń z ulubionych");
                        side = side.child(
                            entry(
                                format!("fav-{g}-{i}").into(),
                                label.clone(),
                                Icon::new(IconName::Star),
                                p.clone(),
                                cx,
                            )
                            // Dragged to reorder, or onto another group.
                            .on_drag(SidebarDrag::Item(g, i, label), |d, _, _, cx| {
                                cx.new(|_| d.clone())
                            })
                            .drag_over::<SidebarDrag>(move |s, _, _, _| {
                                s.border_t_2().border_color(drop_line)
                            })
                            .on_drop(cx.listener(move |this, d: &SidebarDrag, _, cx| {
                                match d {
                                    SidebarDrag::Item(fg, fi, _) => {
                                        this.layout.move_item((*fg, *fi), (g, i))
                                    }
                                    SidebarDrag::Group(from, _) => this.layout.move_group(*from, g),
                                }
                                this.save_favourites();
                                cx.notify();
                            }))
                            .context_menu(move |menu, _, _| {
                                let remove = remove.clone();
                                menu.item(PopupMenuItem::new(remove_label.clone()).on_click(
                                    move |_, _, cx| {
                                        let _ = remove.update(cx, |this, cx| {
                                            this.layout.remove_item(g, i);
                                            this.save_favourites();
                                            cx.notify();
                                        });
                                    },
                                ))
                            }),
                        );
                    }
                }
                Group::Drives => side = self.drives_group(side, g, &header, &entry, cx),
            }
        }
        side.child(
            div().pt_3().child(
                Button::new("new-group")
                    .cursor_pointer()
                    .ghost()
                    .xsmall()
                    .icon(IconName::Plus)
                    .label(tr!("New group", "Nowa grupa"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        let g = this.layout.add_group(tr!("New group", "Nowa grupa"));
                        this.save_favourites();
                        this.start_group_rename(g, window, cx);
                    })),
            ),
        )
    }

    /// The Drives group: This PC, the Recycle Bin, the drives, and the
    /// network drive buttons on its header.
    fn drives_group(
        &self,
        mut side: Stateful<Div>,
        g: usize,
        header: &HeaderFn,
        entry: &EntryFn,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let muted = cx.theme().muted_foreground;
        let title = tr!("Drives", "Dyski");
        let head = header(g, title.clone(), cx);
        let drive_button = |id: &'static str, icon: IconName, tip: String, map: bool| {
            Button::new(id)
                .cursor_pointer()
                .ghost()
                .xsmall()
                .icon(icon)
                .tooltip(tip)
                .on_click(cx.listener(move |this, _, w, cx| this.drive_dialog(map, w, cx)))
        };
        side = side.child(
            head.child(div().flex_1().child(title))
                .child(drive_button(
                    "map-drive",
                    IconName::Plus,
                    tr!("Map network drive", "Podłącz dysk sieciowy"),
                    true,
                ))
                .child(drive_button(
                    "unmap-drive",
                    IconName::Minus,
                    tr!("Disconnect network drive", "Odłącz dysk sieciowy"),
                    false,
                )),
        );
        for (id, label, icon, place) in [
            (
                "this-pc",
                tr!("This PC", "Ten komputer"),
                "monitor",
                places::THIS_PC,
            ),
            (
                "bin",
                tr!("Recycle Bin", "Kosz"),
                "trash",
                places::RECYCLE_BIN,
            ),
        ] {
            let icon = Icon::empty().path(format!("icons/{icon}.svg"));
            side = side.child(entry(id.into(), label, icon, place.into(), cx));
        }
        for (i, (d, share)) in self.drives.iter().enumerate() {
            let icon = if share.is_some() {
                IconName::Network
            } else {
                IconName::HardDrive
            };
            let row = entry(
                format!("drive-{i}").into(),
                d.display().to_string(),
                Icon::new(icon),
                d.clone(),
                cx,
            );
            // The share is secondary: muted, after the letter.
            side = side.child(match share {
                Some(s) => row.child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(muted)
                        .child(s.clone()),
                ),
                None => row,
            });
        }
        side
    }

    /// A group's name becomes editable; Enter or clicking away keeps it.
    fn start_group_rename(&mut self, g: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Group::Folders { name, .. }) = self.layout.0.get(g) else {
            return;
        };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name.clone()));
        cx.subscribe_in(
            &input,
            window,
            move |this, input, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let name = input.read(cx).value().trim().to_string();
                    if !name.is_empty() {
                        this.layout.rename_group(g, name);
                        this.save_favourites();
                    }
                    this.editing_group = None;
                    this.focus_active(window, cx);
                }
            },
        )
        .detach();
        input.update(cx, |i, cx| {
            i.focus(window, cx);
            i.select_all(window, cx);
        });
        self.editing_group = Some((g, input));
        cx.notify();
    }

    /// Windows' map / disconnect network drive dialog; the drive list is
    /// read again when it changed something.
    fn drive_dialog(&self, map: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(h) = explorer::hwnd(window) else {
            return;
        };
        // The dialog runs its own message loop, so not inside GPUI's dispatch.
        cx.spawn(async move |this, cx| {
            let changed = if map {
                files_win::shell::map_drive_dialog(h)
            } else {
                files_win::shell::disconnect_drive_dialog(h)
            };
            if changed {
                let _ = this.update(cx, |w, cx| {
                    w.drives = drives();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn pane_view(&self, pane_ix: usize, cx: &mut Context<Self>) -> Div {
        let pane = &self.panes[pane_ix];
        let focused = pane_ix == self.active_pane && self.panes.len() > 1;
        let mut bar = TabBar::new(("tabs", pane_ix))
            .selected_index(pane.active)
            .on_click(cx.listener(move |this, ix: &usize, window, cx| {
                // A click on a tab's close button reaches the tab too, after
                // the close: that tab, or its whole pane, may be gone.
                let Some(pane) = this.panes.get_mut(pane_ix) else {
                    return;
                };
                pane.active = (*ix).min(pane.tabs.len() - 1);
                this.active_pane = pane_ix;
                this.focus_active(window, cx);
            }));
        for (tab_ix, tab) in pane.tabs.iter().enumerate() {
            let close = Button::new(("close", pane_ix * 1000 + tab_ix))
                .cursor_pointer()
                .ghost()
                .xsmall()
                .icon(IconName::Close)
                .on_click(
                    cx.listener(move |this, _, window, cx| this.close(pane_ix, tab_ix, window, cx)),
                );
            bar = bar.child(
                Tab::new()
                    .cursor_pointer()
                    .label(tab.read(cx).title())
                    .suffix(close)
                    // Dragged onto a pane's files: moves there, or splits.
                    .on_drag(
                        DraggedTab {
                            pane: pane_ix,
                            ix: tab_ix,
                            title: tab.read(cx).title(),
                        },
                        |d, _, _, cx| cx.new(|_| d.clone()),
                    )
                    // Like a browser: the middle button closes a tab.
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, window, cx| {
                            this.close(pane_ix, tab_ix, window, cx)
                        }),
                    ),
            );
        }
        let drop_bg = cx.theme().drag_border.opacity(0.15);
        let border = if focused {
            cx.theme().primary
        } else {
            cx.theme().border
        };
        div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .border_t_2()
            .border_color(if focused {
                border
            } else {
                gpui_kit::transparent_black()
            })
            // Clicking anywhere in a pane makes it the active one.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if this.active_pane != pane_ix {
                        this.active_pane = pane_ix;
                        cx.notify();
                    }
                }),
            )
            .child(div().px_3().child(bar))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .rounded(cx.theme().radius_lg)
                    .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(drop_bg))
                    .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                        this.move_tab(d.pane, d.ix, pane_ix, window, cx)
                    }))
                    .child(pane.current().clone()),
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Drops go to the active tab: its folder, and its status/reload.
        let active = self.active().read(cx);
        *self.drop_dir.lock().unwrap() = active.current_dir().to_path_buf();
        *self.drop_sink.lock().unwrap() = Some(Box::new(active.drop_sink()));
        let current = active.current_dir().to_path_buf();
        let is_favourite = self.layout.contains(Path::new(&current));

        let dark = prefs::get(cx).dark;
        let title =
            TitleBar::new()
                .on_close_window(cx.listener(|this, _, window, cx| {
                    this.save_session(cx);
                    window.remove_window();
                }))
                // gpui-kit already indents 12 px; the logo sits in it, no extra padding.
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            Icon::empty()
                                .path("icons/app.svg")
                                .text_color(cx.theme().primary),
                        )
                        .child("FilesProject"),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_1()
                        .pr_2()
                        // The title bar starts a window move on mouse down + move, and that
                        // swallows the mouse up, so button clicks never finished.
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .children(self.update_button(cx))
                        .child(
                            Button::new("fav")
                                .cursor_pointer()
                                .ghost()
                                .icon(if is_favourite {
                                    IconName::StarFill
                                } else {
                                    IconName::Star
                                })
                                .tooltip(tr!("Favourite (Ctrl+D)", "Ulubiony (Ctrl+D)"))
                                .on_click(cx.listener(|this, _, w, cx| {
                                    this.toggle_favourite(&ToggleFavourite, w, cx)
                                })),
                        )
                        .child(
                            Button::new("split")
                                .cursor_pointer()
                                .ghost()
                                .icon(IconName::PanelRight)
                                .tooltip(tr!("Split panes (F6)", "Podziel panele (F6)"))
                                .on_click(cx.listener(|this, _, w, cx| {
                                    this.toggle_split(&ToggleSplit, w, cx)
                                })),
                        )
                        .child(
                            Button::new("mode")
                                .cursor_pointer()
                                .ghost()
                                .icon(if dark { IconName::Sun } else { IconName::Moon })
                                .tooltip(tr!("Light / dark", "Jasny / ciemny"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    prefs::set(cx, |s| s.dark = !s.dark);
                                    // Back to the file list, so its shortcuts work again.
                                    this.focus_active(window, cx);
                                })),
                        )
                        .child(
                            Button::new("settings")
                                .cursor_pointer()
                                .ghost()
                                .icon(IconName::Settings)
                                .tooltip(tr!("Settings", "Ustawienia"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_settings(window, cx)
                                })),
                        ),
                );

        // flex_row, not h_flex: h_flex centres children vertically instead of
        // stretching them, which collapsed the panes.
        let panes = div().flex().flex_row().flex_1().min_h_0().min_w_0(); // panes share the width instead of pushing past the window
                                                                          // Two panes: a handle between them sets their widths.
        let panes = if self.panes.len() > 1 {
            panes.child(
                h_resizable("panes")
                    .child(resizable_panel().child(self.pane_view(0, cx)))
                    .child(resizable_panel().child(self.pane_view(1, cx))),
            )
        } else {
            panes.child(self.pane_view(0, cx))
        };

        div()
            .v_flex()
            .size_full()
            .key_context(CONTEXT)
            // Files dragged to the window's edge leave it: Windows' own drag
            // takes over, to Explorer, the desktop or any program.
            .on_drag_move(|e: &DragMoveEvent<DraggedFiles>, window, cx| {
                let (pos, size) = (e.event.position, window.viewport_size());
                let edge = px(4.);
                let leaving = pos.x < edge
                    || pos.y < edge
                    || pos.x > size.width - edge
                    || pos.y > size.height - edge;
                if !leaving {
                    return;
                }
                let d = e.drag(cx);
                let (paths, source) = (d.paths().to_vec(), d.source());
                cx.stop_active_drag(window);
                // A modal loop: outside GPUI's event dispatch.
                cx.spawn(async move |_: &mut AsyncApp| {
                    if let Ok(true) = files_win::drag::drag_out(&paths) {
                        source.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                })
                .detach();
            })
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::prev_tab))
            .on_action(cx.listener(Self::toggle_split))
            .on_action(cx.listener(Self::other_pane))
            .on_action(cx.listener(Self::toggle_favourite))
            .on_action(cx.listener(Self::open_palette))
            .on_action(
                cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleDark, window, cx| {
                prefs::set(cx, |s| s.dark = !s.dark);
                this.focus_active(window, cx);
            }))
            .text_color(cx.theme().foreground)
            .child(title)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(self.sidebar(cx))
                    .child(panes),
            )
            // Copies, moves and deletes in progress, across all tabs.
            .children(crate::ops::bar(cx))
    }
}

fn drives() -> Vec<(PathBuf, Option<String>)> {
    files_win::shell::drives()
        .into_iter()
        .map(|d| {
            let share = files_win::shell::network_path(&d);
            (d, share)
        })
        .collect()
}

/// A tab being dragged, and its drag image.
#[derive(Clone)]
struct DraggedTab {
    pane: usize,
    ix: usize,
    title: String,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_1()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_color(cx.theme().foreground)
            .child(self.title.clone())
    }
}

/// Builds a sidebar group header (index, name).
type HeaderFn = dyn Fn(usize, String, &mut Context<Workspace>) -> Stateful<Div>;
/// Builds a sidebar entry (id, label, icon, where it goes).
type EntryFn =
    dyn Fn(SharedString, String, Icon, PathBuf, &mut Context<Workspace>) -> Stateful<Div>;

/// A sidebar item (group, index) or a whole group being dragged, with
/// its label as the drag image.
#[derive(Clone)]
enum SidebarDrag {
    Item(usize, usize, String),
    Group(usize, String),
}

impl Render for SidebarDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (SidebarDrag::Item(_, _, label) | SidebarDrag::Group(_, label)) = self;
        div()
            .px_3()
            .py_1()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_color(cx.theme().foreground)
            .child(label.clone())
    }
}
