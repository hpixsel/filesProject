//! Round trip through the real system clipboard. Its own test binary, so it
//! never runs next to other tests that touch the clipboard.

use files_win::fake::{FakeContent, FakeFile, FakeFiles};
use files_win::{Sta, VirtualFiles};
use windows::Win32::System::Ole::{OleFlushClipboard, OleSetClipboard};

#[test]
fn fake_survives_the_system_clipboard() {
    let sta = Sta::new().unwrap();
    let obj = FakeFiles::new()
        .file("x.txt", FakeFile::new(FakeContent::Bytes(b"hi".to_vec())))
        .into_data_object();
    // SAFETY: OLE is initialised on this thread by `sta`.
    // Flush at once: otherwise clipboard history asks *this* thread for the
    // data, and a test thread that does not pump messages deadlocks it.
    unsafe {
        OleSetClipboard(&obj).unwrap();
        OleFlushClipboard().unwrap();
    }
    let names: Vec<String> = VirtualFiles::from_clipboard(&sta)
        .unwrap()
        .entries()
        .iter()
        .map(|e| e.rel_path.to_string())
        .collect();
    // SAFETY: as above; clears the clipboard again.
    unsafe {
        OleSetClipboard(None).unwrap();
        let _ = OleFlushClipboard();
    }
    assert_eq!(names, ["x.txt"]);
}
