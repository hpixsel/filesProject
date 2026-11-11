//! Shell integration for the UI: Explorer's own context menu, opening files,
//! dark menus and local time formatting. Windows are passed as raw `HWND`
//! values (`isize`) so UI crates need no Windows types.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
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
pub fn context_menu(window: isize, paths: &[PathBuf]) -> crate::Result<bool> {
    let h = hwnd(window);
    let pidls = Pidls::parse(paths)?;
    // SAFETY: standard shell calls; the PIDLs live until `pidls` drops and
    // the menu is destroyed.
    unsafe {
        let result = (|| -> crate::Result<bool> {
            // The items share a folder; each child ID points into its own PIDL.
            let mut folder: Option<IShellFolder> = None;
            let mut children = Vec::with_capacity(pidls.0.len());
            for &p in &pidls.0 {
                let mut child: *mut ITEMIDLIST = std::ptr::null_mut();
                let f: IShellFolder = SHBindToParent(p, Some(&mut child))?;
                folder.get_or_insert(f);
                children.push(child as *const ITEMIDLIST);
            }
            let folder = folder.ok_or(crate::Error::BadDescriptor("no items"))?;
            let cm: IContextMenu = folder.GetUIObjectOf(h, &children, None)?;
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
        result
    }
}

/// Absolute PIDLs for paths, freed on drop.
pub(crate) struct Pidls(pub(crate) Vec<*mut ITEMIDLIST>);

impl Pidls {
    pub(crate) fn parse(paths: &[PathBuf]) -> crate::Result<Pidls> {
        let mut out = Pidls(Vec::with_capacity(paths.len()));
        for p in paths {
            let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            // SAFETY: valid string; on success the PIDL is owned by `out`.
            unsafe { SHParseDisplayName(&HSTRING::from(p.as_os_str()), None, &mut pidl, 0, None)? };
            out.0.push(pidl);
        }
        Ok(out)
    }

    pub(crate) fn as_slice(&self) -> Vec<*const ITEMIDLIST> {
        self.0.iter().map(|&p| p as *const _).collect()
    }
}

impl Drop for Pidls {
    fn drop(&mut self) {
        for &p in &self.0 {
            // SAFETY: allocated by SHParseDisplayName, freed once.
            unsafe { CoTaskMemFree(Some(p as *const _)) };
        }
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

/// Drive roots (`C:\`, `D:\`…). Uses the drive bitmask, so offline network
/// drives cannot make it hang.
pub fn drives() -> Vec<PathBuf> {
    // SAFETY: no arguments, returns a bitmask.
    let mask = unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| PathBuf::from(format!("{}:\\", (b'A' + i) as char)))
        .collect()
}

/// The Windows accent colour as `0xRRGGBB`. With "Accent colour:
/// Automatic", Windows picks it from the wallpaper.
pub fn accent_colour() -> Option<u32> {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let (mut abgr, mut len) = (0u32, 4u32);
    // SAFETY: a 4-byte buffer for a REG_DWORD, its size in `len`.
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            windows::core::w!(r"Software\Microsoft\Windows\DWM"),
            windows::core::w!("AccentColor"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut abgr as *mut u32 as *mut _),
            Some(&mut len),
        )
    }
    .ok()
    .ok()?;
    Some(abgr.swap_bytes() >> 8)
}

/// True when Windows shows its own UI in Polish.
pub fn ui_is_polish() -> bool {
    // SAFETY: no arguments. The low 10 bits are the primary language.
    unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() & 0x3ff == 0x15 }
}
