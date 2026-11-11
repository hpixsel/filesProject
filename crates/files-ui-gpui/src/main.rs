//! FilesProject window: tabs and panes of folders, Explorer context menu and
//! Ctrl+V paste from an RDP session through a child `rdppaste --machine`.
// No console window, in debug builds too (they are what gets tested).
#![windows_subsystem = "windows"]

mod explorer;
mod i18n;
mod theme;
mod workspace;

use gpui_kit::component::*;
use gpui_kit::*;
use std::path::PathBuf;

fn main() {
    let start = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(r"C:\"));
    // `files .` from a terminal: relative to where it was typed.
    let start = std::path::absolute(&start).unwrap_or(start);

    // Already running: the folder opens as a tab there.
    if files_win::instance::send(&start) {
        return;
    }
    // Folders from later `files <folder>` calls; checked on the pipe thread,
    // since a network path can take seconds to answer.
    let (folders, incoming) = std::sync::mpsc::channel();
    if let Err(e) = files_win::instance::serve(move |dir| {
        if dir.is_dir() {
            let _ = folders.send(dir);
        }
    }) {
        eprintln!("another window serves `files <folder>`: {e}");
    }

    // Icons (toolbar, file list, window caption buttons) live in these assets.
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            explorer::bind_keys(cx);
            workspace::bind_keys(cx);
            let settings = files_app::settings::file()
                .map(|f| files_app::settings::load(&f))
                .unwrap_or_default();
            i18n::set_polish(
                settings
                    .polish
                    .unwrap_or_else(files_win::shell::ui_is_polish),
            );
            theme::apply(&settings, None, cx);
            let options = WindowOptions {
                titlebar: Some(TitleBar::title_bar_options()),
                // Mica gets DWM ready for a system backdrop; Workspace::new swaps it
                // for acrylic (files_win::shell::set_acrylic).
                window_background: WindowBackgroundAppearance::MicaBackdrop,
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, move |window, cx| {
                cx.new(|cx| workspace::Workspace::new(start, settings, incoming, window, cx))
            })
            .expect("open window");
        });
}
