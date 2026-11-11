//! Spike: OLE drop target and the native shell context menu on a GPUI window.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use windows::core::{implement, Interface, Result, HSTRING, PCSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, POINTL, WPARAM};
use windows::Win32::System::Com::{CoTaskMemFree, IDataObject};
use windows::Win32::System::Ole::{
    IDropTarget, IDropTarget_Impl, RegisterDragDrop, RevokeDragDrop, DROPEFFECT, DROPEFFECT_COPY,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    DefSubclassProc, IContextMenu, IContextMenu3, IShellFolder, RemoveWindowSubclass,
    SHBindToParent, SHParseDisplayName, SetWindowSubclass, CMF_NORMAL, CMINVOKECOMMANDINFO,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetCursorPos, TrackPopupMenuEx, SW_SHOWNORMAL, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR,
};

/// Lines shown in the UI; written from COM callbacks on the UI thread.
pub type Log = Arc<Mutex<Vec<String>>>;

pub fn hwnd_of(window: &impl raw_window_handle::HasWindowHandle) -> Option<HWND> {
    match window.window_handle().ok()?.as_raw() {
        raw_window_handle::RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut _)),
        _ => None,
    }
}

#[implement(IDropTarget)]
struct DropTarget {
    log: Log,
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(&self, _: Option<&IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe { *effect = DROPEFFECT_COPY };
        Ok(())
    }
    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe { *effect = DROPEFFECT_COPY };
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        Ok(())
    }
    fn Drop(&self, obj: Option<&IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe { *effect = DROPEFFECT_COPY };
        if let Some(obj) = obj {
            let line = describe_drop(obj);
            self.log.lock().unwrap().push(line);
        }
        Ok(())
    }
}

fn describe_drop(obj: &IDataObject) -> String {
    let formats = files_win::formats(obj).unwrap_or_default().join(", ");
    let parsed = files_win::Sta::new()
        .and_then(|sta| files_win::VirtualFiles::from_data_object(&sta, obj.clone()));
    match parsed {
        Ok(vf) => {
            let names: Vec<String> = vf.entries().iter().take(3).map(|e| e.rel_path.to_string()).collect();
            format!("Drop: {} virtual entries ({}…) | formats: {formats}", vf.entries().len(), names.join(", "))
        }
        Err(e) => format!("Drop: {e} | formats: {formats}"),
    }
}

/// Replaces GPUI's drop target with ours.
pub fn register_drop(hwnd: HWND, log: Log) -> Result<()> {
    unsafe {
        let _ = RevokeDragDrop(hwnd);
        let target: IDropTarget = DropTarget { log }.into();
        RegisterDragDrop(hwnd, &target)
    }
}

/// Win32 menus are light unless the process opts into dark mode. Explorer
/// does this through two undocumented uxtheme exports (by ordinal):
/// 135 `SetPreferredAppMode` and 136 `FlushMenuThemes`.
// shortcut: undocumented API; if Windows drops it the menu just stays light
pub fn set_menu_dark(dark: bool) {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    unsafe {
        let Ok(ux) = LoadLibraryW(windows::core::w!("uxtheme.dll")) else { return };
        if let Some(f) = GetProcAddress(ux, PCSTR(135 as *const u8)) {
            let set: extern "system" fn(i32) -> i32 = std::mem::transmute(f);
            set(if dark { 2 } else { 3 }); // ForceDark / ForceLight
        }
        if let Some(f) = GetProcAddress(ux, PCSTR(136 as *const u8)) {
            let flush: extern "system" fn() = std::mem::transmute(f);
            flush();
        }
    }
}

thread_local! {
    static MENU: RefCell<Option<IContextMenu3>> = const { RefCell::new(None) };
}

extern "system" fn menu_subclass(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
    if matches!(msg, WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR) {
        let handled = MENU.with(|m| {
            m.borrow().as_ref().and_then(|cm| {
                let mut res = LRESULT(0);
                unsafe { cm.HandleMenuMsg2(msg, wp, lp, Some(&mut res)) }.ok().map(|_| res)
            })
        });
        if let Some(r) = handled {
            return r;
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}

/// Shows Explorer's own context menu for `path` at the cursor and runs the
/// chosen command. Returns the verb id that was picked, if any.
pub fn show_shell_menu(hwnd: HWND, path: &str) -> Result<Option<u32>> {
    unsafe {
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        SHParseDisplayName(&HSTRING::from(path), None, &mut pidl, 0, None)?;
        let mut child: *mut ITEMIDLIST = std::ptr::null_mut();
        let folder: IShellFolder = SHBindToParent(pidl, Some(&mut child))?;
        let cm: IContextMenu = folder.GetUIObjectOf(hwnd, &[child as *const _], None)?;
        let menu = CreatePopupMenu()?;
        cm.QueryContextMenu(menu, 0, 1, 0x7FFF, CMF_NORMAL)?;

        MENU.with(|m| *m.borrow_mut() = cm.cast::<IContextMenu3>().ok());
        let _ = SetWindowSubclass(hwnd, Some(menu_subclass), 1, 0);
        let mut pt = POINT::default();
        GetCursorPos(&mut pt)?;
        let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None).0;
        let _ = RemoveWindowSubclass(hwnd, Some(menu_subclass), 1);
        MENU.with(|m| *m.borrow_mut() = None);

        let picked = (cmd > 0).then(|| (cmd - 1) as u32);
        if let Some(id) = picked {
            let info = CMINVOKECOMMANDINFO {
                cbSize: std::mem::size_of::<CMINVOKECOMMANDINFO>() as u32,
                hwnd,
                lpVerb: PCSTR(id as usize as *const u8),
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            cm.InvokeCommand(&info)?;
        }
        let _ = DestroyMenu(menu);
        CoTaskMemFree(Some(pidl as *const _));
        Ok(picked)
    }
}
