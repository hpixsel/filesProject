# Architecture

```text
rdppaste (bin)  ──────────────┐
   │ diagnostics (--formats)   v
   │                      files-jobs ──> files-core
   └────────────────────> files-win  ──> files-core
```

Arrows mean "depends on". From M1, `files-app` and `files-ui-gpui` sit
above `files-jobs`.

## Crates

*   **`files-core`** holds plain data and rules: `Entry`, `Progress`, the
    validation of untrusted names (`RelPath`), and the whole-list check
    before writing (`check`). It must not call Windows or use `unsafe`, so
    it stays testable anywhere.
*   **`files-win`** is the only crate allowed to use `unsafe` and COM, OLE
    and Win32 calls. It contains:
    *   `Sta`, the OLE apartment guard;
    *   `VirtualFiles`, which reads `FileGroupDescriptorW` and
        `FileContents` with bounds checks;
    *   `io`: background mode, `AlignedBuf`, delete-on-close partial
        files, a rename that never replaces, free space, and long paths;
    *   `fake`, a COM test double of the RDP data object, behind the
        `fake` feature.

    It exposes safe Rust types only.
*   **`files-jobs`** runs the work. It contains:
    *   `start`, `JobHandle::wait`: one thread per job, a bounded progress
        channel, cancel, and a stall watchdog;
    *   `paste`, the paste job.

    It must not use `unsafe`.
*   **`rdppaste`** is the command-line front end: arguments, progress line,
    exit codes, and logging.

## Threading and process rules

*   **COM objects stay on the thread that got them.** The paste job thread
    creates its own `Sta` and gets the clipboard itself.
*   **No lock is held across a COM call**, because COM pumps messages
    during calls into the RDP client.
*   **Job threads run in background mode**, which gives them low I/O and
    memory priority.
*   **Large files, and files of unknown size, are written unbuffered** so
    they do not fill the Windows file cache.
*   **A blocked COM read cannot be interrupted.**
    *   After cancel, `wait` gives the job 900 ms to stop. If no progress
        arrives for 60 s, the job is declared stalled.
    *   In both cases the caller abandons the thread and the process exits.
    *   `.partial` files are delete-on-close, so Windows removes them on
        every exit path.
*   **From M1, the GUI runs each paste as a child `rdppaste` process**, so
    that it can kill a paste that is stuck (Q8).

The full design is in `docs/designs/files_project_m0.md`.
