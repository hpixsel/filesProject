//! Install and update from GitHub releases (docs/designs/updates.md).
//! Downloads go through Windows' own `curl.exe`, so no HTTP or TLS crate;
//! every file must carry a minisign signature from the release key.

use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const REPO: &str = "https://github.com/hpixsel/filesProject";
/// The release key; the matching secret key signs releases in CI.
const RELEASE_KEY: &str = include_str!("../../../minisign.pub");
/// What a release ships and an install holds (rdppaste sits next to files).
pub const FILES: [&str; 2] = ["files.exe", "rdppaste.exe"];

pub type Result<T> = std::result::Result<T, String>;

/// `%LOCALAPPDATA%\Programs\FilesProject`.
pub fn install_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join(r"Programs\FilesProject"))
}

/// True when this program runs from the install folder, the only copy that
/// updates itself (a build in `target\` must not).
pub fn is_installed() -> bool {
    let here = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf));
    here.is_some() && here == install_dir()
}

/// Copies the programs next to this one into the install folder.
pub fn install() -> Result<PathBuf> {
    let to = install_dir().ok_or("LOCALAPPDATA is not set")?;
    let from = std::env::current_exe().map_err(|e| e.to_string())?;
    let from = from.parent().ok_or("no program folder")?;
    if from == to {
        return Ok(to);
    }
    std::fs::create_dir_all(&to).map_err(|e| format!("{}: {e}", to.display()))?;
    for name in FILES {
        std::fs::copy(from.join(name), to.join(name)).map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(to)
}

fn curl(args: &[&str]) -> Result<Vec<u8>> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let exe = std::env::var_os("SystemRoot")
        .map(|r| PathBuf::from(r).join(r"System32\curl.exe"))
        .unwrap_or_else(|| "curl.exe".into());
    let out = Command::new(exe)
        .args(["--silent", "--show-error", "--fail", "--max-time", "300"])
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(out.stdout)
}

/// The newest release tag, like `v0.2.0`, from where `/releases/latest`
/// redirects (no API token, no JSON).
pub fn latest_tag() -> Result<String> {
    let url = format!("{REPO}/releases/latest");
    let head = curl(&["--head", "--max-time", "20", &url])?;
    tag_from_headers(&String::from_utf8_lossy(&head)).ok_or_else(|| "no release found".into())
}

fn tag_from_headers(headers: &str) -> Option<String> {
    let location = headers.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case("location").then(|| v.trim())
    })?;
    let tag = location.rsplit_once("/tag/")?.1;
    version(tag).map(|_| tag.to_string())
}

fn version(tag: &str) -> Option<(u32, u32, u32)> {
    let mut n = tag.strip_prefix('v')?.split('.').map(|p| p.parse().ok());
    Some((n.next()??, n.next()??, n.next()??))
}

/// True when `tag` (like `v0.2.0`) is newer than `current` (like `0.1.0`).
pub fn is_newer(tag: &str, current: &str) -> bool {
    match (version(tag), version(&format!("v{current}"))) {
        (Some(t), Some(c)) => t > c,
        _ => false,
    }
}

fn verify(file: &Path, signature: &str, key: &str) -> Result<()> {
    let bad = |e: minisign_verify::Error| format!("{}: bad signature ({e})", file.display());
    let key = minisign_verify::PublicKey::decode(key).map_err(bad)?;
    let sig = minisign_verify::Signature::decode(signature).map_err(bad)?;
    let mut check = key.verify_stream(&sig).map_err(bad)?;
    let mut f = std::fs::File::open(file).map_err(|e| e.to_string())?;
    let mut buf = vec![0; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        check.update(&buf[..n]);
    }
    check.finalize().map_err(bad)
}

/// Downloads release `tag` into `staging` and checks every file's
/// signature; nothing is replaced yet.
pub fn download(tag: &str, staging: &Path) -> Result<()> {
    let _ = std::fs::remove_dir_all(staging);
    std::fs::create_dir_all(staging).map_err(|e| e.to_string())?;
    for name in FILES {
        let url = format!("{REPO}/releases/download/{tag}/{name}");
        let file = staging.join(name);
        let path = file.to_string_lossy();
        curl(&["--location", "--output", &path, &url])?;
        let sig = curl(&["--location", &format!("{url}.minisig")])?;
        verify(&file, &String::from_utf8_lossy(&sig), RELEASE_KEY)?;
    }
    Ok(())
}

/// Swaps the staged files in. A running `.exe` can be renamed but not
/// overwritten, so each current file becomes `<name>.old` first; that copy
/// stays for rollback until the next update.
pub fn apply(staging: &Path, dir: &Path) -> Result<()> {
    for name in FILES {
        let (new, cur, old) = (
            staging.join(name),
            dir.join(name),
            dir.join(format!("{name}.old")),
        );
        let _ = std::fs::remove_file(&old);
        if cur.exists() {
            std::fs::rename(&cur, &old).map_err(|e| format!("{name}: {e}"))?;
        }
        if let Err(e) = std::fs::rename(&new, &cur) {
            let _ = std::fs::rename(&old, &cur); // put the old one back
            return Err(format!("{name}: {e}"));
        }
    }
    let _ = std::fs::remove_dir_all(staging);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_and_versions() {
        let h = "HTTP/1.1 302 Found\r\nLocation: https://github.com/hpixsel/filesProject/releases/tag/v0.12.3\r\n\r\n";
        assert_eq!(tag_from_headers(h).as_deref(), Some("v0.12.3"));
        assert_eq!(tag_from_headers("HTTP/1.1 404\r\n"), None);
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("v0.10.0", "0.9.0"), "numeric, not text order");
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("vx", "0.1.0"));
    }

    #[test]
    fn signature_must_match_the_file() {
        // A throwaway key, not the release key.
        let key = "untrusted comment: minisign public key: 67E24D7CB0BF5868\nRWRoWL+wfE3iZ3p9FV6jGBvlbMSo08CPCQ5tuM7rp8wasGrwiw1tF8JK\n";
        let sig = "untrusted comment: signature from rsign secret key\nRURoWL+wfE3iZ42tpMeL/Lxx0jpCxMRWbSHNv4HsCDhfizpfOEaJNJCz2Ul1jI/qDy3LnymVsHHquhD33w95KC4pOLIkW3giygk=\ntrusted comment: timestamp:1794417399\tfile:hello.bin\tprehashed\ntQOJ/mVY4VM8I7kRTfYTePqnvtDtJJVzIs5a81Ns+L8/vTYDyQFasYR6AMpiiLW/J48V2ytXkUxULLopZ1/9AQ==\n";
        let f = std::env::temp_dir().join(format!("files-sig-{}", std::process::id()));
        std::fs::write(&f, b"hello").unwrap();
        assert!(verify(&f, sig, key).is_ok());
        std::fs::write(&f, b"hellO").unwrap();
        assert!(verify(&f, sig, key).is_err());
        assert!(verify(&f, sig, RELEASE_KEY).is_err(), "other key");
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn apply_keeps_the_old_copy() {
        let d = std::env::temp_dir().join(format!("files-apply-{}", std::process::id()));
        let staging = d.join("update");
        std::fs::create_dir_all(&staging).unwrap();
        for name in FILES {
            std::fs::write(d.join(name), b"old").unwrap();
            std::fs::write(staging.join(name), b"new").unwrap();
        }
        apply(&staging, &d).unwrap();
        for name in FILES {
            assert_eq!(std::fs::read(d.join(name)).unwrap(), b"new");
            assert_eq!(
                std::fs::read(d.join(format!("{name}.old"))).unwrap(),
                b"old"
            );
        }
        assert!(!staging.exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
