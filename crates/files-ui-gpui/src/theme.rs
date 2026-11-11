//! One accent hue drives every panel colour (CLAUDE.md: "kolor przewodni").

use gpui_kit::component::*;
use gpui_kit::*;

/// (name, hue in degrees)
pub const ACCENTS: [(&str, f32); 5] = [
    ("Blue", 217.),
    ("Violet", 262.),
    ("Green", 152.),
    ("Orange", 25.),
    ("Pink", 330.),
];

/// Derives the whole palette from `hue` for the current mode. Only the
/// window background is translucent, so the blurred backdrop shows through
/// without alphas stacking up.
pub fn apply(hue: f32, cx: &mut App) {
    let h = hue / 360.;
    let theme = Theme::global_mut(cx);
    let dark = theme.mode.is_dark();
    let c = |s: f32, l: f32, a: f32| hsla(h, s, l, a);
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
    t.title_bar_border = c(0., 0., 0.);
    t.table = c(0., 0., 0.);
    t.table_head = c(0.14, 0.5, 0.06);
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
}
