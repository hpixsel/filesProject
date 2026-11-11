# Stage 9: Implementation Readiness Report (round 1)

*   **Target:** `docs/designs/files_project_m0.md`, DetailedRange lines 763–1456
*   **Reviewers** (fresh context, all completed):
    *   `egm-implementation-readiness`, using the implementation-readiness
        standard;
    *   `egm-codebase-details`, using the implementation-readiness standard
        and checking Rust std, Cargo, windows-rs and Win32 from memory;
    *   `egm-style`.
*   **Date:** 2026-11-11
*   **Verdict:** NOT READY. There are no blockers. After deduplication there
    are 20 major findings (raw counts: 14 + 13 + 11) and the nits listed at
    the end. In the reviewers' words: "the file list, signatures and test
    lists are much better than average". The gaps sit in four places: the
    unbuffered write loop, the cancel and stall contract, arithmetic and
    bounds on untrusted data, and the test fake.

## Interaction with stage 8 (Z1–Z3, not yet applied)

Accepting Z1–Z3 from `intermediate_zoom_out_report.md` makes six of the
findings below disappear without further work:

| Stage 8 change | Findings it removes |
| :-- | :-- |
| Z1: drop `--diagnose-shell` | R9 and part of R2 (the `MK_LBUTTON` and `WindowsAndMessaging` features) |
| Z2: no `--json-progress` in M0 | R11 and R12 |
| Z3: delete-on-close for `.partial` | R10 and the path-publishing part of R7 |

## Major (deduplicated)

### Build and compilation

*   **R1. No phases.** *(readiness M1)*
    *   **Problem:** the plan is a flat list of 29 items, with no checkpoint
        that must build and test green.
    *   **Fix:** five phases, each with an exit check:
        *   P1: skeleton plus `files-core`;
        *   P2: `files-win`;
        *   P3: `files-jobs`;
        *   P4: `rdppaste`;
        *   P5: documents plus the manual RDP run.
*   **R2. Missing `windows` features and crates.** *(readiness M2,
    details M1, N3)*
    *   **Problem:** these are missing:
        *   `Win32_UI_WindowsAndMessaging` for the message pump;
        *   `Win32_System_SystemServices` for `MK_LBUTTON` and the
            `IDropTarget` methods;
        *   the `implement` feature and a direct `windows-core` dependency
            for `#[implement]`.
    *   **Fix:** add them all, and pin the windows-rs version in the plan.
*   **R3. Edition and repository URL left open.** *(readiness M14,
    details N4)*
    *   **Problem:** edition 2024 needs Rust 1.85 and makes `set_var`
        unsafe.
    *   **Fix:** state `edition = "2021"` (or 2024) and the URL.
*   **R4. `tests/virtual_files.rs` does not compile without the `fake`
    feature.** *(details N5)*
    *   **Fix:** add `required-features = ["fake"]`.

### Safety and untrusted data

*   **R5. Out-of-bounds read of `FILEGROUPDESCRIPTORW`.** *(readiness M3,
    details M2; also stage 8 nit 5)*
    *   **Fix:**
        *   check `GlobalSize >= 4 + cItems * sizeof(FILEDESCRIPTORW)`;
        *   check `cItems <= MAX_ENTRIES` before reading any entry;
        *   read `cFileName` up to the first NUL. A name with no NUL in 260
            units is a `BadName` ("descriptor limit"), which also
            implements R3 from the specification;
        *   add `Error::BadDescriptor`;
        *   add a test with `cItems` larger than the block.
*   **R6. `AlignedBuf` is undefined behaviour** (`alloc` returns
    uninitialised memory). *(details M4)*
    *   **Fix:** use `alloc_zeroed`, call `handle_alloc_error` when it
        returns null, and `dealloc` with the same layout.
*   **R7. Data loss: a `.partial` that is not ours gets deleted.**
    *(readiness M8)*
    *   **Problem:** `set_partial` runs before `create_partial`. If
        `create_new` fails, the job deletes another process's file.
    *   **Fix:** delete-on-close (Z3) removes the problem. Otherwise, publish
        the path only after `create_partial` succeeds.
*   **R8. Overflow in the sum of declared sizes.** *(readiness M4)*
    *   **Fix:** use `checked_add`. On overflow, report `NotEnoughSpace`
        with `u64::MAX` and set `bytes_total = None`. Add a test.
*   **R13. Content length from an `HGLOBAL`.** *(readiness M3c, details M3)*
    *   **Problem:** `GlobalSize` can be rounded up, so reading it as the
        content length adds extra bytes.
    *   **Fix:** read `min(GlobalSize, declared)` bytes; read `GlobalSize`
        bytes when there is no declared size.
*   **R14. Converting names from UTF-16.** *(details M12)*
    *   **Fix:** use `String::from_utf16` without lossy conversion. An
        error becomes a `BadName` ("invalid UTF-16"). Add a test.
*   **R15. A file shorter than declared is accepted.** *(readiness M6,
    details M9, style M6)*
    *   **Fix:** add `SizeShort`: remove the `.partial` and fail. Add a
        test and one sentence in the specification.
*   **R16. Conflicts discovered only during writing.** *(readiness M5,
    details M10)*
    *   **Problem:**
        *   implied parent folders (`x\y` with no `x` entry, while a file
            `x` exists or `x` is listed as a file);
        *   `a` and `a.partial` in the same list.
    *   **Fix:** `check` treats every ancestor as an implied `Dir` and
        treats `.partial` names as keys. Add tests.
*   **R17. `rename_no_replace` and `free_bytes` without `\\?\`.**
    *(readiness M11, details M5)*
    *   **Problem:** the long-path test will fail for certain, because these
        call `MoveFileExW` and `GetDiskFreeSpaceExW` directly.
    *   **Fix:** a `long_path()` helper, applied unconditionally:
        `std::path::absolute`, then `\\?\` (or `\\?\UNC\`). Map both 183
        and 80 to `TargetExists`.

### Logic and contracts

*   **R18. The unbuffered write loop contradicts itself.** *(readiness M7,
    details M8, style M5)*
    *   **Fix:** use one explicit loop:
        *   keep a `filled` counter;
        *   end of stream is `read == 0` (`S_FALSE` from `IStream::Read` is
            success);
        *   write only multiples of `ALIGN`, except the final write;
        *   zero the padding;
        *   use `write_all`;
        *   call `set_len(written)` at the end.
*   **R19. How a job reports `Cancelled` is undefined.** *(readiness M9,
    details M7, style M1)*
    *   **Fix:**
        *   add `JobError::Cancelled`, which the job returns at every cancel
            check;
        *   `wait` maps it to `Outcome::Cancelled`, and maps a panic in the
            thread to `Failed`;
        *   an `Ok` that arrives after a late cancel is `Done`;
        *   `wait(self, …)`.
*   **R20. The 1 s test timeout cannot be set, and the timing assertions
    cannot pass reliably.** *(readiness M10, details M6, style M2/M3/M4)*
    *   **Fix:**
        *   `Limits { stall, cancel_grace }` as an argument, with `Default`
            set to the production values;
        *   store the moment of cancel in `CancelToken`;
        *   the stall clock starts when `wait` is called and resets on every
            update;
        *   assertions take the form "≥ X and < X + 500 ms".
*   **R9. `DropRefused`: which call decides.** *(style M10, details N12)*
    Removed if Z1 is accepted.
*   **R10. Publishing the `.partial` path.** Removed if Z3 is accepted.
*   **R11. The `--json-progress` contract is incomplete.** *(readiness M13,
    details M11, style M8/M9)*
    *   **Problem:** it does not say that logs go to stderr, what the final
        object contains, how strings are escaped, or how the GUI sends
        cancel.
    *   **Fix:** removed if Z2 is accepted.
*   **R12. Terminal logs on stdout.** If Z2 is not accepted:
    `.with_writer(stderr)`.
*   **R21. The test fake is under-specified.** *(readiness M12, details M13)*
    *   **Problem:** the plan does not define:
        *   per-entry control of `FD_*` flags;
        *   a mode without descriptors;
        *   a raw descriptor (bad `cItems`);
        *   a `release` field on `Faulty`;
        *   that `FakeFiles` is `Send` and is built on the job thread;
        *   `EnumFormatEtc` through `SHCreateStdEnumFmtEtc`;
        *   the full set of `IStream` and `ISequentialStream` methods
            (unused ones return `E_NOTIMPL`).
    *   **Fix:** specify the builder API explicitly.
*   **R22. Undefined counters and units.** *(style M7/M11)*
    *   **Problem:** `SPACE_CHECK_EVERY` has no unit, and `files_total` and
        `folders_created` are not defined. If stage 8 nit 4 is accepted,
        `SPACE_CHECK_EVERY` disappears.
    *   **Fix:** define `files_total`, counting files only.

## Nits (grouped)

*   **Signatures:**
    *   `from_data_object(sta, …)` is missing the `sta` argument;
    *   `name: &str`;
    *   the return type of `paste_from`;
    *   `enum Problem` needs its name;
    *   where `kind_of_path` lives (`symlink_metadata` in `paste.rs`).
*   **Units:** UTF-16 units, not characters; GiB versus GB; `LARGE_FILE`
    also covers files of unknown size.
*   **Exit codes:** exit codes for `--formats`; the mapping
    `NoVirtualFiles` → 4; `std::process::exit` on the cancel and stall
    paths.
*   **Win32 details:**
    *   `lpFreeBytesAvailableToCaller`, with a trailing `\` for UNC paths;
    *   `ALIGN` versus a sector size above 4096;
    *   FILETIME→`SystemTime` conversion with checked arithmetic;
    *   trim trailing spaces and dots before checking reserved names
        (`CON .txt`).
*   **Case comparison:** compare keys with `to_uppercase`, which is closer
    to NTFS, and add a `shortcut:` comment.
*   **Log:**
    *   the INFO level;
    *   `create_dir_all` for the log folder;
    *   if the log file cannot be opened, the program falls back to the
        terminal only;
    *   a warning with the path when deleting the `.partial` fails (R5 in
        the specification).
*   **CI:** add `rustup show` before cargo; `typeperf -f CSV`, with the
    counter list given explicitly.
*   **Documents:**
    *   moving the 5 reports becomes its own plan item;
    *   the `CLAUDE.md` additions are written in Polish;
    *   the `TargetExists` race test opens its window from `on_progress`;
    *   "checked by eye" becomes a test;
    *   remove the history sentence in item 6.

## Verified as correct (codebase-details)

*   `FD_FILESIZE`, `FD_WRITESTIME`, `FD_ATTRIBUTES`, and the
    `FILEDESCRIPTORW` fields;
*   `OleGetClipboard`, and `OleSetClipboard(None)` to clear;
*   `MoveFileExW` with no flags does not overwrite;
*   `THREAD_MODE_BACKGROUND_BEGIN/END`;
*   `custom_flags(FILE_FLAG_NO_BUFFERING.0)` together with `create_new`;
*   std opens files with `FILE_SHARE_DELETE`;
*   `set_len` after padded unbuffered writes;
*   `BHID_SFUIObject`;
*   the Performance Monitor counter names;
*   `File::set_modified` is available from Rust 1.75;
*   the lint inheritance rule in Cargo.

## Elephant assessment (main context)

*   **Recommendation:** resolve stages 8 and 9 in one pass, applying Z1–Z3
    first and then R1–R22. This shrinks the plan before it is made more
    precise, so no time is spent specifying `--json-progress` or
    `DropRefused` that will be deleted anyway.
*   **Most serious findings:**
    *   R5: an out-of-bounds read in `unsafe` code, driven by the remote
        side;
    *   R6: undefined behaviour in `AlignedBuf`;
    *   R7: deleting another process's file;
    *   R17: a test that will certainly fail.

    All four have local, cheap fixes.
*   **Decision needed from you:** stage 8 nit 7. Either measure buffered
    writes before writing the unbuffered code (an extra first step in M0),
    or keep `NO_BUFFERING` as decided in stage 7.

---

# Round 2 (re-audit after resolution) — 2026-11-11

*   **Resolution:** S1–S14 and the nits were applied to the design on 2026-11-11 (owner choice: option 2, no further audit).
*   **Status:** all three reviewers completed (readiness, codebase-details,
    style).
*   **Round 1:** R1, R3–R8 and R13–R22 are resolved. R9–R12 were removed
    together with Z1/Z2. R2 is only partly resolved, because the windows-rs
    version is still not pinned.
*   **Verdict:** NOT READY, with no blockers. All round-2 findings are
    small, local fixes. The audit cap has been reached (one re-audit), so
    there will be no further rounds.

## Round 2 findings (deduplicated)

| ID | Problem | Fix | Raised by |
| :-- | :-- | :-- | :-- |
| S1 | `lindex` is ambiguous: "for (i, entry) in File entries" gives the wrong file's contents after the first folder | `for (i, e) in entries().iter().enumerate()`, skipping `Dir`. `i` is the descriptor index. The fake answers `lindex` in builder order, and the test interleaves folders and files | all 3 |
| S2 | A failed rename leaves a complete `.partial` behind | Keep the handle open during the rename: clear the flag, rename, and on error set the flag again before returning. Drop the handle afterwards. The race test checks that no `.partial` remains | details, style |
| S3 | The `Duplicate` rule flags implied folders against each other (`x\a` with `x\b`) | A duplicate exists only when at least one side is a file or a `.partial` path. Folder against folder merges. Add a test for `x\a` plus `x\b`, and update spec line 489 | readiness, style |
| S4 | When sizes are unknown, `need` ignores the known sizes, which contradicts the spec | `need` = checked sum of the known sizes + `DISK_RESERVE`. Reported when `free < need`. Add a test | readiness, style |
| S5 | The windows-rs version is still not pinned, and the `implement` feature may not exist in later versions | Pin `windows = "=0.58.0"` and `windows-core = "=0.58.0"` | all 3 |
| S6 | In the "cancel while hung" test, the stall fires before the cancel grace | Once `cancelled_at()` is `Some`, skip the stall check | details |
| S7 | Cancel takes 1.0–1.1 s, while G1 says "within 1 s" | `CANCEL_GRACE = 900 ms` | readiness, details |
| S8 | After cancel, stall, or failure the CLI does not know how many files completed (CUJ 2) | After each rename, use a blocking `send`. The CLI keeps the last `Progress` and prints `files_done/files_total`. Log one INFO event per finished file and per outcome | readiness |
| S9 | Errors do not name the entry, although the spec requires it | `JobError::Entry { path, source }` wraps errors from open, read and write. Add a test | readiness |
| S10 | It is not stated which COM call means "no virtual files" | `QueryGetData(FILEDESCRIPTORW, TYMED_HGLOBAL)`: failure means `NoVirtualFiles`. The fake implements `QueryGetData`. Advise methods return `OLE_E_ADVISENOTSUPPORTED`; the rest return `E_NOTIMPL` | readiness, details |
| S11 | Tests need temporary folders, a hash, and a generator, but no crates are allowed for them | Use `temp_dir()/files-test-<name>-<pid>`. `Generated` is xorshift64 from a seed. Compare by regenerating the stream instead of hashing | details, readiness |
| S12 | `Faulty.delay` is undefined | It is applied before each `Read`. Tests use 50 ms (300 ms for the race test). `Faulty` applies only to `Stream`. `hang_after` without `release` is allowed only in the stall test | style, readiness |
| S13 | `long_path(dir) + "\"` doubles the backslash for `D:\` | Add `\` only if the path does not already end with one | details |
| S14 | Reserved names: the order of trimming and splitting allows `CON .txt` | Take the part before the first dot, trim trailing spaces from it, then compare | style |

**Nits:**

*   `GENERIC_WRITE.0 | DELETE.0`, and `(TYMED_ISTREAM.0 | TYMED_HGLOBAL.0) as u32`, because these are different newtypes.
*   Compare error codes with `e.code() == ERROR_ALREADY_EXISTS.to_hresult()`.
*   `CancelToken(Arc<OnceLock<Instant>>)`, without the `AtomicBool`.
*   `--help` exits with 0.
*   `From` impls for `JobError`.
*   "except two" should say three.
*   Explain how `FakeFile` is constructed and what `raw_name` creates.
*   Tests wait for the `.partial` to disappear by polling for about 2 s.
*   Add tests for `BadDescriptor` short/too many and for `Panicked`.
*   Spec lines 478 and 501 should say "UTF-16 units".
*   Fix the reason given for `Win32_UI_Shell`.
*   R5 points to item 24, not 23.
*   The edition is justified only by Rust 1.79.
*   Pin versions when each dependency is added.
