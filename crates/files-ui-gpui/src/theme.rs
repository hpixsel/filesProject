//! One accent hue drives every panel colour (CLAUDE.md: "kolor przewodni").

use files_app::settings::Settings;
use gpui_kit::component::Size;
use gpui_kit::component::*;
use gpui_kit::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Quick picks in the colour picker, as hues in degrees (saturation 0.85).
pub const PRESETS: [f32; 5] = [217., 262., 152., 25., 330.];

/// Normal, compact, very compact: (base font size, which also scales every
/// rem-based padding, and the file table's size). The table's own sizes keep
/// text and cell padding matched to the row; `Size::Size` drops both.
const DENSITY: [(f32, Size); 3] = [(16., Size::Medium), (14., Size::Small), (13., Size::XSmall)];

static ROWS: AtomicUsize = AtomicUsize::new(0);

/// File table size for the density last applied.
pub fn table_size() -> Size {
    DENSITY[ROWS.load(Ordering::Relaxed)].1
}

/// The accent the settings ask for. "Wallpaper" is the Windows accent colour,
/// read now (shortcut: read on start and on click, not when the wallpaper
/// changes; listen for WM_SETTINGCHANGE if that matters).
pub fn accent(s: &Settings) -> Hsla {
    let preset = hsla(PRESETS[0] / 360., 0.85, 0.6, 1.);
    match s.accent {
        Some((h, sat)) => hsla(h / 360., sat, 0.6, 1.),
        None => files_win::shell::accent_colour()
            .map(|c| rgb(c).into())
            .unwrap_or(preset),
    }
}

/// Applies the mode, the density, and a palette derived from the accent's
/// hue and saturation. Only the window background is translucent, so the
/// blurred backdrop shows through without alphas stacking up.
pub fn apply(s: &Settings, window: Option<&mut Window>, cx: &mut App) {
    let mode = if s.dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    Theme::change(mode, window, cx); // resets the colours set below
    files_win::shell::set_menu_dark(s.dark);
    let accent = accent(s);
    // Presets have saturation 0.85, so for them the values below stay as tuned.
    let (h, k, dark) = (accent.h, accent.s / 0.85, s.dark);
    let theme = Theme::global_mut(cx);
    let c = |sat: f32, l: f32, a: f32| hsla(h, (sat * k).min(1.), l, a);
    let l = |d: f32, li: f32| if dark { d } else { li };
    let t = &mut theme.colors;

    t.background = c(0.14, l(0.07, 0.97), l(0.55, 0.6));
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
    t.tab_bar = c(0., 0., 0.);
    t.tab = c(0., 0., 0.);
    t.tab_active = c(0.7, 0.6, l(0.16, 0.12));
    t.tab_foreground = t.muted_foreground;
    t.tab_active_foreground = t.foreground;
    t.title_bar_border = c(0., 0., 0.);
    t.table = c(0., 0., 0.);
    t.table_head = c(0.14, 0.5, 0.06);
    t.table_head_foreground = t.muted_foreground;
    t.table_even = c(0.14, 0.5, 0.03);
    t.table_hover = c(0.7, 0.6, l(0.10, 0.08));
    t.table_active = c(0.8, 0.6, l(0.24, 0.18));
    t.table_active_border = primary;
    t.list_active = t.table_active;
    t.list_active_border = primary;
    t.list_hover = t.table_hover;

    // Components read tokens derived from the colours, not the colours.
    theme.tokens = ThemeTokens::from(theme.colors);
    theme.radius = px(8.);
    theme.radius_lg = px(12.);
    theme.shadow = true;
    theme.font_family = "Segoe UI Variable Text".into();
    theme.font_size = px(DENSITY[s.density].0);
    ROWS.store(s.density, Ordering::Relaxed);
}
