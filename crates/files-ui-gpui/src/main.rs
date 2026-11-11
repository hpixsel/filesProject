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

/// `files --install`: copy to the install folder and put it on PATH.
fn install() {
    files_win::shell::attach_console();
    let done = files_app::update::install().and_then(|dir| {
        files_win::shell::add_to_user_path(&dir).map_err(|e| e.to_string())?;
        Ok(dir)
    });
    match done {
        Ok(dir) => println!(
            "\nInstalled to {}. Open a new terminal and run `files`.",
            dir.display()
        ),
        Err(e) => eprintln!("\nInstall failed: {e}"),
    }
}

fn main() {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "--install") {
        return install();
    }
    // Started by the update of a window that is closing right now.
    let after_update = args.first().is_some_and(|a| a == "--after-update");
    if after_update {
        args.remove(0);
    }
    let start = args
        .into_iter()
        .next()
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(r"C:\"));
    // `files .` from a terminal: relative to where it was typed.
    let start = std::path::absolute(&start).unwrap_or(start);

    // Already running: the folder opens as a tab there.
    if !after_update && files_win::instance::send(&start) {
        return;
    }
    // Folders from later `files <folder>` calls; checked on the pipe thread,
    // since a network path can take seconds to answer.
    let (folders, incoming) = std::sync::mpsc::channel();
    let on_dir = move |dir: PathBuf| {
        if dir.is_dir() {
            let _ = folders.send(dir);
        }
    };
    if after_update {
        // The old window still holds the pipe for a moment.
        std::thread::spawn(move || {
            let on_dir = std::sync::Arc::new(on_dir);
            for _ in 0..100 {
                let f = on_dir.clone();
                if files_win::instance::serve(move |d| f(d)).is_ok() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });
    } else if let Err(e) = files_win::instance::serve(on_dir) {
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
