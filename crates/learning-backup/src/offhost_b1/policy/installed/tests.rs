use super::*;
use crate::registry::MAX_SCAN_BYTES;
use std::{
    fs::{self, DirBuilder, OpenOptions},
    io::{Cursor, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, symlink},
    path::PathBuf,
};

fn budget() -> Budget {
    Arc::new(Mutex::new(ScanBudget::default()))
}

// Synthetic metadata predicate input only, not an installed File/Dir or actor.
fn root_stamp() -> Stamp {
    Stamp {
        dev: 1,
        ino: 2,
        uid: 0,
        gid: 0,
        mode: libc::S_IFREG | 0o600,
        links: 1,
        len: 4,
        mtime: 10,
        mtime_nsec: 20,
        ctime: 30,
        ctime_nsec: 40,
    }
}

#[test]
fn private_record_predicate_rejects_wrong_type_owner_mode_links_and_size() {
    root_stamp().require_private_record(4).unwrap();
    for n in 0..7 {
        let mut changed = root_stamp();
        match n {
            0 => changed.mode = libc::S_IFIFO | 0o600,
            1 => changed.uid = 1000,
            2 => changed.mode |= 0o040,
            3 => changed.mode |= 0o4000,
            4 => changed.links = 2,
            5 => changed.len = 0,
            6 => changed.len = 5,
            _ => unreachable!(),
        }
        assert!(changed.require_private_record(4).is_err());
    }
}

struct NeverRead;
impl Read for NeverRead {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        panic!("rejected size or exhausted budget must not touch reader")
    }
}

#[test]
fn declaration_cap_plus_one_and_budget_fail_before_read_or_allocation() {
    let budget = budget();
    assert!(read_counted(&mut NeverRead, 5, 4, &budget).is_err());
    assert_eq!(budget.lock().unwrap().bytes, 0);
    budget.lock().unwrap().bytes = MAX_SCAN_BYTES - 3;
    assert!(read_counted(&mut NeverRead, 4, 4, &budget).is_err());
    assert_eq!(budget.lock().unwrap().bytes, MAX_SCAN_BYTES - 3);
}

#[test]
fn exact_cap_requires_real_metadata_eof_and_rejects_suffix_or_truncation() {
    let budget = budget();
    assert_eq!(
        read_counted(&mut Cursor::new(b"data"), 4, 4, &budget).unwrap(),
        b"data"
    );
    assert_eq!(budget.lock().unwrap().bytes, 5);
    assert!(read_counted(&mut Cursor::new(b"data!"), 4, 4, &budget).is_err());
    assert_eq!(budget.lock().unwrap().bytes, 10);
    assert!(read_counted(&mut Cursor::new(b"dat"), 4, 4, &budget).is_err());
    assert_eq!(budget.lock().unwrap().bytes, 14);
}

#[test]
fn repeated_reads_and_eof_reservation_share_the_original_budget() {
    let original = budget();
    let clone = Arc::clone(&original);
    for counter in [&original, &clone] {
        read_counted(&mut Cursor::new(b"data"), 4, 4, counter).unwrap();
    }
    assert_eq!(original.lock().unwrap().bytes, 10);
    original.lock().unwrap().bytes = MAX_SCAN_BYTES - 4;
    assert!(read_counted(&mut Cursor::new(b"data"), 4, 4, &original).is_err());
    assert_eq!(clone.lock().unwrap().bytes, MAX_SCAN_BYTES);
}

struct Fixture {
    path: PathBuf,
    dir: BackupDir,
}

impl Fixture {
    fn new() -> Self {
        // Real ordinary-user rejection fixtures only: never a Root positive.
        assert_ne!(
            unsafe { libc::geteuid() },
            0,
            "fixture requires ordinary UID"
        );
        let path = std::env::temp_dir().join(format!("kw-b1-metadata-{}", uuid::Uuid::new_v4()));
        DirBuilder::new().mode(0o700).create(&path).unwrap();
        let dir = BackupDir::open_private_root(&path).unwrap();
        Self { path, dir }
    }

    fn write(&self, name: &str, value: &[u8]) {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.path.join(name))
            .unwrap();
        file.write_all(value).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for name in [
            "metadata.json",
            "original.json",
            "replacement.json",
            "link",
            "pipe",
        ] {
            let _ = fs::remove_file(self.path.join(name));
        }
        let _ = fs::remove_dir(&self.path);
    }
}

#[test]
fn ordinary_private_file_and_directory_are_not_root_custody() {
    let fixture = Fixture::new();
    fixture.write("metadata.json", b"data");
    assert!(BackupDir::open_trusted_private_root(&fixture.path).is_err());
    let budget = budget();
    let mut file = fixture.dir.open_file("metadata.json").unwrap();
    assert!(read_checked(&mut file, 4, &budget).is_err());
    assert_eq!(budget.lock().unwrap().bytes, 0);
}

#[test]
fn actual_named_replacement_has_different_identity_with_equal_bytes() {
    let fixture = Fixture::new();
    fixture.write("metadata.json", b"data");
    fixture.write("replacement.json", b"data");
    let held = fixture.dir.open_file("metadata.json").unwrap();
    let old = Stamp::from_metadata(&held.metadata().unwrap());
    fs::rename(
        fixture.path.join("metadata.json"),
        fixture.path.join("original.json"),
    )
    .unwrap();
    fs::rename(
        fixture.path.join("replacement.json"),
        fixture.path.join("metadata.json"),
    )
    .unwrap();
    let named = fixture.dir.open_file("metadata.json").unwrap();
    let now = Stamp::from_metadata(&named.metadata().unwrap());
    assert!((old.dev, old.ino) != (now.dev, now.ino));
    assert_eq!(old.len, now.len);
    // These are ordinary UID metadata observations, not a successful loader.
    assert!(old.require_private_record(4).is_err());
    assert!(now.require_private_record(4).is_err());
}

#[test]
fn actual_hardlink_symlink_and_fifo_refuse_without_blocking() {
    let fixture = Fixture::new();
    fixture.write("metadata.json", b"data");
    fs::hard_link(
        fixture.path.join("metadata.json"),
        fixture.path.join("link"),
    )
    .unwrap();
    assert!(fixture.dir.open_file("metadata.json").is_err());
    fs::remove_file(fixture.path.join("link")).unwrap();
    symlink("metadata.json", fixture.path.join("link")).unwrap();
    assert!(fixture.dir.open_file("link").is_err());
    let fifo =
        std::ffi::CString::new(fixture.path.join("pipe").as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert!(fixture.dir.open_file("pipe").is_err());
}

#[test]
fn same_length_metadata_change_is_visible_in_full_stamp() {
    let fixture = Fixture::new();
    fixture.write("metadata.json", b"data");
    let held = fixture.dir.open_file("metadata.json").unwrap();
    let old = Stamp::from_metadata(&held.metadata().unwrap());
    // A mode change preserves inode and size but must fail a complete recheck.
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(
        fixture.path.join("metadata.json"),
        fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    let changed = Stamp::from_metadata(&held.metadata().unwrap());
    assert_eq!(
        (old.dev, old.ino, old.len),
        (changed.dev, changed.ino, changed.len)
    );
    assert!(old != changed);
    assert!(changed.require_private_record(4).is_err());
}
