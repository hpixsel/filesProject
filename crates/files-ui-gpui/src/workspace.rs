//! The window: title bar, sidebar (favourites, drives), one or two panes,
//! each pane with its own tabs. Every tab is an `Explorer`.

use crate::explorer::{self, Explorer};
use crate::theme;
use files_app::favourites;
use gpui_kit::component::button::*;
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

pub struct Workspace {
    panes: Vec<Pane>,
    active_pane: usize,
    favourites: Vec<PathBuf>,
    drives: Vec<PathBuf>,
    accent: usize,
    /// Read by the window's drop target at drop time: the active tab's folder…
    drop_dir: Arc<Mutex<PathBuf>>,
    /// …and where that drop's outcome goes (the active tab).
    drop_sink: Arc<Mutex<Option<DropSink>>>,
    focus: FocusHandle,
}

impl Workspace {
    pub fn new(start: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let first = cx.new(|cx| Explorer::new(start, window, cx));
        let this = Workspace {
            panes: vec![Pane {
                tabs: vec![first],
                active: 0,
            }],
            active_pane: 0,
            favourites: favourites::file()
                .map(|f| favourites::load(&f))
                .unwrap_or_default(),
            drives: files_win::shell::drives(),
            accent: 0,
            drop_dir: Default::default(),
            drop_sink: Default::default(),
            focus: cx.focus_handle(),
        };
        if let Some(h) = explorer::hwnd(window) {
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
        let heading = |text: &'static str| {
            div()
                .px_2()
                .pt_3()
                .pb_1()
                .text_xs()
                .text_color(muted)
                .child(text)
        };
        let entry = |id: SharedString,
                     label: String,
                     icon: IconName,
                     path: PathBuf,
                     cx: &mut Context<Self>| {
            Button::new(id)
                .ghost()
                .w_full()
                .justify_start()
                .icon(icon)
                .label(label)
                .tooltip(path.display().to_string())
                .on_click(
                    cx.listener(move |this, _, window, cx| this.go_to(path.clone(), window, cx)),
                )
        };
        let mut side = div()
            .v_flex()
            .w(px(200.))
            .px_2()
            .child(heading("Favourites"));
        if self.favourites.is_empty() {
            side = side.child(
                div()
                    .px_2()
                    .text_sm()
                    .text_color(muted)
                    .child("Ctrl+D adds the current folder"),
            );
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
        side = side.child(heading("Drives"));
        for (i, d) in self.drives.iter().enumerate() {
            side = side.child(entry(
                format!("drive-{i}").into(),
                d.display().to_string(),
                IconName::HardDrive,
                d.clone(),
                cx,
            ));
        }
        side
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
                .ghost()
                .xsmall()
                .icon(IconName::Close)
                .on_click(
                    cx.listener(move |this, _, window, cx| this.close(pane_ix, tab_ix, window, cx)),
                );
            bar = bar.child(Tab::new().label(tab.read(cx).title()).suffix(close));
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
            .child(div().flex_1().child(pane.current().clone()))
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

        let dark = cx.theme().mode.is_dark();
        let hue = theme::ACCENTS[self.accent].1;
        let title =
            TitleBar::new()
                .child(div().pl_2().child("FilesProject"))
                .child(
                    div()
                        .h_flex()
                        .gap_1()
                        .pr_2()
                        .child(
                            Button::new("fav")
                                .ghost()
                                .icon(if is_favourite {
                                    IconName::StarFill
                                } else {
                                    IconName::Star
                                })
                                .tooltip("Favourite (Ctrl+D)")
                                .on_click(cx.listener(|this, _, w, cx| {
                                    this.toggle_favourite(&ToggleFavourite, w, cx)
                                })),
                        )
                        .child(
                            Button::new("split")
                                .ghost()
                                .icon(IconName::PanelRight)
                                .tooltip("Split panes (F6)")
                                .on_click(cx.listener(|this, _, w, cx| {
                                    this.toggle_split(&ToggleSplit, w, cx)
                                })),
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
                                    this.accent = (this.accent + 1) % theme::ACCENTS.len();
                                    theme::apply(theme::ACCENTS[this.accent].1, cx);
                                    window.refresh();
                                })),
                        ),
                );

        let mut panes = div().h_flex().flex_1().min_h_0().gap_2();
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
                    .h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.sidebar(cx))
                    .child(panes),
            )
    }
}
