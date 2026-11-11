//! One window per user: `files <folder>` hands the folder to the running
//! window through a named pipe (a new tab there), then exits.

use crate::Result;
use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use windows::core::HSTRING;
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};

/// Longest message read; a path is far shorter.
const MAX_MESSAGE: u64 = 64 * 1024;

fn pipe_name() -> String {
    let user = std::env::var("USERNAME").unwrap_or_default();
    format!(r"\\.\pipe\FilesProject-{user}")
}

/// Sends `dir` to the running window. False when no window is running.
pub fn send(dir: &Path) -> bool {
    send_to(&pipe_name(), dir)
}

fn send_to(name: &str, dir: &Path) -> bool {
    const ERROR_PIPE_BUSY: i32 = 231;
    // Busy for a moment while the window takes the previous message.
    let mut tries = 0;
    let mut pipe = loop {
        match std::fs::OpenOptions::new().write(true).open(name) {
            Ok(p) => break p,
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && tries < 40 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return false,
        }
    };
    // Lets the running window come to the front; this process has the
    // foreground right because the user just started it.
    // SAFETY: no pointers.
    let _ = unsafe {
        windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(
            windows::Win32::UI::WindowsAndMessaging::ASFW_ANY,
        )
    };
    pipe.write_all(dir.to_string_lossy().as_bytes()).is_ok()
}

/// Becomes the running window: `on_dir` gets each folder sent by a later
/// `files <folder>`, on a background thread. Fails when another window
/// already serves. The pipe takes local clients only; Windows' default
/// pipe security lets only this user (and admins) write to it.
pub fn serve(on_dir: impl Fn(PathBuf) + Send + 'static) -> Result<()> {
    serve_at(&pipe_name(), on_dir)
}

fn serve_at(name: &str, on_dir: impl Fn(PathBuf) + Send + 'static) -> Result<()> {
    // SAFETY: a valid name; the handle is checked before use.
    let handle = unsafe {
        CreateNamedPipeW(
            &HSTRING::from(name),
            PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            0,
            4096,
            0,
            None,
        )
    };
    if handle.is_invalid() {
        return Err(windows::core::Error::from_win32().into());
    }
    // SAFETY: a fresh, valid handle owned from here on by `pipe`.
    let pipe = unsafe { File::from_raw_handle(handle.0) };
    std::thread::spawn(move || loop {
        let handle = windows::Win32::Foundation::HANDLE(pipe.as_raw_handle());
        // SAFETY: `pipe` keeps the handle open. A client that came before
        // this call gives "already connected", or "no data" when it has
        // already written and closed; its message is still buffered.
        let connected = unsafe { ConnectNamedPipe(handle, None) };
        if let Err(e) = connected {
            use windows::Win32::Foundation::{ERROR_NO_DATA, ERROR_PIPE_CONNECTED};
            if e.code() != ERROR_PIPE_CONNECTED.to_hresult()
                && e.code() != ERROR_NO_DATA.to_hresult()
            {
                tracing::warn!("folder pipe: {e}");
                // SAFETY: as above.
                let _ = unsafe { DisconnectNamedPipe(handle) };
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
        }
        let mut text = String::new();
        let read = (&pipe).take(MAX_MESSAGE).read_to_string(&mut text);
        // SAFETY: as above; frees the pipe for the next client.
        let _ = unsafe { DisconnectNamedPipe(handle) };
        if read.is_ok() && !text.trim().is_empty() {
            on_dir(PathBuf::from(text.trim()));
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_instance_hands_over_its_folder() {
        let name = format!(r"\\.\pipe\FilesProject-test-{}", std::process::id());
        let (tx, rx) = std::sync::mpsc::channel();
        serve_at(&name, move |p| tx.send(p).unwrap()).unwrap();
        assert!(serve_at(&name, |_| {}).is_err(), "only one server");
        // Back-to-back sends hit the races: a client that closes before the
        // server waits, and a pipe busy between two clients.
        let dirs: Vec<String> = (0..20).map(|i| format!(r"C:\Zażółć\{i}")).collect();
        for dir in &dirs {
            assert!(send_to(&name, Path::new(dir)));
        }
        for dir in &dirs {
            let got = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
            assert_eq!(&got, Path::new(dir));
        }
        assert!(!send_to(
            r"\\.\pipe\FilesProject-nobody-here",
            Path::new("x")
        ));
    }
}
