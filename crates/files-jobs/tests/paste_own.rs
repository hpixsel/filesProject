//! End-to-end paste tests with the fake data object passed directly.

use files_core::ConflictPolicy::{self, Fail, Overwrite};
use files_core::Problem;
use files_jobs::{paste_from, start, JobError, Limits, Outcome, Summary};
use files_win::fake::{generated, DeclaredSize, FakeContent, FakeFile, FakeFiles};
use files_win::{Error, VirtualFiles};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

const LIMITS: Limits = Limits {
    stall: Duration::from_secs(1),
    cancel_grace: Duration::from_millis(900),
};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("files-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(dir: &Path, policy: ConflictPolicy, f: FakeFiles) -> Outcome<Summary> {
    paste_from(dir.to_path_buf(), policy, LIMITS, move |sta| {
        VirtualFiles::from_data_object(sta, f.into_data_object())
    })
    .wait(|_| {})
}

fn bytes(b: &[u8]) -> FakeFile {
    FakeFile::new(FakeContent::Bytes(b.to_vec()))
}

fn faulty(
    inner: FakeContent,
    delay_ms: u64,
    fail: Option<u64>,
    hang: Option<u64>,
    release: Option<Arc<AtomicBool>>,
) -> FakeFile {
    FakeFile::new(FakeContent::Faulty {
        inner: Box::new(inner),
        delay: Duration::from_millis(delay_ms),
        fail_after: fail,
        hang_after: hang,
        release,
    })
}

fn no_partials(dir: &Path) -> bool {
    walk(dir)
        .iter()
        .all(|p| p.extension().is_none_or(|e| e != "partial"))
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        }
        out.push(p);
    }
    out
}

/// After an abandoned job, wait for its thread to drop the handle.
fn eventually_no_partials(dir: &Path) -> bool {
    let end = Instant::now() + Duration::from_secs(2);
    while Instant::now() < end {
        if no_partials(dir) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn failed(o: Outcome<Summary>) -> JobError {
    match o {
        Outcome::Failed(e) => e,
        Outcome::Done(s) => panic!("unexpected success: {s:?}"),
        Outcome::Cancelled => panic!("unexpected cancel"),
        Outcome::Stalled => panic!("unexpected stall"),
    }
}

fn problems(o: Outcome<Summary>) -> Vec<Problem> {
    match failed(o) {
        JobError::Problems(p) => p,
        other => panic!("{other}"),
    }
}

#[test]
fn nested_tree_with_interleaved_folders() {
    let d = tmp("tree");
    let t = UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    let o = run(
        &d,
        Fail,
        FakeFiles::new()
            .file("late\\x.txt", bytes(b"before its folder"))
            .dir("a")
            .file("a\\1.txt", bytes(b"one").modified(t))
            .dir("a\\b")
            .file("a\\b\\2.txt", bytes(b"two"))
            .dir("late"),
    );
    let Outcome::Done(s) = o else { panic!("{o:?}") };
    assert_eq!(
        s,
        Summary {
            files_written: 3,
            folders_created: 3,
            bytes: 3 + 3 + 17
        }
    );
    assert_eq!(std::fs::read(d.join("a\\1.txt")).unwrap(), b"one");
    assert_eq!(std::fs::read(d.join("a\\b\\2.txt")).unwrap(), b"two");
    assert_eq!(
        std::fs::read(d.join("late\\x.txt")).unwrap(),
        b"before its folder"
    );
    let m = std::fs::metadata(d.join("a\\1.txt"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(m, t);
    assert!(no_partials(&d));
}

#[test]
fn large_and_unknown_size_files_are_exact() {
    let d = tmp("large");
    let len = 300 * (1 << 20) + 123; // above LARGE_FILE, not aligned
    let mut progress_total = None;
    let o = paste_from(d.clone(), Fail, LIMITS, move |sta| {
        let f = FakeFiles::new()
            .file(
                "big",
                FakeFile::new(FakeContent::Generated { len, seed: 3 }),
            )
            .file("unknown", bytes(b"no size given").size(DeclaredSize::None));
        VirtualFiles::from_data_object(sta, f.into_data_object())
    })
    .wait(|p| progress_total = Some(p.bytes_total));
    assert!(matches!(o, Outcome::Done(_)), "{o:?}");
    assert_eq!(
        progress_total,
        Some(None),
        "an unknown size makes the total unknown"
    );

    let got = std::fs::read(d.join("big")).unwrap();
    let mut want = vec![0u8; len as usize];
    generated(3, 0, &mut want);
    assert!(got == want, "content of the large file differs");
    assert_eq!(std::fs::read(d.join("unknown")).unwrap(), b"no size given");
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn long_paths_under_both_policies() {
    // Descriptor names are limited to 260 units (risk R3), so the long part
    // is the target folder; the final path is well over 300 characters.
    for policy in [Fail, Overwrite] {
        let d = tmp(&format!("long-{policy:?}"))
            .join("t".repeat(120))
            .join("u".repeat(120));
        std::fs::create_dir_all(&d).unwrap();
        let name = format!("{}\\file.txt", "x".repeat(100));
        let o = run(&d, policy, FakeFiles::new().file(&name, bytes(b"deep")));
        assert!(matches!(o, Outcome::Done(_)), "{policy:?}: {o:?}");
        let full = d.join(&name);
        assert!(full.as_os_str().len() > 300);
        assert_eq!(std::fs::read(full).unwrap(), b"deep");
    }
}

#[test]
fn conflicts() {
    let d = tmp("conflicts");
    std::fs::write(d.join("a"), "old a").unwrap();
    std::fs::write(d.join("b"), "old b").unwrap();
    std::fs::create_dir(d.join("dir")).unwrap();
    let both = || {
        FakeFiles::new()
            .file("a", bytes(b"new a"))
            .file("b", bytes(b"new b"))
    };

    assert_eq!(problems(run(&d, Fail, both())).len(), 2);
    assert_eq!(
        std::fs::read(d.join("a")).unwrap(),
        b"old a",
        "nothing written"
    );

    assert!(matches!(run(&d, Overwrite, both()), Outcome::Done(_)));
    assert_eq!(std::fs::read(d.join("a")).unwrap(), b"new a");

    for policy in [Fail, Overwrite] {
        let p = problems(run(&d, policy, FakeFiles::new().file("dir", bytes(b"x"))));
        assert!(
            matches!(
                &p[..],
                [Problem::Conflict {
                    overwritable: false,
                    ..
                }]
            ),
            "{p:?}"
        );
    }

    std::fs::write(d.join("c.partial"), "someone else's").unwrap();
    let p = problems(run(&d, Fail, FakeFiles::new().file("c", bytes(b"x"))));
    assert!(matches!(&p[..], [Problem::PartialExists(_)]), "{p:?}");
    assert_eq!(
        std::fs::read(d.join("c.partial")).unwrap(),
        b"someone else's"
    );

    let p = problems(run(
        &d,
        Fail,
        FakeFiles::new()
            .file("A.txt", bytes(b""))
            .file("a.txt", bytes(b"")),
    ));
    assert!(
        p.iter().any(|x| matches!(x, Problem::Duplicate(_))),
        "{p:?}"
    );
    let p = problems(run(
        &d,
        Fail,
        FakeFiles::new()
            .file("x", bytes(b""))
            .file("x\\y", bytes(b"")),
    ));
    assert!(
        p.iter().any(|x| matches!(x, Problem::Duplicate(_))),
        "{p:?}"
    );
}

#[test]
fn target_created_after_the_check_is_not_overwritten() {
    let d = tmp("race");
    let target = d.join("a");
    let slow = faulty(FakeContent::Bytes(b"new".to_vec()), 300, None, None, None);
    let mut created = false;
    let o = paste_from(d.clone(), Fail, LIMITS, move |sta| {
        VirtualFiles::from_data_object(sta, FakeFiles::new().file("a", slow).into_data_object())
    })
    .wait(|_| {
        if !created {
            std::fs::write(&target, "someone else's").unwrap();
            created = true;
        }
    });
    match failed(o) {
        JobError::Entry { source, .. } => {
            assert!(
                matches!(*source, JobError::Win(Error::TargetExists(_))),
                "{source}"
            )
        }
        other => panic!("{other}"),
    }
    assert_eq!(std::fs::read(d.join("a")).unwrap(), b"someone else's");
    assert!(no_partials(&d));
}

#[test]
fn size_checks() {
    let d = tmp("sizes");
    let e = failed(run(
        &d,
        Fail,
        FakeFiles::new().file("long", bytes(&[1; 20]).size(DeclaredSize::Value(10))),
    ));
    assert!(
        matches!(e, JobError::Entry { ref source, .. } if matches!(**source, JobError::SizeExceeded(_))),
        "{e}"
    );
    let e = failed(run(
        &d,
        Fail,
        FakeFiles::new().file("short", bytes(&[1; 10]).size(DeclaredSize::Value(20))),
    ));
    assert!(
        matches!(e, JobError::Entry { ref source, .. } if matches!(**source, JobError::SizeShort { got: 10, .. })),
        "{e}"
    );
    let p = problems(run(
        &d,
        Fail,
        FakeFiles::new().file("huge", bytes(b"").size(DeclaredSize::Value(u64::MAX / 4))),
    ));
    assert!(matches!(&p[..], [Problem::NotEnoughSpace { .. }]), "{p:?}");
    assert!(no_partials(&d));
}

#[test]
fn cancel_during_slow_reads() {
    let d = tmp("cancel-slow");
    let slow = faulty(
        FakeContent::Generated {
            len: 20 << 20,
            seed: 1,
        },
        50,
        None,
        None,
        None,
    );
    let h = paste_from(d.clone(), Fail, LIMITS, move |sta| {
        let f = FakeFiles::new()
            .file("done.txt", bytes(b"finished"))
            .file("slow", slow);
        VirtualFiles::from_data_object(sta, f.into_data_object())
    });
    let tok = h.cancel_token();
    let started = Instant::now();
    let o = h.wait(|p| {
        if p.files_done == 1 {
            tok.cancel()
        }
    });
    assert!(matches!(o, Outcome::Cancelled), "{o:?}");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(std::fs::read(d.join("done.txt")).unwrap(), b"finished");
    assert!(eventually_no_partials(&d));
}

#[test]
fn cancel_while_a_read_hangs() {
    let d = tmp("cancel-hang");
    let release = Arc::new(AtomicBool::new(false));
    // Hang on the very first read, so the job is surely blocked (not merely
    // between reads, where it would see cancel at once) when we cancel.
    let hang = faulty(
        FakeContent::Bytes(vec![1; 100]),
        0,
        None,
        Some(0),
        Some(release.clone()),
    );
    let h = paste_from(d.clone(), Fail, LIMITS, move |sta| {
        VirtualFiles::from_data_object(sta, FakeFiles::new().file("h", hang).into_data_object())
    });
    // The .partial exists right before the first read starts.
    let partial = d.join("h.partial");
    let until = Instant::now() + Duration::from_secs(5);
    while !partial.exists() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(100));
    let tok = h.cancel_token();
    tok.cancel();
    let asked = Instant::now();
    let o = h.wait(|_| {});
    let took = asked.elapsed();
    assert!(matches!(o, Outcome::Cancelled), "{o:?}");
    assert!(
        took >= Duration::from_millis(900) && took < Duration::from_millis(1400),
        "{took:?}"
    );
    release.store(true, Ordering::SeqCst);
    assert!(eventually_no_partials(&d));
}

#[test]
fn stall_without_cancel() {
    let d = tmp("stall");
    let release = Arc::new(AtomicBool::new(false));
    let hang = faulty(
        FakeContent::Bytes(vec![1; 100]),
        0,
        None,
        Some(10),
        Some(release.clone()),
    );
    let h = paste_from(d.clone(), Fail, LIMITS, move |sta| {
        VirtualFiles::from_data_object(sta, FakeFiles::new().file("h", hang).into_data_object())
    });
    let mut last = Instant::now();
    let o = h.wait(|_| last = Instant::now());
    let took = last.elapsed();
    assert!(matches!(o, Outcome::Stalled), "{o:?}");
    assert!(
        took >= Duration::from_secs(1) && took < Duration::from_millis(1500),
        "{took:?}"
    );
    release.store(true, Ordering::SeqCst);
    assert!(eventually_no_partials(&d));
}

#[test]
fn read_failure_names_the_file() {
    let d = tmp("fail");
    let bad = faulty(FakeContent::Bytes(vec![1; 100]), 0, Some(10), None, None);
    match failed(run(&d, Fail, FakeFiles::new().file("broken.bin", bad))) {
        JobError::Entry { path, .. } => assert_eq!(path.to_string(), "broken.bin"),
        other => panic!("{other}"),
    }
    assert!(no_partials(&d));
}

#[test]
fn panic_in_a_job_is_reported() {
    let h = start("panics", LIMITS, |_ctx| -> Result<(), JobError> {
        panic!("boom")
    });
    assert!(matches!(
        h.wait(|_| {}),
        Outcome::Failed(JobError::Panicked)
    ));
}

#[test]
#[ignore = "writes 5 GiB; run by hand"]
fn five_gib_file() {
    let d = tmp("5gib");
    let len = 5u64 << 30;
    let o = run(
        &d,
        Fail,
        FakeFiles::new().file(
            "big",
            FakeFile::new(FakeContent::Generated { len, seed: 9 }),
        ),
    );
    assert!(matches!(o, Outcome::Done(_)), "{o:?}");
    let mut f = std::fs::File::open(d.join("big")).unwrap();
    let (mut pos, mut got, mut want) = (0u64, vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
    use std::io::Read;
    loop {
        let n = f.read(&mut got).unwrap();
        if n == 0 {
            break;
        }
        generated(9, pos, &mut want[..n]);
        assert!(got[..n] == want[..n], "differs at {pos}");
        pos += n as u64;
    }
    assert_eq!(pos, len);
    std::fs::remove_dir_all(d).unwrap();
}
