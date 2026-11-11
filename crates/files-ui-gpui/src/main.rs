//! FilesProject window: tabs and panes of folders, Explorer context menu and
//! Ctrl+V paste from an RDP session through a child `rdppaste --machine`.
// No console window, in debug builds too (they are what gets tested).
#![windows_subsystem = "windows"]

mod assets;
mod explorer;
mod i18n;
mod palette;
mod prefs;
mod settings_page;
mod theme;
mod workspace;

use gpui_kit::component::*;
use gpui_kit::*;
use std::path::PathBuf;

/// Command-line actions: they print to the terminal and exit. False when
/// `arg` is not one of them.
fn command(arg: &std::ffi::OsStr) -> bool {
    use files_app::{replace_explorer, update};
    let done = match arg.to_str() {
        Some("--install") => update::install().and_then(|dir| {
            files_win::shell::add_to_user_path(&dir).map_err(|e| e.to_string())?;
            Ok(format!(
                "Installed to {}. Open a new terminal and run `files`.",
                dir.display()
            ))
        }),
        Some("--replace-explorer") => replace_explorer::replace().map(|()| {
            "Folders, drives and Win+E now open FilesProject. \
             Undo: files --restore-explorer"
                .into()
        }),
        Some("--restore-explorer") => replace_explorer::restore()
            .map(|()| "Windows Explorer opens folders, drives and Win+E again.".into()),
        _ => return false,
    };
    files_win::shell::attach_console();
    match done {
        Ok(text) => println!("\n{text}"),
        Err(e) => eprintln!("\nFailed: {e}"),
    }
    true
}

/// The window has no console, so a crash goes to
/// `%LOCALAPPDATA%\FilesProject\logs\files.log` with a backtrace.
fn log_panics() {
    let Some(dir) =
        std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join(r"FilesProject\logs"))
    else {
        return;
    };
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("files.log"))
        {
            let trace = std::backtrace::Backtrace::force_capture();
            let _ = writeln!(
                f,
                "{:?} v{}: {info}\n{trace}",
                std::time::SystemTime::now(),
                env!("CARGO_PKG_VERSION")
            );
        }
        default(info);
    }));
}

fn main() {
    log_panics();
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| command(a)) {
        return;
    }
    // Started by the update of a window that is closing right now.
    let after_update = args.first().is_some_and(|a| a == "--after-update");
    if after_update {
        args.remove(0);
    }
    // After an update the saved tabs already hold its folder.
    let explicit = !args.is_empty() && !after_update;
    let start = args
        .into_iter()
        .next()
        // Explorer passes a drive as `"C:\"`, which arrives as `C:"` (the
        // backslash escapes the quote); `"` is never part of a Windows path.
        .map(|a| match a.to_str().and_then(|s| s.strip_suffix('"')) {
            Some(s) => PathBuf::from(format!("{s}\\")),
            None => PathBuf::from(a),
        })
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(r"C:\"));
    // `files .` from a terminal: relative to where it was typed.
    let start = if files_app::places::is_place(&start) {
        start // `shell:MyComputerFolder` is not a path on a disk
    } else {
        std::path::absolute(&start).unwrap_or(start)
    };

    // Already running: the folder opens as a tab there.
    if !after_update && files_win::instance::send(&start) {
        return;
    }
    // Folders from later `files <folder>` calls; checked on the pipe thread,
    // since a network path can take seconds to answer.
    let (folders, incoming) = std::sync::mpsc::channel();
    let on_dir = move |dir: PathBuf| {
        if dir.is_dir() || files_app::places::is_place(&dir) {
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
        .with_assets(assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            explorer::bind_keys(cx);
            workspace::bind_keys(cx);
            let settings = files_app::settings::file()
                .map(|f| files_app::settings::load(&f))
                .unwrap_or_default();
            prefs::init(settings, cx);
            // Where and how big the window was when it was last closed.
            let window_bounds = files_app::session::file()
                .and_then(|f| files_app::session::load(&f))
                .and_then(|s| s.window)
                .map(|(x, y, w, h, max)| {
                    let b = Bounds::new(point(px(x), px(y)), size(px(w), px(h)));
                    if max {
                        WindowBounds::Maximized(b)
                    } else {
                        WindowBounds::Windowed(b)
                    }
                });
            let options = WindowOptions {
                window_bounds,
                titlebar: Some(TitleBar::title_bar_options()),
                // Mica gets DWM ready for a system backdrop; Workspace::new swaps it
                // for acrylic (files_win::shell::set_acrylic).
                window_background: WindowBackgroundAppearance::MicaBackdrop,
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, move |window, cx| {
                cx.new(|cx| workspace::Workspace::new(start, explicit, incoming, window, cx))
            })
            .expect("open window");
        });
}
