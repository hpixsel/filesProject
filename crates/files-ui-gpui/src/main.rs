//! FilesProject window (M1): one pane, navigation, Explorer context menu and
//! Ctrl+V paste from an RDP session through a child `rdppaste --machine`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod explorer;
mod theme;

use gpui_kit::component::*;
use gpui_kit::*;
use std::path::PathBuf;

fn main() {
    let start = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(r"C:\"));

    // Icons (toolbar, file list, window caption buttons) live in these assets.
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            explorer::bind_keys(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            files_win::shell::set_menu_dark(true);
            theme::apply(theme::ACCENTS[0].1, cx);
            let options = WindowOptions {
                titlebar: Some(TitleBar::title_bar_options()),
                window_background: WindowBackgroundAppearance::Blurred,
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| explorer::Explorer::new(start, window, cx))
            })
            .expect("open window");
        });
}
