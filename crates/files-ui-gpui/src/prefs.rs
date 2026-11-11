//! The user's settings as GPUI global state: the settings page and the
//! title bar read them, and every change is applied and saved at once.

use crate::{i18n, theme};
use files_app::settings::{self, Settings};
use gpui_kit::{App, Global};

struct Prefs(Settings);

impl Global for Prefs {}

/// Applies `s` (theme, density, language) and makes it the current settings.
pub fn init(s: Settings, cx: &mut App) {
    apply(&s, cx);
    cx.set_global(Prefs(s));
}

pub fn get(cx: &App) -> &Settings {
    &cx.global::<Prefs>().0
}

/// Changes the settings, applies them to every window and saves them.
pub fn set(cx: &mut App, change: impl FnOnce(&mut Settings)) {
    change(&mut cx.global_mut::<Prefs>().0);
    let s = get(cx).clone();
    apply(&s, cx);
    if let Some(f) = settings::file() {
        if let Err(e) = settings::save(&f, &s) {
            eprintln!("cannot save settings: {e}");
        }
    }
    cx.refresh_windows();
}

fn apply(s: &Settings, cx: &mut App) {
    i18n::set_polish(s.polish.unwrap_or_else(files_win::shell::ui_is_polish));
    theme::apply(s, cx);
}
