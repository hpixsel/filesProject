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
    SHBindToObject, SHBindToParent, SHParseDisplayName, SetWindowSubclass, ShellExecuteW,
    CMF_CANRENAME, CMF_NORMAL, CMINVOKECOMMANDINFO, GCS_VERBA,
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

/// What the user chose in a shell menu.
#[derive(Debug, PartialEq, Eq)]
pub enum Picked {
    Nothing,
    /// A command ran; the folder may have changed.
    Ran,
    /// "Rename": Explorer edits the name in place, so the caller does.
    Rename,
}

/// Explorer's own context menu for `paths` (all in one folder), at the
/// cursor. Modal: call it outside GPUI's event dispatch.
pub fn context_menu(window: isize, paths: &[PathBuf]) -> crate::Result<Picked> {
    let h = hwnd(window);
    let pidls = Pidls::parse(paths)?;
    // SAFETY: standard shell calls; the PIDLs live until `pidls` drops.
    unsafe {
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
        track(h, &cm, CMF_NORMAL | CMF_CANRENAME)
    }
}

/// Explorer's menu for the empty space of folder `dir`: New (folder, file
/// types), Paste, Properties and so on. Modal, like [`context_menu`].
pub fn folder_menu(window: isize, dir: &Path) -> crate::Result<Picked> {
    let h = hwnd(window);
    let pidls = Pidls::parse(&[dir.to_path_buf()])?;
    // SAFETY: as above; the folder's PIDL lives until `pidls` drops.
    unsafe {
        let folder: IShellFolder = SHBindToObject(None, pidls.0[0], None)?;
        let cm: IContextMenu = folder.CreateViewObject(h)?;
        track(h, &cm, CMF_NORMAL)
    }
}

/// Shows `cm` at the cursor and runs the chosen command, except "rename",
/// which is handed back.
unsafe fn track(h: HWND, cm: &IContextMenu, flags: u32) -> crate::Result<Picked> {
    let menu = CreatePopupMenu()?;
    let result = (|| -> crate::Result<Picked> {
        cm.QueryContextMenu(menu, 0, 1, 0x7FFF, flags)?;
        // Owner-drawn and "New" submenus need their messages forwarded.
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
        if cmd <= 0 {
            return Ok(Picked::Nothing);
        }
        let id = (cmd - 1) as usize;
        let mut verb = [0u8; 64];
        let named = cm.GetCommandString(
            id,
            GCS_VERBA,
            None,
            windows::core::PSTR(verb.as_mut_ptr()),
            verb.len() as u32,
        );
        if named.is_ok() && verb.starts_with(b"rename\0") {
            return Ok(Picked::Rename);
        }
        let info = CMINVOKECOMMANDINFO {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFO>() as u32,
            hwnd: h,
            lpVerb: PCSTR(id as *const u8),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        cm.InvokeCommand(&info)?;
        Ok(Picked::Ran)
    })();
    let _ = DestroyMenu(menu);
    result
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

/// Windows 11's own acrylic backdrop (DWMSBT_TRANSIENTWINDOW). It moves with
/// the window smoothly, unlike the older `SetWindowCompositionAttribute`
/// acrylic behind GPUI's `Blurred`, which lags while dragging. Older Windows
/// ignores it and keeps what was set before.
pub fn set_acrylic(window: isize) {
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_SYSTEMBACKDROP_TYPE};
    let acrylic = 3u32;
    // SAFETY: a valid window and a u32 attribute of the right size.
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd(window),
            DWMWA_SYSTEMBACKDROP_TYPE,
            &acrylic as *const u32 as *const _,
            4,
        )
    };
}

/// The share a mapped network drive points to, like `\\server\share`.
/// Answered from Windows' own record, so a server that is down does not
/// block it.
pub fn network_path(drive: &Path) -> Option<String> {
    use windows::Win32::NetworkManagement::WNet::WNetGetConnectionW;
    let letter = HSTRING::from(drive.to_str()?.trim_end_matches('\\'));
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `buf` holds `len` UTF-16 units.
    unsafe { WNetGetConnectionW(&letter, windows::core::PWSTR(buf.as_mut_ptr()), &mut len) }
        .ok()
        .ok()?;
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..end]))
}

/// Windows' own "Map network drive" dialog (letter, folder, sign-in,
/// reconnect at sign-in). Modal: call it outside GPUI's event dispatch.
/// True when a drive was mapped.
pub fn map_drive_dialog(window: isize) -> bool {
    use windows::Win32::NetworkManagement::WNet::{WNetConnectionDialog, RESOURCETYPE_DISK};
    // SAFETY: a valid owner window.
    unsafe { WNetConnectionDialog(hwnd(window), RESOURCETYPE_DISK.0) }.is_ok()
}

/// Windows' own "Disconnect network drives" dialog. Modal, like
/// [`map_drive_dialog`]. True when a drive was disconnected.
pub fn disconnect_drive_dialog(window: isize) -> bool {
    use windows::Win32::NetworkManagement::WNet::{WNetDisconnectDialog, RESOURCETYPE_DISK};
    // SAFETY: a valid owner window.
    unsafe { WNetDisconnectDialog(hwnd(window), RESOURCETYPE_DISK.0) }.is_ok()
}

/// Lets a GUI program print to the terminal that started it (`--install`).
pub fn attach_console() {
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    // SAFETY: no pointers; fails harmlessly without a parent console.
    let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

/// Adds `dir` to the user's PATH (HKCU\Environment) unless it is there, and
/// tells running programs. New terminals see it; open ones do not.
pub fn add_to_user_path(dir: &Path) -> crate::Result<()> {
    use windows::Win32::System::Registry::{
        RegCloseKey, RegGetValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_READ, KEY_WRITE, REG_EXPAND_SZ, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };
    let (sub, name) = (windows::core::w!("Environment"), windows::core::w!("Path"));
    let mut key = HKEY::default();
    // SAFETY: valid strings and an out-pointer; the key is closed below.
    unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, sub, 0, KEY_READ | KEY_WRITE, &mut key) }.ok()?;
    let result = (|| {
        let mut bytes = 0u32;
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
        // SAFETY: size query first, then a buffer of that size.
        let mut buf: Vec<u16> = unsafe {
            match RegGetValueW(key, None, name, flags, None, None, Some(&mut bytes)).ok() {
                Ok(()) => {
                    let mut buf = vec![0u16; bytes as usize / 2 + 1];
                    RegGetValueW(
                        key,
                        None,
                        name,
                        flags,
                        None,
                        Some(buf.as_mut_ptr() as *mut _),
                        Some(&mut bytes),
                    )
                    .ok()?;
                    buf.truncate(bytes as usize / 2);
                    buf
                }
                Err(_) => Vec::new(), // no user PATH yet
            }
        };
        while buf.last() == Some(&0) {
            buf.pop();
        }
        let path = String::from_utf16_lossy(&buf);
        let want = dir.to_string_lossy();
        let has = path.split(';').any(|p| {
            p.trim_end_matches('\\')
                .eq_ignore_ascii_case(want.trim_end_matches('\\'))
        });
        if has {
            return Ok(());
        }
        let mut new = path.trim_end_matches(';').to_string();
        if !new.is_empty() {
            new.push(';');
        }
        new.push_str(&want);
        let wide: Vec<u16> = new.encode_utf16().chain([0]).collect();
        // SAFETY: the bytes of a NUL-terminated UTF-16 string.
        unsafe {
            let data = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
            RegSetValueExW(key, name, 0, REG_EXPAND_SZ, Some(data)).ok()?;
            let env = windows::core::w!("Environment");
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                WPARAM(0),
                LPARAM(env.as_ptr() as isize),
                SMTO_ABORTIFHUNG,
                2000,
                None,
            );
        }
        Ok(())
    })();
    // SAFETY: opened above.
    let _ = unsafe { RegCloseKey(key) };
    result
}

/// A drive's volume label ("Data"), or `None` when it has none or does not
/// answer. Can be slow for a network drive: call it off the UI thread.
pub fn volume_label(root: &Path) -> Option<String> {
    use windows::Win32::Storage::FileSystem::GetVolumeInformationW;
    let mut name = [0u16; 261];
    // SAFETY: a NUL-terminated root and a buffer the API fills.
    unsafe {
        GetVolumeInformationW(
            &HSTRING::from(root.as_os_str()),
            Some(&mut name),
            None,
            None,
            None,
            None,
        )
    }
    .ok()?;
    let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    (end > 0).then(|| String::from_utf16_lossy(&name[..end]))
}

/// Windows' "Empty Recycle Bin", with its own confirmation and progress.
/// Modal: call it outside GPUI's event dispatch.
pub fn empty_recycle_bin(window: isize) -> bool {
    use windows::Win32::UI::Shell::SHEmptyRecycleBinW;
    // SAFETY: a valid owner window; no root means every drive.
    unsafe { SHEmptyRecycleBinW(hwnd(window), PCWSTR::null(), 0) }.is_ok()
}

/// Windows' Properties window for a file or folder (it stays open on its
/// own). Call it outside GPUI's event dispatch.
pub fn properties(window: isize, path: &Path) {
    use windows::Win32::UI::Shell::{SHObjectProperties, SHOP_FILEPATH};
    // SAFETY: a valid owner window and a NUL-terminated path.
    let _ = unsafe {
        SHObjectProperties(
            hwnd(window),
            SHOP_FILEPATH,
            &HSTRING::from(path.as_os_str()),
            PCWSTR::null(),
        )
    };
}

/// Where a `.lnk` shortcut points, when that is a path on a disk (not a
/// shell place or a URL). Reads the file through Windows' own shell link.
pub fn link_target(link: &Path) -> Option<PathBuf> {
    use windows::Win32::System::Com::{
        CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    let _sta = crate::Sta::new().ok()?;
    let mut buf = vec![0u16; 32_768];
    // SAFETY: COM on this thread's apartment; `buf` is the output buffer.
    unsafe {
        let sl: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        sl.cast::<IPersistFile>()
            .ok()?
            .Load(&HSTRING::from(link.as_os_str()), STGM_READ)
            .ok()?;
        sl.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(0);
    (end > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..end])))
}

/// Pixels of the picture Windows shows for `path`, as (width, height,
/// top-down BGRA): its icon, or with `thumbnail` its preview (a photo, a
/// PDF page, a video frame), `None` when it has none. Can be slow: call it
/// off the UI thread.
pub fn shell_image(path: &Path, size: i32, thumbnail: bool) -> Option<(u32, u32, Vec<u8>)> {
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::{
        DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, DIB_RGB_COLORS,
    };
    use windows::Win32::UI::Shell::{
        IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK,
        SIIGBF_ICONONLY, SIIGBF_THUMBNAILONLY,
    };
    let _sta = crate::Sta::new().ok()?;
    // SAFETY: COM on this thread's apartment; the bitmap is read into a
    // buffer of its own size and then deleted.
    unsafe {
        let item: IShellItem =
            SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).ok()?;
        let factory: IShellItemImageFactory = item.cast().ok()?;
        let kind = if thumbnail {
            SIIGBF_THUMBNAILONLY // at most `size`: a photo would come whole
        } else {
            SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK
        };
        let bitmap = factory.GetImage(SIZE { cx: size, cy: size }, kind).ok()?;
        let mut bm = BITMAP::default();
        let got = GetObjectW(
            bitmap,
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        );
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        let mut pixels = vec![0u8; (w.max(0) * h.max(0) * 4) as usize];
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down rows
                biPlanes: 1,
                biBitCount: 32,
                ..Default::default()
            },
            ..Default::default()
        };
        let dc = GetDC(None);
        let rows = GetDIBits(
            dc,
            bitmap,
            0,
            h as u32,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut info,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, dc);
        let _ = DeleteObject(bitmap);
        if got == 0 || rows == 0 || pixels.is_empty() {
            return None;
        }
        // Photos come without alpha: make them opaque instead of invisible.
        if pixels.iter().skip(3).step_by(4).all(|&a| a == 0) {
            pixels.iter_mut().skip(3).step_by(4).for_each(|a| *a = 255);
        }
        Some((w as u32, h as u32, pixels))
    }
}

#[cfg(test)]
mod image_tests {
    use super::*;

    #[test]
    fn icon_of_a_program() {
        let exe = std::env::var("SystemRoot").unwrap() + r"\explorer.exe";
        let (w, h, px) = shell_image(Path::new(&exe), 32, false).expect("an icon");
        assert!(w >= 32 && h >= 32);
        assert_eq!(px.len(), (w * h * 4) as usize);
        assert!(
            px.iter().skip(3).step_by(4).any(|&a| a > 0),
            "something visible"
        );
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;

    #[test]
    fn reads_a_shortcut_to_a_folder() {
        use windows::Win32::System::Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER};
        use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
        let dir = std::env::temp_dir().join(format!("files-lnk-{}", std::process::id()));
        let target = dir.join("Zażółć target");
        std::fs::create_dir_all(&target).unwrap();
        let link = dir.join("to target.lnk");
        {
            let _sta = crate::Sta::new().unwrap();
            // SAFETY: COM on this thread; makes the shortcut file.
            unsafe {
                let sl: IShellLinkW =
                    CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
                sl.SetPath(&HSTRING::from(target.as_os_str())).unwrap();
                sl.cast::<IPersistFile>()
                    .unwrap()
                    .Save(&HSTRING::from(link.as_os_str()), true)
                    .unwrap();
            }
        }
        // Temp may be an 8.3 path (KACPER~1); compare where both lead.
        let got = link_target(&link).map(|p| p.canonicalize().unwrap());
        assert_eq!(got, Some(target.canonicalize().unwrap()));
        assert_eq!(link_target(&dir.join("missing.lnk")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod menu_tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetMenuItemCount, GetMenuStringW, MF_BYPOSITION,
    };

    /// The folder background menu builds outside Explorer (no window, no
    /// view) and offers "New".
    #[test]
    fn folder_background_menu_has_items() {
        let _sta = crate::Sta::new().unwrap();
        let dir = std::env::temp_dir();
        let pidls = Pidls::parse(std::slice::from_ref(&dir)).unwrap();
        // SAFETY: as in `folder_menu`.
        unsafe {
            let folder: IShellFolder = SHBindToObject(None, pidls.0[0], None).unwrap();
            let cm: IContextMenu = folder.CreateViewObject(HWND::default()).unwrap();
            let menu = CreatePopupMenu().unwrap();
            cm.QueryContextMenu(menu, 0, 1, 0x7FFF, CMF_NORMAL).unwrap();
            let n = GetMenuItemCount(menu);
            let names: Vec<String> = (0..n)
                .map(|i| {
                    let mut buf = [0u16; 128];
                    let len = GetMenuStringW(menu, i as u32, Some(&mut buf), MF_BYPOSITION);
                    String::from_utf16_lossy(&buf[..len.max(0) as usize])
                })
                .collect();
            let _ = DestroyMenu(menu);
            println!("{names:?}");
            assert!(n > 0, "empty background menu");
        }
    }
}
