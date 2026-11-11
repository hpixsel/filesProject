use std::marker::PhantomData;
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};

/// OLE single-threaded apartment for the current thread. `!Send`, so COM
/// objects obtained while it lives stay on this thread.
pub struct Sta(PhantomData<*const ()>);

impl Sta {
    pub fn new() -> crate::Result<Sta> {
        // SAFETY: plain OLE init on the current thread; balanced in Drop.
        unsafe { OleInitialize(None)? };
        Ok(Sta(PhantomData))
    }
}

impl Drop for Sta {
    fn drop(&mut self) {
        // SAFETY: matches the successful OleInitialize in `new`.
        unsafe { OleUninitialize() };
    }
}
