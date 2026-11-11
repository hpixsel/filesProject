//! Shell integration for the UI: Explorer's own context menu, opening files,
//! dark menus and local time formatting. Windows are passed as raw `HWND`
//! values (`isize`) so UI crates need no Windows types.

use std::cell::RefCell;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use windows::core::{Interface, HSTRING, PCSTR, PCWSTR};
use windows::Win32::Foundation::{FILETIME, HWND, LPARAM, LRESULT, POINT, SYSTEMTIME, WPARAM};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    DefSubclassProc, IContextMenu, IContextMenu3, IShellFolder, RemoveWindowSubclass,
    SHBindToParent, SHParseDisplayName, SetWindowSubclass, ShellExecuteW, CMF_NORMAL,
    CMINVOKECOMMANDINFO,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetCursorPos, TrackPopupMenuEx, SW_SHOWNORMAL, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR,
};

fn hwnd(raw: isize) -> HWND {
    HWND(raw as *mut _)
}

/// Opens `path` with its default program, like a double click in Explorer.
pub fn open(window: isize, path: &Path) -> crate::Result<()> {
    let file = HSTRING::from(path.as_os_str());
    // SAFETY: both strings are valid for the call.
    let r = unsafe {
        ShellExecuteW(
            hwnd(window),
            windows::core::w!("open"),
            &file,
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute returns a value > 32 on success.
    if r.0 as isize > 32 {
        Ok(())
    } else {
        Err(crate::Error::Win(windows::core::Error::from_win32()))
    }
}

/// Win32 menus stay light unless the process opts into dark mode. Explorer
/// uses two undocumented uxtheme exports: ordinal 135 `SetPreferredAppMode`
/// and 136 `FlushMenuThemes`.
// shortcut: undocumented API; if Windows drops it the menus just stay light
pub fn set_menu_dark(dark: bool) {
    // SAFETY: the ordinals have these signatures on Windows 10 1903+; missing
    // exports are skipped.
    unsafe {
        let Ok(ux) = LoadLibraryW(windows::core::w!("uxtheme.dll")) else {
            return;
        };
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

extern "system" fn menu_subclass(
    h: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    if matches!(
        msg,
        WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR
    ) {
        let handled = MENU.with(|m| {
            m.borrow().as_ref().and_then(|cm| {
                let mut res = LRESULT(0);
                // SAFETY: forwards a message the menu owner received.
                unsafe { cm.HandleMenuMsg2(msg, wp, lp, Some(&mut res)) }
                    .ok()
                    .map(|_| res)
            })
        });
        if let Some(r) = handled {
            return r;
        }
    }
    // SAFETY: default handling for everything else.
    unsafe { DefSubclassProc(h, msg, wp, lp) }
}

/// Shows Explorer's context menu for `path` at the cursor and runs the chosen
/// command. Runs a modal message loop: call it outside UI-framework event
/// dispatch (e.g. from a spawned task). Returns whether a command ran.
pub fn context_menu(window: isize, path: &Path) -> crate::Result<bool> {
    let h = hwnd(window);
    // SAFETY: standard shell calls; the PIDL is freed below and the menu destroyed.
    unsafe {
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        SHParseDisplayName(&HSTRING::from(path.as_os_str()), None, &mut pidl, 0, None)?;
        let result = (|| -> crate::Result<bool> {
            let mut child: *mut ITEMIDLIST = std::ptr::null_mut();
            let folder: IShellFolder = SHBindToParent(pidl, Some(&mut child))?;
            let cm: IContextMenu = folder.GetUIObjectOf(h, &[child as *const _], None)?;
            let menu = CreatePopupMenu()?;
            cm.QueryContextMenu(menu, 0, 1, 0x7FFF, CMF_NORMAL)?;

            MENU.with(|m| *m.borrow_mut() = cm.cast::<IContextMenu3>().ok());
            let _ = SetWindowSubclass(h, Some(menu_subclass), 1, 0);
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let cmd = TrackPopupMenuEx(
                menu,
                (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
                pt.x,
                pt.y,
                h,
                None,
            )
            .0;
            let _ = RemoveWindowSubclass(h, Some(menu_subclass), 1);
            MENU.with(|m| *m.borrow_mut() = None);

            let ran = cmd > 0;
            if ran {
                let info = CMINVOKECOMMANDINFO {
                    cbSize: std::mem::size_of::<CMINVOKECOMMANDINFO>() as u32,
                    hwnd: h,
                    lpVerb: PCSTR((cmd - 1) as usize as *const u8),
                    nShow: SW_SHOWNORMAL.0,
                    ..Default::default()
                };
                cm.InvokeCommand(&info)?;
            }
            let _ = DestroyMenu(menu);
            Ok(ran)
        })();
        CoTaskMemFree(Some(pidl as *const _));
        result
    }
}

/// `YYYY-MM-DD HH:MM` in the local time zone.
pub fn local_time(t: SystemTime) -> String {
    let Ok(d) = t.duration_since(UNIX_EPOCH) else {
        return String::new();
    };
    let ticks = d.as_nanos() / 100 + 116_444_736_000_000_000;
    let ft = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    // SAFETY: plain conversions on stack values.
    let ok = unsafe {
        FileTimeToSystemTime(&ft, &mut utc).is_ok()
            && SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_ok()
    };
    if !ok {
        return String::new();
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute
    )
}
