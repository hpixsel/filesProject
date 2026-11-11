# Replacing Windows Explorer (M6)

Brief: `files_project.md` section 10. The owner chose (2026-11-11): folders,
drives and Win+E open FilesProject; This PC and the Recycle Bin were added
later the same day.

## What is written

Only under `HKCU\Software\Classes`, so no admin rights and nothing for other
users:

| Key | Value | Effect |
| :-- | :-- | :-- |
| `Directory\shell` | default = `FilesProject` | folders open with our verb |
| `Directory\shell\FilesProject\command` | `"<install>\files.exe" "%1"` | |
| `Drive\shell` (and `...\FilesProject\command`) | the same | drives too |
| `CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\opennewwindow\command` | default = `"<install>\files.exe"`, `DelegateExecute` = empty | Win+E |
| `CLSID\{20D04FE0-3AEA-1069-A2D8-08002B30309D}\shell` and `...\open\command` | default verb `open` = `files.exe shell:MyComputerFolder`, `DelegateExecute` = empty | This PC (v0.4.0) |
| `CLSID\{645FF040-5081-101B-9F08-08002B30309D}\shell` and `...\open\command` | default verb `open` = `files.exe shell:RecycleBinFolder`, `DelegateExecute` = empty | Recycle Bin (v0.4.0) |

*   **Shell locations stay with Explorer by design:** Control Panel, MTP
    phones, zip folders and `search-ms` are other classes (`Folder`,
    `CompressedFolder`, CLSIDs), not `Directory` or `Drive`.
*   **This PC and the Recycle Bin** are shown by FilesProject itself
    (`shell:` names, drive tiles, the bin read from `$I`/`$R` files).
*   **Install first:** the command points at the installed copy
    (`files --install`), which keeps its path across updates.
*   **Drives:** `"%1"` for `C:\` arrives as `C:"` (a backslash before a quote
    escapes it), so a trailing `"` in the argument is read as `\`.

## Turning it on and off

*   `files --replace-explorer`:
    1.  exports the keys above that exist (other tools add verbs there,
        e.g. FilePilot and ShareX) to
        `%APPDATA%\FilesProject\explorer-backup\`, once;
    2.  imports the keys above.
*   `files --restore-explorer`:
    1.  imports `restore-explorer.reg` (deletes our keys and the default
        verb);
    2.  imports the backup, then deletes it.
*   **Without the program:** double-click `restore-explorer.reg` (in the
    repo and in every release).
*   Both go through Windows' `reg.exe`, the same format a person can read
    and run.

## Not covered

*   "Show in folder" from other programs (`SHOpenFolderAndSelectItems`) is
    handled by Explorer itself and keeps opening Explorer.
*   A toggle in the window: later, on the settings page.
