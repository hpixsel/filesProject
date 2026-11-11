//! Local copy / move / delete through Windows' own file engine
//! (`IFileOperation`): progress dialog, conflict prompts, undo, Recycle Bin.
//! Each operation runs on its own OLE thread, so the UI never waits on it.

use crate::{Result, Sta};
use std::path::{Path, PathBuf};
use windows::core::{Interface, HSTRING};
use windows::Win32::Foundation::{HWND, S_OK};
use windows::Win32::System::Com::{
    CoCreateInstance, IDataObject, CLSCTX_ALL, FORMATETC, STGMEDIUM,
};
use windows::Win32::System::DataExchange::IsClipboardFormatAvailable;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{OleFlushClipboard, OleGetClipboard, OleSetClipboard, CF_HDROP};
use windows::Win32::UI::Shell::{
    BHID_DataObject, FileOperation, IFileOperation, IShellItem, IShellItemArray,
    SHCreateItemFromParsingName, SHCreateShellItemArrayFromIDLists, CFSTR_PREFERREDDROPEFFECT,
    FILEOPERATION_FLAGS, FOF_ALLOWUNDO, FOF_NOCONFIRMMKDIR,
};

const DROPEFFECT_MOVE: u32 = 2;

fn item(path: &Path) -> Result<IShellItem> {
    // SAFETY: valid string for the call.
    Ok(unsafe { SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None)? })
}

/// True when the clipboard holds real paths (`CF_HDROP`): an Explorer copy,
/// NanaZip/7-Zip, or our own Ctrl+C. RDP and zip contents are virtual files
/// and go through `rdppaste` instead.
pub fn clipboard_has_paths() -> bool {
    // SAFETY: a plain query; it does not open the clipboard.
    unsafe { IsClipboardFormatAvailable(u32::from(CF_HDROP.0)).is_ok() }
}

/// The items as one shell item array (they share a folder).
fn item_array(paths: &[PathBuf]) -> Result<IShellItemArray> {
    let pidls = crate::shell::Pidls::parse(paths)?;
    // SAFETY: the PIDLs stay alive until `pidls` drops, after the call.
    Ok(unsafe { SHCreateShellItemArrayFromIDLists(&pidls.as_slice())? })
}

/// Puts `paths` on the clipboard the way Explorer does, so they can be
/// pasted here or in Explorer. `cut` marks them to be moved on paste.
pub fn set_clipboard(_sta: &Sta, paths: &[PathBuf], cut: bool) -> Result<()> {
    // SAFETY: shell data object for the items; HGLOBAL ownership passes to SetData.
    unsafe {
        let data: IDataObject = item_array(paths)?.BindToHandler(None, &BHID_DataObject)?;
        if cut {
            let h = GlobalAlloc(GMEM_MOVEABLE, 4)?;
            *(GlobalLock(h) as *mut u32) = DROPEFFECT_MOVE;
            let _ = GlobalUnlock(h);
            let fmt = FORMATETC {
                cfFormat: crate::clipboard::cf(CFSTR_PREFERREDDROPEFFECT),
                ..crate::clipboard::formatetc(
                    0,
                    -1,
                    windows::Win32::System::Com::TYMED_HGLOBAL.0 as u32,
                )
            };
            let medium = STGMEDIUM {
                tymed: fmt.tymed,
                u: windows::Win32::System::Com::STGMEDIUM_0 { hGlobal: h },
                pUnkForRelease: std::mem::ManuallyDrop::new(None),
            };
            data.SetData(&fmt, &medium, true)?;
        }
        crate::clipboard::retry_open(|| OleSetClipboard(&data))?;
        // Render now, so the clipboard survives this program closing.
        let _ = OleFlushClipboard();
    }
    Ok(())
}

fn wants_move(data: &IDataObject) -> bool {
    let fmt = crate::clipboard::formatetc(
        crate::clipboard::cf(CFSTR_PREFERREDDROPEFFECT),
        -1,
        windows::Win32::System::Com::TYMED_HGLOBAL.0 as u32,
    );
    // SAFETY: reads a 4-byte DWORD from a locked HGLOBAL; the medium is released.
    unsafe {
        if data.QueryGetData(&fmt) != S_OK {
            return false;
        }
        let Ok(mut m) = data.GetData(&fmt) else {
            return false;
        };
        let p = GlobalLock(m.u.hGlobal) as *const u32;
        let effect = if p.is_null() { 0 } else { *p };
        let _ = GlobalUnlock(m.u.hGlobal);
        windows::Win32::System::Ole::ReleaseStgMedium(&mut m);
        effect & DROPEFFECT_MOVE != 0
    }
}

fn operation(window: isize, flags: FILEOPERATION_FLAGS) -> Result<IFileOperation> {
    // SAFETY: standard COM creation on an OLE thread.
    unsafe {
        let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
        op.SetOwnerWindow(HWND(window as *mut _))?;
        op.SetOperationFlags(flags)?;
        Ok(op)
    }
}

/// Copies (or moves) `items` (a data object or shell items) into `dest` on
/// the calling OLE thread; returns when Windows has finished.
pub fn transfer_now(
    window: isize,
    items: &windows::core::IUnknown,
    dest: &Path,
    mv: bool,
) -> Result<()> {
    let dest = item(dest)?;
    let op = operation(window, FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR)?;
    // SAFETY: COM calls on this thread's apartment.
    unsafe {
        if mv {
            op.MoveItems(items, &dest)?;
        } else {
            op.CopyItems(items, &dest)?;
        }
        op.PerformOperations()?;
    }
    Ok(())
}

/// Runs `f` on a new OLE thread and `done` when it finishes.
fn on_ole_thread(
    f: impl FnOnce(&Sta) -> Result<()> + Send + 'static,
    done: impl FnOnce(Result<()>) + Send + 'static,
) {
    std::thread::spawn(move || {
        let r = Sta::new().and_then(|sta| f(&sta));
        done(r);
    });
}

/// Pastes the paths on the clipboard into `target`: moves them if they were
/// cut, copies otherwise. Windows shows progress and asks about conflicts.
pub fn paste_paths(window: isize, target: PathBuf, done: impl FnOnce(Result<()>) + Send + 'static) {
    on_ole_thread(
        move |_sta| {
            // Windows' engine opens the clipboard itself while reading the
            // items; if another process holds it, nothing was copied yet, so
            // the whole operation is retried.
            crate::clipboard::retry_open(|| {
                // SAFETY: called on this thread's apartment.
                let data = unsafe { OleGetClipboard()? };
                match transfer_now(window, &data.cast()?, &target, wants_move(&data)) {
                    Err(crate::Error::Win(e)) => Err(e),
                    Err(other) => Ok(Err(other)),
                    Ok(()) => Ok(Ok(())),
                }
            })?
        },
        done,
    );
}

/// Deletes `paths`: to the Recycle Bin, or for good when `permanent`.
pub fn delete(
    window: isize,
    paths: Vec<PathBuf>,
    permanent: bool,
    done: impl FnOnce(Result<()>) + Send + 'static,
) {
    on_ole_thread(
        move |_sta| {
            let flags = if permanent {
                FILEOPERATION_FLAGS(0)
            } else {
                FOF_ALLOWUNDO
            };
            // SAFETY: COM calls on this thread's apartment.
            unsafe {
                let op = operation(window, flags)?;
                op.DeleteItems(&item_array(&paths)?)?;
                op.PerformOperations()?;
            }
            Ok(())
        },
        done,
    );
}
