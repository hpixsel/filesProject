# Manual test procedure for `rdppaste`

Build first:

```powershell
cargo build --release -p rdppaste
```

The program is then at `.\target\release\rdppaste.exe`.

## 0. Local check without RDP (2 minutes)

Windows Explorer serves files from inside a `.zip` as virtual files, in
the same format the RDP client uses. That makes it a quick local check.

1.  In Explorer, open any `.zip` and select a few files and a folder
    inside it.
2.  Press Ctrl+C.
3.  Run `rdppaste --formats`. The list should include
    `FileGroupDescriptorW` and `FileContents`.
4.  Run `rdppaste C:\Temp\paste-test`. The target folder must already
    exist. Compare the result with the contents of the zip.
5.  Run the same command again. It should stop with "already exists (use
    --overwrite)" and write nothing.

## 1. Test rig for RDP

*   First try loopback: run `mstsc /v:localhost` (or `127.0.0.2`) with
    clipboard redirection turned on.
*   If loopback does not work, create a Hyper-V Windows 11 evaluation VM,
    enable Remote Desktop in it, and connect with `mstsc`.
*   Finally, run the tests once against a real work server.

## 2. Checklist

| Case | Steps | Expected |
| :-- | :-- | :-- |
| CUJ 1 | Ctrl+C a 5 GiB file on the server, then `rdppaste D:\in` | The `Get-FileHash` value matches the server; no `.partial` left |
| CUJ 2 | Ctrl+C a folder with about 2,000 files, run `rdppaste`, press Ctrl+C halfway (with no text selected) | Exit 2 within 1 s; finished files kept; no `.partial`; "n/m files completed" printed |
| CUJ 3 | Text on the clipboard | Exit 4 and a list of formats |
| CUJ 6 | During a large paste, disconnect the session or unplug the network | Exit 3 within 61 s; no `.partial` |
| Q6a | During a paste, copy something else on the server | Record what happens |
| Q6b | During a paste, copy text locally in Notepad | Record what happens |

## 3. G2 measurement

Start this 10 s before the copy:

```powershell
typeperf "\Memory\Modified Page List Bytes" "\Memory\Standby Cache Normal Priority Bytes" "\Memory\Standby Cache Reserve Bytes" "\Memory\Standby Cache Core Bytes" -si 1 -f CSV -o g2-system.csv
```

Once `rdppaste` is running, start this as well:

```powershell
typeperf "\Process(rdppaste)\Private Bytes" -si 1 -f CSV -o g2-proc.csv
```

During the copy, type in Notepad and switch windows.

**Limits:**

*   the program's RAM stays under 150 MB;
*   Modified pages grow by 512 MB or less;
*   the Standby counters, added together, grow by 512 MB or less;
*   no pause longer than 1 s.

**Q1 baseline:** repeat the same copy with Ctrl+V in Windows Explorer and
measure it the same way.

## 4. Checking the server

If both `rdppaste` and Explorer fail:

*   run `gpresult /h report.html` on the server and look for the policies
    "Do not allow Clipboard redirection" and "Restrict clipboard transfer";
*   run `rdppaste --formats`. If the descriptor formats are missing, the
    RDP client or the server does not pass files through the clipboard.

## 5. Diagnosis

If `rdppaste` fails, press Ctrl+V into the same folder in Windows Explorer:

*   **Explorer works:** the defect is in `rdppaste`. Send the log from
    `%LOCALAPPDATA%\FilesProject\logs\rdppaste.log`.
*   **Explorer fails too:** the cause is the RDP client, the server policy,
    or permissions on the folder.

Record every result in `docs/HANDOFF.md` under "Experiments".
