//! Reading virtual files from the fake data object, passed directly (no
//! system clipboard), so these tests can run in parallel.

use files_core::EntryKind;
use files_win::fake::{generated, DeclaredSize, FakeContent, FakeFile, FakeFiles};
use files_win::{Error, Sta, VirtualFiles};
use std::io::Read;
use std::time::{Duration, UNIX_EPOCH};

fn load(f: FakeFiles) -> (Sta, files_win::Result<VirtualFiles>) {
    let sta = Sta::new().unwrap();
    let obj = f.into_data_object();
    let vf = VirtualFiles::from_data_object(&sta, obj);
    (sta, vf)
}

fn bytes(b: &[u8]) -> FakeFile {
    FakeFile::new(FakeContent::Bytes(b.to_vec()))
}

#[test]
fn parses_names_kinds_sizes_and_times() {
    let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let (_sta, vf) = load(
        FakeFiles::new()
            .dir("d")
            .file("d\\a.txt", bytes(b"abc").modified(t))
            .file(
                "big",
                FakeFile::new(FakeContent::Bytes(vec![])).size(DeclaredSize::Value(5 << 30)),
            )
            .file("unknown", bytes(b"x").size(DeclaredSize::None))
            .file("noattr", bytes(b"y").no_attributes()),
    );
    let vf = vf.unwrap();
    let e = vf.entries();
    assert_eq!(e.len(), 5);
    assert_eq!(e[0].kind, EntryKind::Dir);
    assert_eq!(e[1].rel_path.to_string(), "d\\a.txt");
    assert_eq!(e[1].kind, EntryKind::File { size: Some(3) });
    assert_eq!(e[1].modified, Some(t));
    assert_eq!(
        e[2].kind,
        EntryKind::File {
            size: Some(5 << 30)
        }
    );
    assert_eq!(e[3].kind, EntryKind::File { size: None });
    assert_eq!(e[4].kind, EntryKind::File { size: Some(1) });
}

#[test]
fn reports_all_bad_names() {
    let (_sta, vf) = load(
        FakeFiles::new()
            .file("a\\..\\b", bytes(b""))
            .file("CON", bytes(b"")),
    );
    match vf {
        Err(Error::BadNames(n)) => assert_eq!(n.len(), 2),
        other => panic!("{:?}", other.err()),
    }
}

#[test]
fn name_without_nul_and_invalid_utf16() {
    let (_sta, vf) = load(FakeFiles::new().raw_name([b'a' as u16; 260]));
    let Err(Error::BadNames(n)) = vf else {
        panic!()
    };
    assert_eq!(n[0].reason, "descriptor path limit");

    let mut units = [0u16; 260];
    units[0] = 0xD800; // unpaired surrogate
    let (_sta, vf) = load(FakeFiles::new().raw_name(units));
    let Err(Error::BadNames(n)) = vf else {
        panic!()
    };
    assert_eq!(n[0].reason, "invalid UTF-16");
}

#[test]
fn bad_descriptor_blocks() {
    let reason = |f: FakeFiles| match load(f).1 {
        Err(Error::BadDescriptor(r)) => r,
        other => panic!("{:?}", other.err()),
    };
    assert_eq!(
        reason(FakeFiles::new().file("a", bytes(b"")).raw_count(5)),
        "count exceeds block"
    );
    assert_eq!(
        reason(FakeFiles::new().raw_count(2_000_000)),
        "too many entries"
    );
    assert_eq!(reason(FakeFiles::new().short_block()), "short block");
}

#[test]
fn no_virtual_files_lists_formats() {
    let (_sta, vf) = load(FakeFiles::new().without_descriptors().extra_format(13));
    match vf {
        Err(Error::NoVirtualFiles { formats }) => assert_eq!(formats, ["CF_UNICODETEXT"]),
        other => panic!("{:?}", other.err()),
    }
}

#[test]
fn open_reads_both_media_by_descriptor_index() {
    let gen = FakeContent::Generated {
        len: 10_000,
        seed: 7,
    };
    let (_sta, vf) = load(
        FakeFiles::new()
            .dir("d")
            .file("d\\s", FakeFile::new(gen))
            .dir("e")
            .file("e\\m", bytes(b"memory!").memory()),
    );
    let vf = vf.unwrap();

    let mut got = Vec::new();
    vf.open(1).unwrap().read_to_end(&mut got).unwrap();
    let mut want = vec![0u8; 10_000];
    generated(7, 0, &mut want);
    assert_eq!(got, want);

    got.clear();
    vf.open(3).unwrap().read_to_end(&mut got).unwrap();
    assert_eq!(got, b"memory!");

    assert!(vf.open(0).is_err(), "folders have no contents");
}

#[test]
fn hglobal_yields_exactly_the_declared_size() {
    let (_sta, vf) = load(FakeFiles::new().file(
        "m",
        bytes(b"0123456789").memory().size(DeclaredSize::Value(4)),
    ));
    let mut got = Vec::new();
    vf.unwrap().open(0).unwrap().read_to_end(&mut got).unwrap();
    assert_eq!(got, b"0123");
}

#[test]
fn faulty_stream_fails_after_limit() {
    let f = FakeContent::Faulty {
        inner: Box::new(FakeContent::Bytes(vec![1; 100])),
        delay: Duration::ZERO,
        fail_after: Some(40),
        hang_after: None,
        release: None,
    };
    let (_sta, vf) = load(FakeFiles::new().file("f", FakeFile::new(f)));
    let mut r = vf.unwrap().open(0).unwrap();
    let mut buf = [0u8; 64];
    assert_eq!(r.read(&mut buf).unwrap(), 40);
    assert!(r.read(&mut buf).is_err());
}
