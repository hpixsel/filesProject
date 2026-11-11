//! Test double for the RDP client's clipboard data object (feature `fake`).
//! Build a `FakeFiles` anywhere (it is `Send`), then call
//! `into_data_object` on the thread that will use it.

use crate::clipboard::{cf, formatetc};
use std::mem::{size_of, ManuallyDrop};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows::core::{implement, Result, HRESULT};
use windows::Win32::Foundation::{
    BOOL, DV_E_FORMATETC, E_FAIL, E_NOTIMPL, FILETIME, OLE_E_ADVISENOTSUPPORTED, S_FALSE, S_OK,
};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::System::Com::{
    IAdviseSink, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA,
    ISequentialStream_Impl, IStream, IStream_Impl, FORMATETC, LOCKTYPE, STATFLAG, STATSTG, STGC,
    STGMEDIUM, STGMEDIUM_0, STREAM_SEEK, TYMED_HGLOBAL, TYMED_ISTREAM,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::Shell::{
    SHCreateStdEnumFmtEtc, CFSTR_FILECONTENTS, CFSTR_FILEDESCRIPTORW, FD_ATTRIBUTES, FD_FILESIZE,
    FD_WRITESTIME, FILEDESCRIPTORW,
};

#[derive(Clone, Debug)]
pub enum FakeContent {
    Bytes(Vec<u8>),
    /// Deterministic pseudo-random bytes; see [`generated`].
    Generated {
        len: u64,
        seed: u64,
    },
    /// Valid only with `Medium::Stream`. `delay` is slept before every Read.
    Faulty {
        inner: Box<FakeContent>,
        delay: Duration,
        fail_after: Option<u64>,
        hang_after: Option<u64>,
        release: Option<Arc<AtomicBool>>,
    },
}

impl FakeContent {
    pub fn len(&self) -> u64 {
        match self {
            FakeContent::Bytes(b) => b.len() as u64,
            FakeContent::Generated { len, .. } => *len,
            FakeContent::Faulty { inner, .. } => inner.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Copies bytes starting at `pos`; `Err` makes the stream Read fail.
    fn read_at(&self, pos: u64, buf: &mut [u8]) -> std::result::Result<usize, HRESULT> {
        match self {
            FakeContent::Bytes(b) => {
                let start = (pos as usize).min(b.len());
                let n = (b.len() - start).min(buf.len());
                buf[..n].copy_from_slice(&b[start..start + n]);
                Ok(n)
            }
            FakeContent::Generated { len, seed } => {
                let n = (len.saturating_sub(pos) as usize).min(buf.len());
                generated(*seed, pos, &mut buf[..n]);
                Ok(n)
            }
            FakeContent::Faulty {
                inner,
                delay,
                fail_after,
                hang_after,
                release,
            } => {
                std::thread::sleep(*delay);
                if fail_after.is_some_and(|f| pos >= f) {
                    return Err(E_FAIL);
                }
                if hang_after.is_some_and(|h| pos >= h) {
                    while !release.as_ref().is_some_and(|r| r.load(Ordering::SeqCst)) {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    return Err(E_FAIL); // released: behave like a dropped session
                }
                // Stop at the next fault boundary so it triggers on the next Read.
                let limit = [*fail_after, *hang_after]
                    .into_iter()
                    .flatten()
                    .map(|b| (b - pos) as usize)
                    .min()
                    .unwrap_or(usize::MAX);
                let take = buf.len().min(limit);
                inner.read_at(pos, &mut buf[..take])
            }
        }
    }
}

/// Fills `buf` with the bytes of the `Generated { seed }` stream at `offset`.
/// Tests use it to verify written files without a hash crate.
pub fn generated(seed: u64, offset: u64, buf: &mut [u8]) {
    for (i, b) in buf.iter_mut().enumerate() {
        let n = offset + i as u64;
        // xorshift64* of the 8-byte block index, so any offset is cheap to compute.
        let mut x = seed ^ (n / 8).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x2545_F491;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        *b = (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> ((n % 8) * 8)) as u8;
    }
}

#[derive(Clone, Copy, Debug)]
pub enum DeclaredSize {
    /// The content length.
    Real,
    None,
    Value(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Medium {
    Stream,
    Memory,
}

#[derive(Clone, Debug)]
pub struct FakeFile {
    content: FakeContent,
    size: DeclaredSize,
    modified: Option<SystemTime>,
    medium: Medium,
    attributes: bool,
}

impl FakeFile {
    pub fn new(content: FakeContent) -> FakeFile {
        FakeFile {
            content,
            size: DeclaredSize::Real,
            modified: None,
            medium: Medium::Stream,
            attributes: true,
        }
    }
    pub fn size(mut self, s: DeclaredSize) -> Self {
        self.size = s;
        self
    }
    pub fn modified(mut self, t: SystemTime) -> Self {
        self.modified = Some(t);
        self
    }
    pub fn memory(mut self) -> Self {
        self.medium = Medium::Memory;
        self
    }
    pub fn no_attributes(mut self) -> Self {
        self.attributes = false;
        self
    }
}

#[derive(Clone, Debug)]
enum Item {
    Dir(String),
    File(String, FakeFile),
    Raw(Box<[u16; 260]>),
}

/// Builder for a fake clipboard data object. Holds no COM objects.
#[derive(Clone, Debug, Default)]
pub struct FakeFiles {
    items: Vec<Item>,
    extra: Vec<u16>,
    no_descriptors: bool,
    raw_count: Option<u32>,
    short_block: bool,
}

impl FakeFiles {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn dir(mut self, name: &str) -> Self {
        self.items.push(Item::Dir(name.into()));
        self
    }
    pub fn file(mut self, name: &str, f: FakeFile) -> Self {
        self.items.push(Item::File(name.into(), f));
        self
    }
    /// A file entry with exactly these name units, empty, no declared size.
    pub fn raw_name(mut self, units: [u16; 260]) -> Self {
        self.items.push(Item::Raw(Box::new(units)));
        self
    }
    pub fn extra_format(mut self, cf: u16) -> Self {
        self.extra.push(cf);
        self
    }
    pub fn without_descriptors(mut self) -> Self {
        self.no_descriptors = true;
        self
    }
    /// Overrides `cItems` in the descriptor block.
    pub fn raw_count(mut self, n: u32) -> Self {
        self.raw_count = Some(n);
        self
    }
    /// Makes the descriptor block 2 bytes long.
    pub fn short_block(mut self) -> Self {
        self.short_block = true;
        self
    }

    fn descriptor_block(&self) -> Vec<u8> {
        if self.short_block {
            return vec![0, 0];
        }
        let count = self.raw_count.unwrap_or(self.items.len() as u32);
        let mut block = count.to_ne_bytes().to_vec();
        for item in &self.items {
            let mut d = FILEDESCRIPTORW::default();
            let mut flags = 0u32;
            let name: Vec<u16> = match item {
                Item::Dir(n) => {
                    flags |= FD_ATTRIBUTES.0 as u32;
                    d.dwFileAttributes = FILE_ATTRIBUTE_DIRECTORY.0;
                    n.encode_utf16().collect()
                }
                Item::File(n, f) => {
                    if f.attributes {
                        flags |= FD_ATTRIBUTES.0 as u32;
                        d.dwFileAttributes = FILE_ATTRIBUTE_NORMAL.0;
                    }
                    let size = match f.size {
                        DeclaredSize::Real => Some(f.content.len()),
                        DeclaredSize::None => None,
                        DeclaredSize::Value(v) => Some(v),
                    };
                    if let Some(s) = size {
                        flags |= FD_FILESIZE.0 as u32;
                        d.nFileSizeHigh = (s >> 32) as u32;
                        d.nFileSizeLow = s as u32;
                    }
                    if let Some(t) = f.modified {
                        flags |= FD_WRITESTIME.0 as u32;
                        d.ftLastWriteTime = to_filetime(t);
                    }
                    n.encode_utf16().collect()
                }
                Item::Raw(units) => {
                    flags |= FD_ATTRIBUTES.0 as u32;
                    d.dwFileAttributes = FILE_ATTRIBUTE_NORMAL.0;
                    d.cFileName = **units;
                    Vec::new()
                }
            };
            if !name.is_empty() {
                assert!(
                    name.len() < 260,
                    "fake names must fit the 260-unit descriptor field; use raw_name"
                );
                let mut cname = [0u16; 260];
                cname[..name.len()].copy_from_slice(&name);
                d.cFileName = cname;
            }
            d.dwFlags = flags;
            // SAFETY: FILEDESCRIPTORW is plain data; we copy its bytes.
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    (&d as *const FILEDESCRIPTORW).cast::<u8>(),
                    size_of::<FILEDESCRIPTORW>(),
                )
            };
            block.extend_from_slice(bytes);
        }
        block
    }

    /// Builds the COM object. Call on the thread that will use it.
    pub fn into_data_object(self) -> IDataObject {
        let cf_desc = cf(CFSTR_FILEDESCRIPTORW);
        let cf_contents = cf(CFSTR_FILECONTENTS);
        let mut formats = Vec::new();
        if !self.no_descriptors {
            formats.push(formatetc(cf_desc, -1, TYMED_HGLOBAL.0 as u32));
            formats.push(formatetc(
                cf_contents,
                -1,
                (TYMED_ISTREAM.0 | TYMED_HGLOBAL.0) as u32,
            ));
        }
        for &c in &self.extra {
            formats.push(formatetc(c, -1, TYMED_HGLOBAL.0 as u32));
        }
        let descriptor = self.descriptor_block();
        DataObject {
            files: self,
            formats,
            descriptor,
            cf_desc,
            cf_contents,
        }
        .into()
    }
}

fn to_filetime(t: SystemTime) -> FILETIME {
    let ns100 = t
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() / 100) as u64
        + 116_444_736_000_000_000;
    FILETIME {
        dwLowDateTime: ns100 as u32,
        dwHighDateTime: (ns100 >> 32) as u32,
    }
}

fn hglobal_medium(data: &[u8]) -> Result<STGMEDIUM> {
    // SAFETY: fresh allocation of data.len() bytes, locked while copying.
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1))?;
        let p = GlobalLock(h) as *mut u8;
        std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
        let _ = GlobalUnlock(h);
        Ok(STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: STGMEDIUM_0 { hGlobal: h },
            pUnkForRelease: ManuallyDrop::new(None),
        })
    }
}

#[implement(IDataObject)]
struct DataObject {
    files: FakeFiles,
    formats: Vec<FORMATETC>,
    descriptor: Vec<u8>,
    cf_desc: u16,
    cf_contents: u16,
}

impl IDataObject_Impl for DataObject_Impl {
    fn GetData(&self, pformatetcin: *const FORMATETC) -> Result<STGMEDIUM> {
        // SAFETY: COM guarantees a valid pointer for the call.
        let f = unsafe { &*pformatetcin };
        if f.cfFormat == self.cf_desc && !self.files.no_descriptors {
            return hglobal_medium(&self.descriptor);
        }
        if f.cfFormat == self.cf_contents && !self.files.no_descriptors {
            let content = match self.files.items.get(f.lindex as usize) {
                Some(Item::File(_, file)) => file,
                Some(Item::Raw(_)) => return hglobal_medium(&[]),
                _ => return Err(DV_E_FORMATETC.into()),
            };
            return match content.medium {
                Medium::Memory => {
                    let mut data = vec![0u8; content.content.len() as usize];
                    let _ = content.content.read_at(0, &mut data);
                    hglobal_medium(&data)
                }
                Medium::Stream => {
                    let stream: IStream = FakeStream {
                        content: content.content.clone(),
                        pos: AtomicU64::new(0),
                    }
                    .into();
                    Ok(STGMEDIUM {
                        tymed: TYMED_ISTREAM.0 as u32,
                        u: STGMEDIUM_0 {
                            pstm: ManuallyDrop::new(Some(stream)),
                        },
                        pUnkForRelease: ManuallyDrop::new(None),
                    })
                }
            };
        }
        if self.files.extra.contains(&f.cfFormat) {
            return hglobal_medium(&[0, 0]);
        }
        Err(DV_E_FORMATETC.into())
    }

    fn GetDataHere(&self, _: *const FORMATETC, _: *mut STGMEDIUM) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn QueryGetData(&self, pformatetc: *const FORMATETC) -> HRESULT {
        // SAFETY: COM guarantees a valid pointer for the call.
        let f = unsafe { &*pformatetc };
        let known = self.formats.iter().any(|k| k.cfFormat == f.cfFormat);
        if known {
            S_OK
        } else {
            DV_E_FORMATETC
        }
    }

    fn GetCanonicalFormatEtc(&self, _: *const FORMATETC, _: *mut FORMATETC) -> HRESULT {
        E_NOTIMPL
    }

    fn SetData(&self, _: *const FORMATETC, _: *const STGMEDIUM, _: BOOL) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn EnumFormatEtc(&self, _dwdirection: u32) -> Result<IEnumFORMATETC> {
        // SAFETY: the slice is valid for the call; the shell copies it.
        unsafe { SHCreateStdEnumFmtEtc(&self.formats) }
    }

    fn DAdvise(&self, _: *const FORMATETC, _: u32, _: Option<&IAdviseSink>) -> Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn DUnadvise(&self, _: u32) -> Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn EnumDAdvise(&self) -> Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}

#[implement(IStream)]
struct FakeStream {
    content: FakeContent,
    pos: AtomicU64,
}

impl ISequentialStream_Impl for FakeStream_Impl {
    fn Read(&self, pv: *mut core::ffi::c_void, cb: u32, pcbread: *mut u32) -> HRESULT {
        // SAFETY: COM guarantees `pv` is valid for `cb` bytes.
        let buf = unsafe { std::slice::from_raw_parts_mut(pv.cast::<u8>(), cb as usize) };
        let pos = self.pos.load(Ordering::SeqCst);
        match self.content.read_at(pos, buf) {
            Ok(n) => {
                self.pos.store(pos + n as u64, Ordering::SeqCst);
                if !pcbread.is_null() {
                    // SAFETY: caller-provided out pointer.
                    unsafe { *pcbread = n as u32 };
                }
                if n < cb as usize {
                    S_FALSE
                } else {
                    S_OK
                }
            }
            Err(hr) => hr,
        }
    }

    fn Write(&self, _: *const core::ffi::c_void, _: u32, _: *mut u32) -> HRESULT {
        E_NOTIMPL
    }
}

impl IStream_Impl for FakeStream_Impl {
    fn Seek(&self, _: i64, _: STREAM_SEEK, _: *mut u64) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn SetSize(&self, _: u64) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn CopyTo(&self, _: Option<&IStream>, _: u64, _: *mut u64, _: *mut u64) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn Commit(&self, _: &STGC) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn Revert(&self) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn LockRegion(&self, _: u64, _: u64, _: &LOCKTYPE) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn UnlockRegion(&self, _: u64, _: u64, _: u32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn Stat(&self, _: *mut STATSTG, _: &STATFLAG) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn Clone(&self) -> Result<IStream> {
        Err(E_NOTIMPL.into())
    }
}
