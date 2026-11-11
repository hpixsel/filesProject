//! Reading virtual files (`FileGroupDescriptorW` + `FileContents`) from an
//! `IDataObject`, which is what the RDP client puts on the clipboard.

use crate::{Error, Result, Sta};
use files_core::{BadName, Entry, EntryKind, RelPath, MAX_ENTRIES};
use std::mem::size_of;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CLIPBRD_E_CANT_OPEN, FILETIME, HGLOBAL, S_OK};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;
use windows::Win32::System::Com::{
    IDataObject, IStream, DATADIR_GET, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, TYMED,
    TYMED_HGLOBAL, TYMED_ISTREAM,
};
use windows::Win32::System::DataExchange::{GetClipboardFormatNameW, RegisterClipboardFormatW};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{OleGetClipboard, ReleaseStgMedium};
use windows::Win32::UI::Shell::{
    CFSTR_FILECONTENTS, CFSTR_FILEDESCRIPTORW, FD_ATTRIBUTES, FD_FILESIZE, FD_WRITESTIME,
    FILEDESCRIPTORW,
};

/// Retries `f` for up to ~1 s while another process (e.g. clipboard
/// history) holds the clipboard open.
fn retry_open<T>(mut f: impl FnMut() -> windows::core::Result<T>) -> windows::core::Result<T> {
    for _ in 0..20 {
        match f() {
            Err(e) if e.code() == CLIPBRD_E_CANT_OPEN => {
                std::thread::sleep(Duration::from_millis(50))
            }
            r => return r,
        }
    }
    f()
}

pub fn clipboard_object(_sta: &Sta) -> Result<IDataObject> {
    // SAFETY: called on an OLE-initialised thread (proved by `&Sta`).
    Ok(retry_open(|| unsafe { OleGetClipboard() })?)
}

pub(crate) fn cf(name: PCWSTR) -> u16 {
    // SAFETY: `name` is a static NUL-terminated string from windows-rs.
    unsafe { RegisterClipboardFormatW(name) as u16 }
}

pub(crate) fn formatetc(cf: u16, lindex: i32, tymed: u32) -> FORMATETC {
    FORMATETC {
        cfFormat: cf,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex,
        tymed,
    }
}

const STANDARD_FORMATS: [&str; 17] = [
    "CF_TEXT",
    "CF_BITMAP",
    "CF_METAFILEPICT",
    "CF_SYLK",
    "CF_DIF",
    "CF_TIFF",
    "CF_OEMTEXT",
    "CF_DIB",
    "CF_PALETTE",
    "CF_PENDATA",
    "CF_RIFF",
    "CF_WAVE",
    "CF_UNICODETEXT",
    "CF_ENHMETAFILE",
    "CF_HDROP",
    "CF_LOCALE",
    "CF_DIBV5",
];

pub fn format_name(id: u16) -> String {
    if (1..=17).contains(&id) {
        return STANDARD_FORMATS[id as usize - 1].to_string();
    }
    let mut buf = [0u16; 256];
    // SAFETY: buffer is valid for its length.
    let n = unsafe { GetClipboardFormatNameW(id as u32, &mut buf) };
    if n > 0 {
        String::from_utf16_lossy(&buf[..n as usize])
    } else {
        format!("#{id}")
    }
}

/// Names of every format the data object offers.
pub fn formats(obj: &IDataObject) -> Result<Vec<String>> {
    // SAFETY: plain COM calls on a live interface.
    let en = unsafe { obj.EnumFormatEtc(DATADIR_GET.0 as u32)? };
    let mut out = Vec::new();
    loop {
        let mut f = [FORMATETC::default()];
        let mut got = 0u32;
        // SAFETY: `f` and `got` outlive the call.
        let hr = unsafe { en.Next(&mut f, Some(&mut got)) };
        if hr != S_OK || got == 0 {
            break;
        }
        out.push(format_name(f[0].cfFormat));
    }
    Ok(out)
}

/// Releases an `STGMEDIUM` on drop.
struct Medium(STGMEDIUM);

impl Drop for Medium {
    fn drop(&mut self) {
        // SAFETY: the medium came from a successful GetData and is released once.
        unsafe { ReleaseStgMedium(&mut self.0) };
    }
}

/// A locked `HGLOBAL`; unlocks on drop.
struct Locked {
    h: HGLOBAL,
    ptr: *const u8,
    len: usize,
}

impl Locked {
    fn new(h: HGLOBAL) -> Result<Locked> {
        // SAFETY: `h` is a valid global handle owned by the medium we hold.
        let ptr = unsafe { GlobalLock(h) } as *const u8;
        if ptr.is_null() {
            return Err(Error::BadDescriptor("cannot lock clipboard memory"));
        }
        // SAFETY: same handle.
        let len = unsafe { GlobalSize(h) };
        Ok(Locked { h, ptr, len })
    }

    fn bytes(&self) -> &[u8] {
        // SAFETY: GlobalSize bytes are readable while the block is locked.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl Drop for Locked {
    fn drop(&mut self) {
        // SAFETY: balances the GlobalLock in `new`; the result only reports the lock count.
        let _ = unsafe { GlobalUnlock(self.h) };
    }
}

/// The file list of one paste, plus the data object to read contents from.
pub struct VirtualFiles {
    obj: IDataObject,
    entries: Vec<Entry>,
    cf_contents: u16,
}

impl VirtualFiles {
    pub fn from_clipboard(sta: &Sta) -> Result<VirtualFiles> {
        Self::from_data_object(sta, clipboard_object(sta)?)
    }

    pub fn from_data_object(_sta: &Sta, obj: IDataObject) -> Result<VirtualFiles> {
        let fd = formatetc(cf(CFSTR_FILEDESCRIPTORW), -1, TYMED_HGLOBAL.0 as u32);
        // SAFETY: plain COM calls on a live interface.
        let query = retry_open(|| match unsafe { obj.QueryGetData(&fd) } {
            hr if hr == CLIPBRD_E_CANT_OPEN => Err(hr.into()),
            hr => Ok(hr),
        });
        if query.ok() != Some(S_OK) {
            return Err(Error::NoVirtualFiles {
                formats: formats(&obj).unwrap_or_default(),
            });
        }
        // SAFETY: as above; the medium is released by `Medium`.
        let medium = Medium(retry_open(|| unsafe { obj.GetData(&fd) })?);
        if medium.0.tymed != TYMED_HGLOBAL.0 as u32 {
            return Err(Error::BadDescriptor("descriptor is not in global memory"));
        }
        // SAFETY: tymed says the union holds an HGLOBAL.
        let locked = Locked::new(unsafe { medium.0.u.hGlobal })?;
        let entries = parse_descriptors(locked.bytes())?;
        Ok(VirtualFiles {
            obj,
            entries,
            cf_contents: cf(CFSTR_FILECONTENTS),
        })
    }

    /// Entries in descriptor order, folders included. The position in this
    /// slice is the descriptor index used by [`VirtualFiles::open`].
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Opens the contents of entry `index` (its descriptor index / `lindex`).
    pub fn open(&self, index: usize) -> Result<ContentReader> {
        let declared = match self.entries.get(index).map(|e| e.kind) {
            Some(EntryKind::File { size }) => size,
            _ => {
                return Err(Error::BadDescriptor(
                    "open() on a folder or a missing index",
                ))
            }
        };
        let tymed = (TYMED_ISTREAM.0 | TYMED_HGLOBAL.0) as u32;
        let f = formatetc(self.cf_contents, index as i32, tymed);
        // SAFETY: plain COM call; the medium is owned by `Medium` below.
        let medium = Medium(retry_open(|| unsafe { self.obj.GetData(&f) })?);
        match TYMED(medium.0.tymed as i32) {
            TYMED_ISTREAM => {
                // SAFETY: tymed says the union holds an IStream; clone adds a reference.
                let stream = unsafe { (*medium.0.u.pstm).clone() }
                    .ok_or(Error::BadDescriptor("empty stream"))?;
                Ok(ContentReader(Source::Stream {
                    stream,
                    _medium: medium,
                }))
            }
            TYMED_HGLOBAL => {
                // SAFETY: tymed says the union holds an HGLOBAL.
                let locked = Locked::new(unsafe { medium.0.u.hGlobal })?;
                // shortcut: GlobalSize may include allocation padding when no size is declared; reject HGLOBAL without FD_FILESIZE if this shows up
                let len = declared.map_or(locked.len, |d| (d as usize).min(locked.len));
                Ok(ContentReader(Source::Memory {
                    locked,
                    len,
                    pos: 0,
                    _medium: medium,
                }))
            }
            _ => Err(Error::BadDescriptor("unsupported contents medium")),
        }
    }
}

/// `std::io::Read` over one file's contents.
pub struct ContentReader(Source);

// Field order matters: the lock/stream drop before the medium is released.
enum Source {
    Stream {
        stream: IStream,
        _medium: Medium,
    },
    Memory {
        locked: Locked,
        len: usize,
        pos: usize,
        _medium: Medium,
    },
}

impl std::io::Read for ContentReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match &mut self.0 {
            Source::Stream { stream, .. } => {
                let mut got = 0u32;
                let want = buf.len().min(u32::MAX as usize) as u32;
                // SAFETY: `buf` is valid for `want` bytes; S_FALSE is a success code.
                unsafe { stream.Read(buf.as_mut_ptr().cast(), want, Some(&mut got)) }
                    .ok()
                    .map_err(std::io::Error::other)?;
                Ok(got as usize)
            }
            Source::Memory {
                locked, len, pos, ..
            } => {
                let n = (*len - *pos).min(buf.len());
                buf[..n].copy_from_slice(&locked.bytes()[*pos..*pos + n]);
                *pos += n;
                Ok(n)
            }
        }
    }
}

/// Parses a `FILEGROUPDESCRIPTORW` block. Pure: no COM, only bytes.
pub fn parse_descriptors(block: &[u8]) -> Result<Vec<Entry>> {
    let count_bytes: [u8; 4] = block
        .get(..4)
        .and_then(|b| b.try_into().ok())
        .ok_or(Error::BadDescriptor("short block"))?;
    let count = u32::from_ne_bytes(count_bytes) as usize;
    if count > MAX_ENTRIES {
        return Err(Error::BadDescriptor("too many entries"));
    }
    let item = size_of::<FILEDESCRIPTORW>();
    if block.len() < 4 + count * item {
        return Err(Error::BadDescriptor("count exceeds block"));
    }
    let mut entries = Vec::with_capacity(count);
    let mut bad = Vec::new();
    for i in 0..count {
        // SAFETY: bounds checked above; read_unaligned copies the plain-data struct.
        let d: FILEDESCRIPTORW =
            unsafe { std::ptr::read_unaligned(block.as_ptr().add(4 + i * item).cast()) };
        match entry(&d) {
            Ok(e) => entries.push(e),
            Err(b) => bad.push(b),
        }
    }
    if bad.is_empty() {
        Ok(entries)
    } else {
        Err(Error::BadNames(bad))
    }
}

fn entry(d: &FILEDESCRIPTORW) -> std::result::Result<Entry, BadName> {
    let name = &{ d.cFileName }; // copy out of the packed struct
    let Some(end) = name.iter().position(|&c| c == 0) else {
        let input = String::from_utf16_lossy(name);
        return Err(BadName {
            input,
            reason: "descriptor path limit",
        });
    };
    let text = String::from_utf16(&name[..end]).map_err(|_| BadName {
        input: String::from_utf16_lossy(&name[..end]),
        reason: "invalid UTF-16",
    })?;
    let rel_path = RelPath::parse(&text)?;
    let flags = d.dwFlags;
    let has = |f: u32| flags & f != 0;
    let kind =
        if has(FD_ATTRIBUTES.0 as u32) && d.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
            EntryKind::Dir
        } else {
            let size = has(FD_FILESIZE.0 as u32)
                .then(|| (u64::from(d.nFileSizeHigh) << 32) | u64::from(d.nFileSizeLow));
            EntryKind::File { size }
        };
    let modified = has(FD_WRITESTIME.0 as u32)
        .then(|| filetime_to_system(d.ftLastWriteTime))
        .flatten();
    Ok(Entry {
        rel_path,
        kind,
        modified,
    })
}

/// FILETIME (100 ns since 1601) to SystemTime; `None` on overflow.
pub fn filetime_to_system(ft: FILETIME) -> Option<SystemTime> {
    const UNIX_OFFSET: u64 = 116_444_736_000_000_000;
    let t = (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime);
    if t >= UNIX_OFFSET {
        UNIX_EPOCH.checked_add(Duration::from_nanos((t - UNIX_OFFSET).checked_mul(100)?))
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_nanos((UNIX_OFFSET - t).checked_mul(100)?))
    }
}
