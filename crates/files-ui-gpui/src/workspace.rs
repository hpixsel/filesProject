//! The window: title bar, sidebar (favourites, drives), one or two panes,
//! each pane with its own tabs. Every tab is an `Explorer`.

use crate::explorer::{self, Explorer};
use crate::i18n::{self, tr};
use crate::theme;
use files_app::favourites;
use files_app::settings::{self, Settings};
use files_app::update;
use gpui_kit::component::button::*;
use gpui_kit::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
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
        ToggleFavourite
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
    favourites: Vec<PathBuf>,
    /// Drive roots, with the share for mapped network drives.
    drives: Vec<(PathBuf, Option<String>)>,
    settings: Settings,
    picker: Entity<ColorPickerState>,
    /// Read by the window's drop target at drop time: the active tab's folder…
    drop_dir: Arc<Mutex<PathBuf>>,
    /// …and where that drop's outcome goes (the active tab).
    drop_sink: Arc<Mutex<Option<DropSink>>>,
    focus: FocusHandle,
    update: Arc<Mutex<Update>>,
}

impl Workspace {
    pub fn new(
        start: PathBuf,
        settings: Settings,
        incoming: std::sync::mpsc::Receiver<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let first = cx.new(|cx| Explorer::new(start, window, cx));
        let accent = theme::accent(&settings);
        let picker = cx.new(|cx| ColorPickerState::new(window, cx).default_value(accent));
        cx.subscribe_in(
            &picker,
            window,
            |this, _, e: &ColorPickerEvent, window, cx| {
                if let ColorPickerEvent::Change(Some(c)) = e {
                    this.settings.accent = Some((c.h * 360., c.s));
                    this.change(window, cx);
                }
            },
        )
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
        let this = Workspace {
            panes: vec![Pane {
                tabs: vec![first],
                active: 0,
            }],
            active_pane: 0,
            favourites: favourites::file()
                .map(|f| favourites::load(&f))
                .unwrap_or_default(),
            drives: drives(),
            settings,
            picker,
            drop_dir: Default::default(),
            drop_sink: Default::default(),
            focus: cx.focus_handle(),
            update: Default::default(),
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
        this.focus_active(window, cx);
        this
    }

    fn active(&self) -> &Entity<Explorer> {
        self.panes[self.active_pane].current()
    }

    fn focus_active(&self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.active().read(cx).focus_handle();
        focus.focus(window, cx);
        cx.notify();
    }

    fn open_tab(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let tab = cx.new(|cx| Explorer::new(dir, window, cx));
        let pane = &mut self.panes[self.active_pane];
        pane.tabs.insert(pane.active + 1, tab);
        pane.active += 1;
        self.focus_active(window, cx);
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
            let tab = cx.new(|cx| Explorer::new(dir, window, cx));
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
        favourites::toggle(&mut self.favourites, &dir);
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

    /// Applies and saves `self.settings` after a title-bar change.
    fn change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        i18n::set_polish(
            self.settings
                .polish
                .unwrap_or_else(files_win::shell::ui_is_polish),
        );
        theme::apply(&self.settings, Some(window), cx);
        if let Some(f) = settings::file() {
            if let Err(e) = settings::save(&f, &self.settings) {
                eprintln!("cannot save settings: {e}");
            }
        }
        // Back to the file list, so its shortcuts (Ctrl+Alt+T, ...) work again.
        self.focus_active(window, cx);
        window.refresh();
    }

    fn save_favourites(&self) {
        if let Some(f) = favourites::file() {
            if let Err(e) = favourites::save(&f, &self.favourites) {
                eprintln!("cannot save favourites: {e}");
            }
        }
    }

    fn go_to(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.active().update(cx, |e, cx| e.go_to(dir, window, cx));
        self.focus_active(window, cx);
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let heading = |text: String| {
            div()
                .px_2()
                .pt_3()
                .pb_1()
                .text_xs()
                .text_color(muted)
                .child(text)
        };
        let (hover, icon_color, radius) = (
            cx.theme().table_hover,
            cx.theme().primary,
            cx.theme().radius,
        );
        let entry = |id: SharedString,
                     label: String,
                     icon: IconName,
                     path: PathBuf,
                     cx: &mut Context<Self>| {
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
                .child(Icon::new(icon).text_color(icon_color))
                .child(div().truncate().child(label))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.go_to(path.clone(), window, cx)),
                )
        };
        let mut side = div()
            .v_flex()
            .w(px(200.))
            .flex_shrink_0()
            .px_2()
            .child(heading(tr!("Favourites", "Ulubione")));
        if self.favourites.is_empty() {
            side = side.child(div().px_2().text_sm().text_color(muted).child(tr!(
                "Ctrl+D adds the current folder",
                "Ctrl+D dodaje bieżący folder"
            )));
        }
        for (i, p) in self.favourites.iter().enumerate() {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string());
            side = side.child(entry(
                format!("fav-{i}").into(),
                name,
                IconName::Star,
                p.clone(),
                cx,
            ));
        }
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
            div()
                .flex()
                .flex_row()
                .items_end()
                .child(heading(tr!("Drives", "Dyski")).flex_1())
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
        for (i, (d, share)) in self.drives.iter().enumerate() {
            let icon = if share.is_some() {
                IconName::Network
            } else {
                IconName::HardDrive
            };
            let row = entry(
                format!("drive-{i}").into(),
                d.display().to_string(),
                icon,
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
                this.active_pane = pane_ix;
                this.panes[pane_ix].active = *ix;
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
                    .suffix(close),
            );
        }
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
        let is_favourite = self
            .favourites
            .iter()
            .any(|p| p.as_path() == Path::new(&current));

        let dark = self.settings.dark;
        let density = [
            tr!("Density: normal", "Gęstość: zwykła"),
            tr!("Density: compact", "Gęstość: kompaktowa"),
            tr!("Density: very compact", "Gęstość: bardzo kompaktowa"),
        ][self.settings.density]
            .clone();
        let presets = theme::PRESETS
            .iter()
            .map(|h| hsla(h / 360., 0.85, 0.6, 1.))
            .collect();
        let title =
            TitleBar::new()
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
                                    this.settings.dark = !this.settings.dark;
                                    this.change(window, cx);
                                })),
                        )
                        .child(
                            ColorPicker::new(&self.picker)
                                .icon(IconName::Palette)
                                .featured_colors(presets)
                                .anchor(Anchor::TopRight),
                        )
                        .child(
                            Button::new("wallpaper")
                                .cursor_pointer()
                                .ghost()
                                .icon(IconName::Frame)
                                .tooltip(tr!(
                                    "Accent from Windows (from the wallpaper when Windows picks it automatically)",
                                    "Kolor z Windows (z tapety, gdy Windows dobiera go automatycznie)"
                                ))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.settings.accent = None;
                                    let shown = theme::accent(&this.settings);
                                    this.picker
                                        .update(cx, |p, cx| p.set_value(shown, window, cx));
                                    this.change(window, cx);
                                })),
                        )
                        .child(
                            Button::new("density")
                                .cursor_pointer()
                                .ghost()
                                .icon(IconName::ALargeSmall)
                                .tooltip(density)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.settings.density = (this.settings.density + 1) % 3;
                                    this.change(window, cx);
                                })),
                        )
                        .child(
                            Button::new("lang")
                                .cursor_pointer()
                                .ghost()
                                .icon(IconName::Globe)
                                .label(if i18n::polish() { "PL" } else { "EN" })
                                .tooltip(tr!("Language", "Język"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.settings.polish = Some(!i18n::polish());
                                    this.change(window, cx);
                                })),
                        ),
                );

        // flex_row, not h_flex: h_flex centres children vertically instead of
        // stretching them, which collapsed the panes.
        let mut panes = div()
            .flex()
            .flex_row()
            .flex_1()
            .min_h_0()
            .min_w_0() // panes share the width instead of pushing past the window
            .gap_2();
        for i in 0..self.panes.len() {
            panes = panes.child(self.pane_view(i, cx));
        }

        div()
            .v_flex()
            .size_full()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::prev_tab))
            .on_action(cx.listener(Self::toggle_split))
            .on_action(cx.listener(Self::other_pane))
            .on_action(cx.listener(Self::toggle_favourite))
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
