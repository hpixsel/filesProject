//! Dragging files out of the window to Explorer, the desktop or any other
//! program, with Windows' own drag and drop (the same data Explorer gives).

use std::path::PathBuf;
use windows::core::implement;
use windows::Win32::Foundation::{
    BOOL, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK,
};
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::{
    DoDragDrop, IDropSource, IDropSource_Impl, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK,
    DROPEFFECT_MOVE,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::BHID_DataObject;

/// Ends the drag when the left button is released, cancels it on Esc.
#[implement(IDropSource)]
struct Source;

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> windows::core::HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if keys.0 & MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _: DROPEFFECT) -> windows::core::HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Drags `paths` (all in one folder) with Windows' drag and drop until the
/// button is released; true when the target moved them away. Runs a modal
/// loop: call it on the window's thread, outside GPUI's event dispatch,
/// while the mouse button is still down.
pub fn drag_out(paths: &[PathBuf]) -> crate::Result<bool> {
    // SAFETY: shell data object for the items; OLE is initialised on the
    // window's thread (its drop target is registered there).
    unsafe {
        let data: IDataObject =
            crate::fileop::item_array(paths)?.BindToHandler(None, &BHID_DataObject)?;
        let source: IDropSource = Source.into();
        let mut effect = DROPEFFECT(0);
        let r = DoDragDrop(
            &data,
            &source,
            DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK,
            &mut effect,
        );
        Ok(r == DRAGDROP_S_DROP && effect.0 & DROPEFFECT_MOVE.0 != 0)
    }
}
