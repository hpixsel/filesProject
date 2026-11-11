//! Opening folders, drives and Win+E with FilesProject instead of Explorer,
//! and undoing it (docs/designs/replace_explorer.md). Everything goes
//! through Windows' `reg.exe` and `.reg` text, under HKCU only.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub type Result<T> = std::result::Result<T, String>;

/// The keys we change; exported before the first change.
const KEYS: [&str; 3] = [
    r"HKCU\Software\Classes\Directory\shell",
    r"HKCU\Software\Classes\Drive\shell",
    r"HKCU\Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\opennewwindow",
];

/// Undoes our keys; also shipped for a double-click without the program.
const RESTORE: &str = include_str!("../../../restore-explorer.reg");

fn backup_dir() -> Option<PathBuf> {
    super::favourites::file().map(|f| f.with_file_name("explorer-backup"))
}

fn reg(args: &[&str]) -> Result<()> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let exe = std::env::var_os("SystemRoot")
        .map(|r| PathBuf::from(r).join(r"System32\reg.exe"))
        .unwrap_or_else(|| "reg.exe".into());
    let out = Command::new(exe)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("reg: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// `reg import` of `.reg` text, written as UTF-16 so any path survives.
fn import(text: &str) -> Result<()> {
    let file = std::env::temp_dir().join(format!("filesproject-{}.reg", std::process::id()));
    let bytes: Vec<u8> = std::iter::once(0xFEFF)
        .chain(text.encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect();
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    let r = reg(&["import", &file.to_string_lossy()]);
    let _ = std::fs::remove_file(&file);
    r
}

/// `.reg` text pointing folders, drives and Win+E at `exe`.
fn replace_reg(exe: &Path) -> String {
    // .reg strings escape `\` and `"`.
    let q = |s: &str| s.replace('\\', r"\\").replace('"', r#"\""#);
    let exe = exe.to_string_lossy();
    let open = q(&format!(r#""{exe}" "%1""#));
    let plain = q(&format!(r#""{exe}""#));
    let mut text = String::from("Windows Registry Editor Version 5.00\r\n");
    for class in ["Directory", "Drive"] {
        let base = format!(r"HKEY_CURRENT_USER\Software\Classes\{class}\shell");
        text += &format!("\r\n[{base}]\r\n@=\"FilesProject\"\r\n");
        text += &format!("\r\n[{base}\\FilesProject]\r\n@=\"Open in FilesProject\"\r\n");
        text += &format!("\r\n[{base}\\FilesProject\\command]\r\n@=\"{open}\"\r\n");
    }
    text += &format!(
        "\r\n[HKEY_CURRENT_USER\\Software\\Classes\\CLSID\\{{52205fd8-5dfb-447d-801a-d0b52f2e83e1}}\\shell\\opennewwindow\\command]\r\n@=\"{plain}\"\r\n\"DelegateExecute\"=\"\"\r\n"
    );
    text
}

/// Folders, drives and Win+E open the installed FilesProject.
pub fn replace() -> Result<()> {
    let exe = super::update::install_dir()
        .map(|d| d.join("files.exe"))
        .filter(|e| e.exists())
        .ok_or("install first: files --install")?;
    let backup = backup_dir().ok_or("APPDATA is not set")?;
    // Once: a second run would back up our own keys.
    if !backup.exists() {
        std::fs::create_dir_all(&backup).map_err(|e| e.to_string())?;
        for (i, key) in KEYS.iter().enumerate() {
            if reg(&["query", key]).is_ok() {
                let file = backup.join(format!("{i}.reg"));
                reg(&["export", key, &file.to_string_lossy(), "/y"])?;
            }
        }
    }
    import(&replace_reg(&exe))
}

/// Explorer again: our keys go, then whatever was there before comes back.
pub fn restore() -> Result<()> {
    import(RESTORE)?;
    let Some(backup) = backup_dir().filter(|b| b.exists()) else {
        return Ok(());
    };
    let mut files: Vec<_> = std::fs::read_dir(&backup)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .collect();
    files.sort();
    for f in files {
        reg(&["import", &f.to_string_lossy()])?;
    }
    std::fs::remove_dir_all(&backup).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reg_text_escapes_the_path() {
        let text = replace_reg(Path::new(r"C:\Users\Zażółć Ąę\files.exe"));
        assert!(text.contains(r#"@="\"C:\\Users\\Zażółć Ąę\\files.exe\" \"%1\"""#));
        assert!(text.contains(r#"@="\"C:\\Users\\Zażółć Ąę\\files.exe\"""#));
        assert!(text.contains("\"DelegateExecute\"=\"\""));
        assert_eq!(text.matches("@=\"FilesProject\"").count(), 2);
        // What restore removes is what replace adds.
        for key in [
            r"Directory\shell\FilesProject]",
            r"Drive\shell\FilesProject]",
            "opennewwindow]",
        ] {
            assert!(RESTORE.contains(key), "{key}");
        }
    }
}
