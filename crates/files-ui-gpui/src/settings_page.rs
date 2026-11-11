//! The settings panel: appearance (mode, accent, density and its text sizes)
//! and general options (language, remembered tabs, Explorer replacement).
//! Every field reads and writes `prefs`, which applies and saves at once.

use crate::i18n::tr;
use crate::{prefs, theme};
use files_app::{replace_explorer, update};
use gpui_kit::component::button::*;
use gpui_kit::component::color_picker::{ColorPicker, ColorPickerState};
use gpui_kit::component::setting::*;
use gpui_kit::component::*;
use gpui_kit::*;

/// Opens the panel on the right of `window`. `picker` is the workspace's
/// accent picker, kept alive across openings.
pub fn open(picker: Entity<ColorPickerState>, window: &mut Window, cx: &mut App) {
    window.open_sheet(cx, move |sheet, _, _| {
        sheet
            .title(tr!("Settings", "Ustawienia"))
            .size(px(640.))
            .child(page(picker.clone()))
    });
}

fn options(pairs: &[(&str, String)]) -> Vec<(SharedString, SharedString)> {
    pairs
        .iter()
        .map(|(v, l)| ((*v).into(), l.clone().into()))
        .collect()
}

fn page(picker: Entity<ColorPickerState>) -> Settings {
    let font = |ix: usize, title: String| {
        SettingItem::new(
            title,
            SettingField::number_input(
                NumberFieldOptions {
                    min: 10.,
                    max: 24.,
                    step: 1.,
                },
                move |cx| prefs::get(cx).fonts[ix] as f64,
                move |v, cx| prefs::set(cx, |s| s.fonts[ix] = v as f32),
            ),
        )
    };
    let appearance = SettingPage::new(tr!("Appearance", "Wygląd"))
        .group(SettingGroup::new().title(tr!("Theme", "Motyw")).items([
            SettingItem::new(
                tr!("Mode", "Tryb"),
                SettingField::dropdown(
                    options(&[
                        ("dark", tr!("Dark", "Ciemny")),
                        ("light", tr!("Light", "Jasny")),
                    ]),
                    |cx| if prefs::get(cx).dark { "dark" } else { "light" }.into(),
                    |v, cx| prefs::set(cx, |s| s.dark = v == "dark"),
                ),
            ),
            SettingItem::render(move |_, _, cx| accent_row(&picker, cx)),
        ]))
        .group(
            SettingGroup::new()
                .title(tr!("Density", "Gęstość"))
                .description(tr!(
                    "Text size for each level; paddings scale with it.",
                    "Rozmiar tekstu dla każdego poziomu; odstępy skalują się razem z nim."
                ))
                .items([
                    SettingItem::new(
                        tr!("Density", "Gęstość"),
                        SettingField::dropdown(
                            options(&[
                                ("0", tr!("Normal", "Zwykła")),
                                ("1", tr!("Compact", "Kompaktowa")),
                                ("2", tr!("Very compact", "Bardzo kompaktowa")),
                            ]),
                            |cx| prefs::get(cx).density.to_string().into(),
                            |v, cx| prefs::set(cx, |s| s.density = v.parse().unwrap_or(0)),
                        ),
                    ),
                    font(0, tr!("Text size: normal", "Rozmiar tekstu: zwykła")),
                    font(1, tr!("Text size: compact", "Rozmiar tekstu: kompaktowa")),
                    font(
                        2,
                        tr!(
                            "Text size: very compact",
                            "Rozmiar tekstu: bardzo kompaktowa"
                        ),
                    ),
                ]),
        );

    let installed = update::is_installed();
    let general = SettingPage::new(tr!("General", "Ogólne"))
        .group(
            SettingGroup::new().title(tr!("Language", "Język")).item(SettingItem::new(
                tr!("Interface language", "Język interfejsu"),
                SettingField::dropdown(
                    options(&[
                        ("system", tr!("Like Windows", "Jak w Windows")),
                        ("pl", "Polski".into()),
                        ("en", "English".into()),
                    ]),
                    |cx| match prefs::get(cx).polish {
                        None => "system".into(),
                        Some(true) => "pl".into(),
                        Some(false) => "en".into(),
                    },
                    |v, cx| {
                        prefs::set(cx, |s| {
                            s.polish = match v.as_ref() {
                                "pl" => Some(true),
                                "en" => Some(false),
                                _ => None,
                            }
                        })
                    },
                ),
            )),
        )
        .group(
            SettingGroup::new().title(tr!("Files", "Pliki")).item(
                SettingItem::new(
                    tr!("Show hidden files (Ctrl+H)", "Pokazuj ukryte pliki (Ctrl+H)"),
                    SettingField::switch(
                        |cx| prefs::get(cx).show_hidden,
                        |v, cx| prefs::set(cx, |s| s.show_hidden = v),
                    ),
                )
                .description(tr!(
                    "Hidden and system files, like desktop.ini.",
                    "Pliki ukryte i systemowe, np. desktop.ini."
                )),
            ),
        )
        .group(
            SettingGroup::new().title(tr!("Tabs", "Karty")).item(SettingItem::new(
                tr!("Reopen the last tabs at start", "Przywracaj ostatnie karty przy starcie"),
                SettingField::switch(
                    |cx| prefs::get(cx).restore_tabs,
                    |v, cx| prefs::set(cx, |s| s.restore_tabs = v),
                ),
            )),
        )
        .group(
            SettingGroup::new().title("Windows").item(
                SettingItem::new(
                    tr!("Open folders with FilesProject", "Otwieraj foldery w FilesProject"),
                    SettingField::switch(|_| replace_explorer::is_on(), set_explorer),
                )
                .description(if installed {
                    tr!(
                        "Folders, drives, Win+E, This PC and the Recycle Bin. Off restores Explorer.",
                        "Foldery, dyski, Win+E, Ten komputer i Kosz. Wyłączenie przywraca Eksplorator."
                    )
                } else {
                    tr!(
                        "Needs the installed copy: run files --install.",
                        "Wymaga zainstalowanej kopii: uruchom files --install."
                    )
                })
                .disabled(!installed),
            ),
        );

    Settings::new("settings").pages([appearance, general])
}

/// The accent: gpui-kit's picker with the presets, and the Windows accent
/// (from the wallpaper when Windows picks it automatically).
fn accent_row(picker: &Entity<ColorPickerState>, cx: &mut App) -> impl IntoElement {
    let presets = theme::PRESETS
        .iter()
        .map(|h| hsla(h / 360., 0.85, 0.6, 1.))
        .collect();
    let from_windows = prefs::get(cx).accent.is_none();
    let picker_for_click = picker.clone();
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(tr!("Accent colour", "Kolor przewodni"))
        .child(
            div()
                .flex()
                .flex_row()
                .gap_2()
                .child(
                    ColorPicker::new(picker)
                        .featured_colors(presets)
                        .anchor(Anchor::TopRight),
                )
                .child(
                    Button::new("accent-windows")
                        .cursor_pointer()
                        .outline()
                        .selected(from_windows)
                        .label(tr!("From Windows / wallpaper", "Z Windows / tapety"))
                        .on_click(move |_, window, cx| {
                            prefs::set(cx, |s| s.accent = None);
                            let shown = theme::accent(prefs::get(cx));
                            picker_for_click.update(cx, |p, cx| p.set_value(shown, window, cx));
                        }),
                ),
        )
}

/// Turns the Explorer replacement on or off; `reg.exe` runs off the UI
/// thread, then the panel shows the new state.
fn set_explorer(on: bool, cx: &mut App) {
    let task = cx.background_spawn(async move {
        if on {
            replace_explorer::replace()
        } else {
            replace_explorer::restore()
        }
    });
    cx.spawn(async move |cx| {
        if let Err(e) = task.await {
            eprintln!("Explorer replacement: {e}");
        }
        cx.update(|cx| cx.refresh_windows());
    })
    .detach();
}
