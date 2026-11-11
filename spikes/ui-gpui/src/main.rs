//! M1 spike: 100k-row virtual table, accent-derived palette on Windows 11 Mica.

use gpui_kit::component::button::*;
use gpui_kit::component::table::*;
use gpui_kit::component::*;
use gpui_kit::*;
use std::time::Duration;

mod win;

const ROWS: usize = 100_000;
/// The shell menu always targets a real file so its entries are meaningful.
const MENU_FILE: &str = r"C:\Windows\notepad.exe";
/// (name, hue in degrees)
const ACCENTS: [(&str, f32); 5] = [
    ("Blue", 217.),
    ("Violet", 262.),
    ("Green", 152.),
    ("Orange", 25.),
    ("Pink", 330.),
];

/// Every panel colour is derived from one accent hue, tinted for the mode.
/// Backgrounds are slightly transparent so the Mica backdrop shows through.
fn apply_palette(hue: f32, opaque: bool, cx: &mut App) {
    let h = hue / 360.;
    let theme = Theme::global_mut(cx);
    let dark = theme.mode.is_dark();
    let c = |s: f32, l: f32, a: f32| hsla(h, s, l, a);
    // (lightness in dark mode, lightness in light mode)
    let l = |d: f32, li: f32| if dark { d } else { li };
    let t = &mut theme.colors;

    // Only this layer is translucent; everything above it is transparent, so
    // alphas do not stack up and hide the Mica backdrop.
    t.background = c(0.14, l(0.07, 0.97), if opaque { 1. } else { l(0.55, 0.6) });
    t.foreground = c(0.10, l(0.94, 0.13), 1.);
    t.muted = c(0.12, l(0.15, 0.93), 1.);
    t.muted_foreground = c(0.08, l(0.64, 0.42), 1.);
    t.secondary = c(0.12, l(0.17, 0.93), 0.9);
    t.border = c(0.14, l(0.21, 0.87), 1.);
    t.input = c(0.14, l(0.24, 0.84), 1.);
    t.popover = c(0.14, l(0.11, 0.99), 0.98);

    let primary = c(0.85, l(0.62, 0.52), 1.);
    t.primary = primary;
    t.primary_hover = c(0.85, l(0.68, 0.47), 1.);
    t.primary_active = c(0.85, l(0.56, 0.42), 1.);
    t.primary_foreground = c(0.2, l(0.08, 0.99), 1.);
    t.button_primary = t.primary;
    t.button_primary_hover = t.primary_hover;
    t.button_primary_active = t.primary_active;
    t.button_primary_foreground = t.primary_foreground;
    t.ring = primary;
    t.accent = c(0.7, 0.6, l(0.16, 0.12));
    t.accent_foreground = t.foreground;
    t.selection = c(0.85, 0.6, 0.35);
    t.drag_border = primary;

    t.title_bar = c(0., 0., 0.);
    t.title_bar_border = c(0., 0., 0.);
    t.table = c(0., 0., 0.);
    t.table_head = c(0.14, l(0.5, 0.5), 0.06);
    t.table_even = c(0.14, l(0.5, 0.5), 0.03);
    t.table_hover = c(0.7, 0.6, l(0.10, 0.08));
    t.table_active = c(0.8, 0.6, l(0.24, 0.18));
    t.table_active_border = primary;
    t.list_active = t.table_active;
    t.list_active_border = primary;
    t.list_hover = t.table_hover;

    // Components read tokens derived from the colours, not the colours themselves.
    theme.tokens = ThemeTokens::from(theme.colors);
    theme.radius = px(8.);
    theme.radius_lg = px(12.);
    theme.shadow = true;
    theme.font_family = "Segoe UI Variable Text".into();
}

struct Files {
    log: win::Log,
}

impl TableDelegate for Files {
    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let log = self.log.clone();
        div()
            .id(("row", row_ix))
            .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                let Some(hwnd) = win::hwnd_of(window) else { return };
                let log = log.clone();
                // TrackPopupMenuEx runs a modal message loop. Running it inside
                // GPUI's event dispatch re-enters GPUI while its state is
                // borrowed and panics, so open the menu from a task instead,
                // outside any App borrow.
                cx.spawn(async move |_: &mut AsyncApp| {
                    let line = match win::show_shell_menu(hwnd, MENU_FILE) {
                        Ok(Some(id)) => format!("Shell menu: ran command {id} on {MENU_FILE}"),
                        Ok(None) => "Shell menu: closed without a choice".to_string(),
                        Err(e) => format!("Shell menu error: {e}"),
                    };
                    log.lock().unwrap().push(line);
                })
                .detach();
            })
    }
    fn columns_count(&self, _: &App) -> usize {
        3
    }
    fn rows_count(&self, _: &App) -> usize {
        ROWS
    }
    fn column(&self, col_ix: usize, _: &App) -> Column {
        match col_ix {
            0 => Column::new("name", "Name").width(px(380.)),
            1 => Column::new("size", "Size").width(px(110.)),
            _ => Column::new("modified", "Modified").width(px(170.)),
        }
    }
    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        match col_ix {
            0 => div().child(format!("file_{row_ix:06}.txt")),
            1 => div().text_color(muted).child(format!("{} KB", (row_ix * 37) % 9_000 + 1)),
            _ => div()
                .text_color(muted)
                .child(format!("2026-11-{:02} 12:{:02}", row_ix % 28 + 1, row_ix % 60)),
        }
    }
}

const BACKDROPS: [(&str, WindowBackgroundAppearance); 4] = [
    ("Mica", WindowBackgroundAppearance::MicaBackdrop),
    ("Mica Alt", WindowBackgroundAppearance::MicaAltBackdrop),
    ("Blurred", WindowBackgroundAppearance::Blurred),
    ("Opaque", WindowBackgroundAppearance::Opaque),
];

struct Spike {
    table: Entity<TableState<Files>>,
    accent: usize,
    backdrop: usize,
    log: win::Log,
}

impl Spike {
    fn opaque(&self) -> bool {
        matches!(BACKDROPS[self.backdrop].1, WindowBackgroundAppearance::Opaque)
    }
}

impl Render for Spike {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = cx.theme().mode.is_dark();
        let accent = self.accent;
        let hue = ACCENTS[accent].1;
        let opaque = self.opaque();
        div()
            .v_flex()
            .size_full()
            .text_color(cx.theme().foreground)
            .child(TitleBar::new().child(div().pl_2().child("FilesProject")))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .px_3()
                    .pb_2()
                    .child(
                        Button::new("mode")
                            .ghost()
                            .label(if dark { "Light mode" } else { "Dark mode" })
                            .on_click(move |_, window, cx| {
                                let mode = if dark { ThemeMode::Light } else { ThemeMode::Dark };
                                Theme::change(mode, Some(window), cx);
                                win::set_menu_dark(!dark);
                                apply_palette(hue, opaque, cx); // change() resets the colours
                            }),
                    )
                    .child(
                        Button::new("accent")
                            .primary()
                            .label(format!("Accent: {}", ACCENTS[accent].0))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.accent = (accent + 1) % ACCENTS.len();
                                apply_palette(ACCENTS[this.accent].1, this.opaque(), cx);
                                window.refresh();
                            })),
                    )
                    .child(
                        Button::new("backdrop")
                            .outline()
                            .label(format!("Backdrop: {}", BACKDROPS[self.backdrop].0))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.backdrop = (this.backdrop + 1) % BACKDROPS.len();
                                window.set_background_appearance(BACKDROPS[this.backdrop].1);
                                apply_palette(ACCENTS[this.accent].1, this.opaque(), cx);
                                window.refresh();
                            })),
                    )
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{ROWS} rows")),
                    ),
            )
            .child(
                div().px_3().pb_2().text_sm().text_color(cx.theme().muted_foreground).child(
                    self.log.lock().unwrap().last().cloned().unwrap_or_else(|| {
                        "Drag files here from Explorer (also from a .zip); right-click a row for the Windows menu."
                            .to_string()
                    }),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .mx_3()
                    .mb_3()
                    .rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(cx.theme().border)
                    .overflow_hidden()
                    .child(DataTable::new(&self.table)),
            )
    }
}

fn main() {
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        win::set_menu_dark(true);
        apply_palette(ACCENTS[0].1, false, cx);
        let options = WindowOptions {
            titlebar: Some(TitleBar::title_bar_options()),
            window_background: WindowBackgroundAppearance::Blurred,
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            let log = win::Log::default();
            if let Some(hwnd) = win::hwnd_of(window) {
                if let Err(e) = win::register_drop(hwnd, log.clone()) {
                    log.lock().unwrap().push(format!("RegisterDragDrop failed: {e}"));
                }
            }
            let table = cx.new(|cx| TableState::new(Files { log: log.clone() }, window, cx));
            cx.new(|cx| {
                // COM callbacks cannot reach GPUI, so repaint a few times a second.
                cx.spawn(async move |this: WeakEntity<Spike>, cx: &mut AsyncApp| loop {
                    cx.background_executor().timer(Duration::from_millis(250)).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                })
                .detach();
                Spike { table, accent: 0, backdrop: 2, log }
            })
        })
        .expect("open window");
    });
}
