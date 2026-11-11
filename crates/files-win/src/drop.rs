//! Files dropped on the window (from Explorer, NanaZip, 7-Zip…) are copied
//! into the current folder by Windows' file engine; Shift moves instead.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use windows::core::{implement, IUnknown, Interface, Result};
use windows::Win32::Foundation::{HWND, POINTL};
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::{
    IDropTarget, IDropTarget_Impl, RegisterDragDrop, RevokeDragDrop, DROPEFFECT, DROPEFFECT_COPY,
    DROPEFFECT_MOVE,
};
use windows::Win32::System::SystemServices::{MK_SHIFT, MODIFIERKEYS_FLAGS};

type OnDrop = Box<dyn Fn(crate::Result<()>)>;

#[implement(IDropTarget)]
struct DropTarget {
    window: isize,
    dir: Arc<Mutex<PathBuf>>,
    on_drop: OnDrop,
}

fn effect(keys: MODIFIERKEYS_FLAGS) -> DROPEFFECT {
    if keys.0 & MK_SHIFT.0 != 0 {
        DROPEFFECT_MOVE
    } else {
        DROPEFFECT_COPY
    }
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(
        &self,
        _: Option<&IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        _: &POINTL,
        eff: *mut DROPEFFECT,
    ) -> Result<()> {
        // SAFETY: OLE passes a valid out pointer.
        unsafe { *eff = effect(keys) };
        Ok(())
    }

    fn DragOver(&self, keys: MODIFIERKEYS_FLAGS, _: &POINTL, eff: *mut DROPEFFECT) -> Result<()> {
        // SAFETY: as above.
        unsafe { *eff = effect(keys) };
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        Ok(())
    }

    /// Runs the copy before returning: some sources (7-Zip, NanaZip) extract
    /// to a temporary folder and delete it once the drop returns.
    fn Drop(
        &self,
        data: Option<&IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        _: &POINTL,
        eff: *mut DROPEFFECT,
    ) -> Result<()> {
        let mv = effect(keys) == DROPEFFECT_MOVE;
        // SAFETY: as above.
        unsafe { *eff = if mv { DROPEFFECT_MOVE } else { DROPEFFECT_COPY } };
        let Some(data) = data else { return Ok(()) };
        let dir = self.dir.lock().unwrap().clone();
        let r = data
            .cast::<IUnknown>()
            .map_err(crate::Error::Win)
            .and_then(|items| crate::fileop::transfer_now(self.window, &items, &dir, mv));
        (self.on_drop)(r);
        Ok(())
    }
}

/// Replaces the UI toolkit's drop handling on `window` with ours. `dir` is
/// read at drop time, so the caller keeps it pointing at the current folder.
/// Must be called on the window's (OLE-initialised) UI thread.
pub fn register(
    window: isize,
    dir: Arc<Mutex<PathBuf>>,
    on_drop: impl Fn(crate::Result<()>) + 'static,
) -> crate::Result<()> {
    let hwnd = HWND(window as *mut _);
    let target: IDropTarget = DropTarget {
        window,
        dir,
        on_drop: Box::new(on_drop),
    }
    .into();
    // SAFETY: valid window handle owned by this thread.
    unsafe {
        let _ = RevokeDragDrop(hwnd);
        RegisterDragDrop(hwnd, &target)?;
    }
    Ok(())
}
