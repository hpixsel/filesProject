//! Copy and cut-paste through the real clipboard and Windows' file engine.
//! Own test binary: it uses the system clipboard. Delete is checked by
//! hand: Windows asks for confirmation, which a test cannot answer.

use files_win::{fileop, Sta};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("files-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn paste_into(dst: PathBuf) {
    let (tx, rx) = mpsc::channel();
    fileop::paste_paths(0, dst, move |r| {
        tx.send(r.map_err(|e| e.to_string())).unwrap()
    });
    rx.recv_timeout(Duration::from_secs(30)).unwrap().unwrap();
}

#[test]
fn copy_then_cut_and_paste() {
    let (src, dst) = (tmp("fileop-src"), tmp("fileop-dst"));
    std::fs::write(src.join("a.txt"), "alpha").unwrap();
    std::fs::write(src.join("b.txt"), "bravo").unwrap();
    std::fs::write(src.join("c.txt"), "charlie").unwrap();
    let sta = Sta::new().unwrap();

    fileop::set_clipboard(&sta, &[src.join("a.txt")], false).unwrap();
    assert!(fileop::clipboard_has_paths());
    paste_into(dst.clone());
    assert_eq!(std::fs::read_to_string(dst.join("a.txt")).unwrap(), "alpha");
    assert!(src.join("a.txt").exists(), "copy keeps the source");

    fileop::set_clipboard(&sta, &[src.join("b.txt"), src.join("c.txt")], true).unwrap();
    paste_into(dst.clone());
    assert_eq!(std::fs::read_to_string(dst.join("b.txt")).unwrap(), "bravo");
    assert_eq!(
        std::fs::read_to_string(dst.join("c.txt")).unwrap(),
        "charlie"
    );
    assert!(
        !src.join("b.txt").exists() && !src.join("c.txt").exists(),
        "cut moves both"
    );

    // SAFETY: clears the clipboard this test filled.
    unsafe { windows::Win32::System::Ole::OleSetClipboard(None).unwrap() };
    drop(sta);
    let _ = std::fs::remove_dir_all(src);
    let _ = std::fs::remove_dir_all(dst);
}
