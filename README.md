# FilesProject

A lightweight file explorer for Windows, written in Rust. It aims to replace
FilePilot for one user: it must never freeze the PC during file operations,
and it must paste files copied inside a Remote Desktop (RDP) session.

**Status:** M0 — command-line RDP paste prototype (`rdppaste`), in progress.
See [`docs/designs/files_project_m0.md`](docs/designs/files_project_m0.md).

## Build

```sh
cargo build --release
cargo test --workspace --all-features
```

Requires Windows 10/11 x64 and the MSVC build tools. The Rust version is
pinned in `rust-toolchain.toml`.

## `rdppaste` (M0)

Copy files with Ctrl+C in Windows Explorer inside an RDP session, then run
locally:

```sh
rdppaste <target-dir> [--overwrite | --skip | --rename]
rdppaste --formats      # list the formats currently on the clipboard
rdppaste --help
```

| Exit code | Meaning |
| :-- | :-- |
| 0 | done |
| 1 | failed (including problems found before writing) |
| 2 | cancelled (Ctrl+C) |
| 3 | stalled (no progress for 60 s) |
| 4 | no virtual files on the clipboard |
| 5 | files already exist; nothing written (run again with --overwrite, --skip or --rename) |
| 64 | usage error |

The log is written to `%LOCALAPPDATA%\FilesProject\logs\rdppaste.log`. It
contains remote file names and never leaves the machine.

If `rdppaste` fails, press Ctrl+V into the same folder in Windows Explorer:
if Explorer works, the bug is in `rdppaste`; if Explorer fails too, the RDP
client or the server's policy blocks file transfer.

## More

*   [Roadmap](docs/ROADMAP.md)
*   [Architecture](docs/ARCHITECTURE.md)

Licensed under the [MIT licence](LICENSE).
