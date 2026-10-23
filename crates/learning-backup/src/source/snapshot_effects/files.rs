//! All adjunct I/O is relative to retained no-follow handles. A failure leaves
//! its unique attempt in place; no retry adopts or removes those entries.
use super::wire::{MAX_FILE, MAX_FILES, hash, invalid, path};
use crate::{BackupError, FileRecord, registry::ScanBudget};
use learning_assets::{
    backup_fs::{BackupDir, BackupEntryKind},
    root_journal::SharedJournalBudget,
};
use serde::Serialize;
use std::os::unix::fs::MetadataExt;
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    sync::{Arc, Mutex},
};

pub(crate) type Budget = Arc<Mutex<ScanBudget>>;

pub(crate) fn charge(
    budget: &Budget,
    journal: &SharedJournalBudget,
    bytes: u64,
    entries: usize,
) -> Result<(), BackupError> {
    let mut meter = budget.lock().map_err(|_| invalid())?;
    if let Err(error) = meter.charge(bytes).and_then(|()| meter.entries(entries)) {
        // The older registry meter does not itself latch a byte overflow.
        // Poison the shared original epoch meter so clones cannot reset or
        // turn a smaller subsequent operation into success after this error.
        let _ = journal.charge(u64::MAX, u64::MAX);
        return Err(error);
    }
    journal.charge(bytes, entries as u64)?;
    Ok(())
}

pub(crate) fn encode<T: Serialize + std::any::Any>(
    value: &T,
    budget: &Budget,
    journal: &SharedJournalBudget,
) -> Result<Vec<u8>, BackupError> {
    let any = value as &dyn std::any::Any;
    // These trees already own sorted maps. Never clone an unbounded payload
    // into a second Value, and never use the recursive string formatter.
    if any.is::<serde_json::Value>() || any.is::<crate::full_restore::jobs::JobInventory>() {
        return typed_bytes(value, budget, journal);
    }
    // Only closed, shallow metadata schemas enter the tree conversion below.
    // No caller JSON, job checkpoint, display text or outbox is admitted here.
    if !(any.is::<crate::registry::Authority>()
        || any.is::<crate::registry::Generation>()
        || any.is::<crate::registry::SourceGroup>()
        || any.is::<crate::registry::RootRecord>()
        || any.is::<super::wire::Capture>()
        || any.is::<Vec<super::wire::NamespaceEntry>>())
    {
        return Err(invalid());
    }
    let n = count(value, budget, journal)?;
    charge(
        budget,
        journal,
        n.checked_mul(32).ok_or(BackupError::Overflow)?,
        1,
    )?;
    let raw = crate::registry::canonical(value)?;
    if raw.len() as u64 != n {
        return Err(invalid());
    }
    Ok(raw)
}

fn count(
    value: &impl Serialize,
    budget: &Budget,
    journal: &SharedJournalBudget,
) -> Result<u64, BackupError> {
    struct Count<'a> {
        n: u64,
        budget: &'a Budget,
        journal: &'a SharedJournalBudget,
    }
    impl Write for Count<'_> {
        fn write(&mut self, raw: &[u8]) -> std::io::Result<usize> {
            let next = self
                .n
                .checked_add(raw.len() as u64)
                .filter(|n| *n <= MAX_FILE)
                .ok_or_else(|| std::io::Error::other("adjunct encoding capacity"))?;
            charge(self.budget, self.journal, raw.len() as u64, 0)
                .map_err(std::io::Error::other)?;
            self.n = next;
            Ok(raw.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Count {
        n: 0,
        budget,
        journal,
    };
    serde_json::to_writer(&mut counter, value)?;
    Ok(counter.n)
}

/// Original typed field order, used only for lifecycle hashes, or an already
/// sorted Value/Inventory. Count, prepay output/hash, reserve once, bounded emit.
pub(crate) fn typed_bytes(
    value: &impl Serialize,
    budget: &Budget,
    journal: &SharedJournalBudget,
) -> Result<Vec<u8>, BackupError> {
    let n = count(value, budget, journal)?;
    charge(
        budget,
        journal,
        n.checked_mul(2).ok_or(BackupError::Overflow)?,
        1,
    )?;
    struct Output {
        raw: Vec<u8>,
        limit: usize,
    }
    impl Write for Output {
        fn write(&mut self, raw: &[u8]) -> std::io::Result<usize> {
            if self
                .raw
                .len()
                .checked_add(raw.len())
                .is_none_or(|n| n > self.limit)
            {
                return Err(std::io::Error::other("adjunct second encode grew"));
            }
            self.raw.extend_from_slice(raw);
            Ok(raw.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let limit = usize::try_from(n).map_err(|_| invalid())?;
    let mut output = Output {
        raw: Vec::new(),
        limit,
    };
    output
        .raw
        .try_reserve_exact(limit)
        .map_err(|_| BackupError::Capacity("adjunct allocation"))?;
    serde_json::to_writer(&mut output, value)?;
    if output.raw.len() != limit {
        return Err(invalid());
    }
    Ok(output.raw)
}

fn parent(
    root: &BackupDir,
    path_text: &str,
    create: bool,
) -> Result<(BackupDir, String), BackupError> {
    if !path(path_text) {
        return Err(invalid());
    }
    let mut parts = path_text.split('/').peekable();
    let mut dir = root.try_clone()?;
    dir.require_private_directory()?;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return Ok((dir, part.to_owned()));
        }
        dir = match dir.kind(part) {
            Ok(BackupEntryKind::Directory) => dir.open_dir(part)?,
            Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
                dir.create_dir(part)?
            }
            _ => return Err(invalid()),
        };
        dir.require_private_directory()?;
    }
    Err(invalid())
}

pub(crate) fn put(
    root: &BackupDir,
    name: &str,
    raw: &[u8],
    budget: &Budget,
    journal: &SharedJournalBudget,
) -> Result<FileRecord, BackupError> {
    if raw.is_empty() || raw.len() as u64 > MAX_FILE {
        return Err(invalid());
    }
    charge(
        budget,
        journal,
        (raw.len() as u64)
            .checked_mul(3)
            .ok_or(BackupError::Overflow)?,
        1,
    )?;
    let (dir, leaf) = parent(root, name, true)?;
    let mut file = dir.create_file(&leaf)?;
    file.write_all(raw)?;
    file.sync_all()?;
    dir.seal_file(&leaf)?;
    dir.sync()?;
    let record = FileRecord {
        path: name.into(),
        size: raw.len() as u64,
        sha256: hash(raw),
    };
    if read(root, &record, budget, journal)? != raw {
        return Err(invalid());
    }
    Ok(record)
}

pub(crate) fn read(
    root: &BackupDir,
    record: &FileRecord,
    budget: &Budget,
    journal: &SharedJournalBudget,
) -> Result<Vec<u8>, BackupError> {
    if record.size == 0 || record.size > MAX_FILE || !crate::valid_digest(&record.sha256) {
        return Err(invalid());
    }
    let (dir, leaf) = parent(root, &record.path, false)?;
    let mut file = dir.open_file(&leaf)?;
    let before = file.metadata()?;
    if before.len() != record.size
        || before.uid() != 0
        || before.gid() != 0
        || before.nlink() != 1
        || before.mode() & 0o7777 != 0o400
    {
        return Err(invalid());
    }
    charge(
        budget,
        journal,
        record.size.checked_mul(2).ok_or(BackupError::Overflow)?,
        1,
    )?;
    let mut raw = vec![0; usize::try_from(record.size).map_err(|_| invalid())?];
    file.read_exact(&mut raw)?;
    let mut extra = [0];
    if file.read(&mut extra)? != 0
        || hash(&raw) != record.sha256
        || !same(&before, &file.metadata()?)
    {
        return Err(invalid());
    }
    Ok(raw)
}

pub(crate) fn from_reader(
    root: &BackupDir,
    name: &str,
    size: u64,
    reader: &mut dyn Read,
    budget: &Budget,
    journal: &SharedJournalBudget,
) -> Result<FileRecord, BackupError> {
    if size == 0 || size > MAX_FILE {
        return Err(invalid());
    }
    charge(budget, journal, size, 1)?;
    let mut raw = vec![0; usize::try_from(size).map_err(|_| invalid())?];
    reader.read_exact(&mut raw)?;
    let mut extra = [0];
    if reader.read(&mut extra)? != 0 {
        return Err(invalid());
    }
    put(root, name, &raw, budget, journal)
}

fn same(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    (
        a.dev(),
        a.ino(),
        a.nlink(),
        a.len(),
        a.mtime(),
        a.mtime_nsec(),
        a.ctime(),
        a.ctime_nsec(),
    ) == (
        b.dev(),
        b.ino(),
        b.nlink(),
        b.len(),
        b.mtime(),
        b.mtime_nsec(),
        b.ctime(),
        b.ctime_nsec(),
    )
}

pub(crate) fn exact_tree(
    root: &BackupDir,
    records: &[FileRecord],
    budget: &Budget,
    journal: &SharedJournalBudget,
) -> Result<(), BackupError> {
    let mut expected = BTreeSet::new();
    let mut expected_dirs = BTreeSet::new();
    for record in records {
        if !path(&record.path) || !expected.insert(record.path.clone()) {
            return Err(invalid());
        }
        charge(budget, journal, record.path.len() as u64 * 2, 1)?;
        let mut cursor = record.path.as_str();
        while let Some((parent, _)) = cursor.rsplit_once('/') {
            expected_dirs.insert(parent.to_owned());
            cursor = parent;
        }
    }
    fn walk(
        dir: &BackupDir,
        prefix: &str,
        actual: &mut BTreeSet<String>,
        directories: &BTreeSet<String>,
        budget: &Budget,
        journal: &SharedJournalBudget,
    ) -> Result<(), BackupError> {
        dir.require_private_directory()?;
        charge(budget, journal, 0, 1)?;
        let identity = dir.identity()?;
        let before = dir.list_bounded(MAX_FILES.saturating_sub(actual.len()))?;
        for leaf in &before {
            let name = if prefix.is_empty() {
                leaf.clone()
            } else {
                format!("{prefix}/{leaf}")
            };
            if !path(&name) {
                return Err(invalid());
            }
            charge(budget, journal, name.len() as u64, 1)?;
            match dir.kind(leaf)? {
                BackupEntryKind::File => {
                    if !actual.insert(name) {
                        return Err(invalid());
                    }
                }
                BackupEntryKind::Directory => {
                    if !directories.contains(&name) {
                        return Err(invalid());
                    }
                    let child = dir.open_dir(leaf)?;
                    let child_identity = child.identity()?;
                    walk(&child, &name, actual, directories, budget, journal)?;
                    if dir.open_dir(leaf)?.identity()? != child_identity {
                        return Err(invalid());
                    }
                }
                BackupEntryKind::Other => return Err(invalid()),
            }
        }
        if dir.identity()? != identity || dir.list_bounded(MAX_FILES)? != before {
            return Err(invalid());
        }
        Ok(())
    }
    let mut actual = BTreeSet::new();
    walk(root, "", &mut actual, &expected_dirs, budget, journal)?;
    if actual != expected {
        return Err(invalid());
    }

    for record in records {
        read(root, record, budget, journal)?;
    }
    Ok(())
}

// The normal source publisher seals package directories 0500. This separate
// reader is only for the private Fresh(pin) binding; adjunct directories keep
// the existing exact_tree 0700 contract.
#[cfg(test)]
pub(crate) fn sealed_package_tree(
    root: &BackupDir,
    records: &[FileRecord],
    budget: &Budget,
    journal: &SharedJournalBudget,
) -> Result<(), BackupError> {
    use std::{collections::BTreeMap, fs::Metadata};

    fn directory_at(
        root: &BackupDir,
        name: &str,
        budget: &Budget,
        journal: &SharedJournalBudget,
    ) -> Result<BackupDir, BackupError> {
        charge(budget, journal, 512, 1)?;
        let mut dir = root.try_clone()?;
        dir.require_sealed_directory()?;
        if !name.is_empty() {
            for part in name.split('/') {
                charge(budget, journal, 512, 1)?;
                dir = dir.open_dir(part)?;
                dir.require_sealed_directory()?;
            }
        }
        Ok(dir)
    }
    fn sealed_file(meta: &Metadata, record: &FileRecord) -> Result<(), BackupError> {
        if !meta.is_file()
            || meta.uid() != 0
            || meta.gid() != 0
            || meta.mode() & 0o7777 != 0o400
            || meta.nlink() != 1
            || meta.len() != record.size
        {
            return Err(invalid());
        }
        Ok(())
    }
    struct Scan<'a> {
        records: BTreeMap<&'a str, &'a FileRecord>,
        expected_dirs: BTreeSet<String>,
        directories: BTreeMap<String, (Metadata, Vec<String>)>,
        files: BTreeMap<&'a str, Metadata>,
        budget: &'a Budget,
        journal: &'a SharedJournalBudget,
    }
    impl Scan<'_> {
        fn names(&self, dir: &BackupDir, count: usize) -> Result<Vec<String>, BackupError> {
            // Linux names contain at most 255 bytes. 512/entry covers those
            // bytes and Vec<String> growth/moves before bounded enumeration.
            charge(
                self.budget,
                self.journal,
                (count as u64)
                    .checked_mul(512)
                    .and_then(|n| n.checked_add(512))
                    .ok_or(BackupError::Overflow)?,
                count.checked_add(1).ok_or(BackupError::Overflow)?,
            )?;
            let mut names = dir.list_bounded(count)?;
            names.sort_unstable();
            Ok(names)
        }
        fn walk(&mut self, dir: &BackupDir, prefix: &str) -> Result<(), BackupError> {
            charge(self.budget, self.journal, 512, 1)?;
            let before = dir.require_sealed_directory()?;
            let mut direct = BTreeSet::new();
            for name in self.records.keys() {
                // Every key is inspected, including nonmatches in nested scans.
                charge(
                    self.budget,
                    self.journal,
                    (name.len() + prefix.len()) as u64,
                    0,
                )?;
                let tail = if prefix.is_empty() {
                    Some(*name)
                } else {
                    name.strip_prefix(prefix).and_then(|s| s.strip_prefix('/'))
                };
                if let Some(leaf) = tail.and_then(|s| s.split('/').next()) {
                    charge(self.budget, self.journal, 128, 1)?;
                    direct.insert(leaf);
                }
            }
            let names = self.names(dir, direct.len())?;
            if names.len() != direct.len()
                || names.iter().map(String::as_str).ne(direct.iter().copied())
            {
                return Err(invalid());
            }
            for leaf in &names {
                charge(
                    self.budget,
                    self.journal,
                    ((prefix.len() + leaf.len() + 1) as u64) * 2 + 512,
                    1,
                )?;
                let mut name = String::with_capacity(prefix.len() + leaf.len() + 1);
                if !prefix.is_empty() {
                    name.push_str(prefix);
                    name.push('/');
                }
                name.push_str(leaf);
                match dir.kind(leaf)? {
                    BackupEntryKind::File => {
                        let record = *self.records.get(name.as_str()).ok_or_else(invalid)?;
                        charge(self.budget, self.journal, 1024, 4)?;
                        let mut file = dir.open_file(leaf)?;
                        let before = file.metadata()?;
                        sealed_file(&before, record)?;
                        sealed_stream_digest(&mut file, record, self.budget, self.journal)?;
                        let after = file.metadata()?;
                        sealed_file(&after, record)?;
                        let named = dir.open_file(leaf)?.metadata()?;
                        sealed_file(&named, record)?;
                        if !same(&before, &after) || !same(&before, &named) {
                            return Err(invalid());
                        }
                        if self.files.insert(record.path.as_str(), before).is_some() {
                            return Err(invalid());
                        }
                    }
                    BackupEntryKind::Directory => {
                        if !self.expected_dirs.contains(&name) {
                            return Err(invalid());
                        }
                        self.walk(&dir.open_dir(leaf)?, &name)?;
                    }
                    BackupEntryKind::Other => return Err(invalid()),
                }
            }
            if !same(&before, &dir.require_sealed_directory()?) {
                return Err(invalid());
            }
            charge(self.budget, self.journal, prefix.len() as u64 + 128, 1)?;
            self.directories.insert(prefix.to_owned(), (before, names));
            Ok(())
        }
    }
    sealed_record_count(records.len())?;
    let mut scan = Scan {
        records: BTreeMap::new(),
        expected_dirs: BTreeSet::new(),
        directories: BTreeMap::new(),
        files: BTreeMap::new(),
        budget,
        journal,
    };
    for record in records {
        if !path(&record.path) || !crate::valid_digest(&record.sha256) {
            return Err(invalid());
        }
        charge(budget, journal, 128, 1)?;
        if scan.records.insert(record.path.as_str(), record).is_some() {
            return Err(invalid());
        }
        let mut cursor = record.path.as_str();
        while let Some((parent, _)) = cursor.rsplit_once('/') {
            charge(budget, journal, parent.len() as u64 + 128, 1)?;
            scan.expected_dirs.insert(parent.to_owned());
            cursor = parent;
        }
    }
    scan.walk(root, "")?;
    if scan.files.len() != records.len() {
        return Err(invalid());
    }
    for (name, (before, names)) in &scan.directories {
        let dir = directory_at(root, name, budget, journal)?;
        if !same(before, &dir.require_sealed_directory()?)
            || scan.names(&dir, names.len())? != *names
        {
            return Err(invalid());
        }
    }
    for (name, before) in &scan.files {
        let (parent, leaf) = name.rsplit_once('/').unwrap_or(("", name));
        let dir = directory_at(root, parent, budget, journal)?;
        charge(budget, journal, 512, 1)?;
        let after = dir.open_file(leaf)?.metadata()?;
        sealed_file(&after, scan.records[name])?;
        if !same(before, &after) {
            return Err(invalid());
        }
    }
    if !same(&scan.directories[""].0, &root.require_sealed_directory()?) {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
fn sealed_record_count(count: usize) -> Result<(), BackupError> {
    let maximum = crate::MAX_LOGICAL_ASSETS
        .checked_add(4)
        .ok_or(BackupError::Overflow)?;
    if count == 0 || count > maximum {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
fn sealed_stream_digest(
    reader: &mut impl Read,
    record: &FileRecord,
    budget: &Budget,
    journal: &SharedJournalBudget,
) -> Result<(), BackupError> {
    use sha2::{Digest, Sha256};
    if !crate::valid_digest(&record.sha256) {
        return Err(invalid());
    }
    // The original opaque Fresh manifest supplies base-package size limits.
    // Do not impose the separate adjunct MAX_FILE limit on a database dump.
    // Prepay the fixed buffer, every read/hash byte, EOF probe and hex output.
    charge(
        budget,
        journal,
        record
            .size
            .checked_mul(2)
            .and_then(|n| n.checked_add(8192 + 129))
            .ok_or(BackupError::Overflow)?,
        1,
    )?;
    let mut buffer = [0_u8; 8192];
    let mut remaining = record.size;
    let mut digest = Sha256::new();
    while remaining != 0 {
        let limit = usize::try_from(remaining.min(buffer.len() as u64)).map_err(|_| invalid())?;
        let count = reader.read(&mut buffer[..limit])?;
        if count == 0 {
            return Err(invalid());
        }
        remaining -= count as u64;
        digest.update(&buffer[..count]);
    }
    if reader.read(&mut buffer[..1])? != 0 || hex::encode(digest.finalize()) != record.sha256 {
        return Err(invalid());
    }
    Ok(())
}

pub(crate) fn seal_tree(root: &BackupDir) -> Result<(), BackupError> {
    for leaf in root.list_bounded(MAX_FILES)? {
        if root.kind(&leaf)? == BackupEntryKind::Directory {
            seal_tree(&root.open_dir(&leaf)?)?;
        }
    }
    // Root's private directories stay 0700 so the existing no-create package
    // reader accepts them. Every regular file was sealed 0400 and read back;
    // no writable directory handle escapes the opaque object.
    root.require_private_directory()?;
    root.sync()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "actual Root plus fresh TASK6_CUSTODY_TEST_PARENT; sealed package FS only, no PG"]
    fn sealed_source_package_requires_exact_read_only_tree_and_original_budget() {
        use std::{
            fs,
            io::Cursor,
            os::unix::fs::{PermissionsExt, symlink},
            path::Path,
        };
        assert_eq!(unsafe { libc::geteuid() }, 0, "actual Root required");
        let parent_path =
            std::env::var_os("TASK6_CUSTODY_TEST_PARENT").expect("fresh Root FS fixture");
        let parent = BackupDir::open_trusted_private_root(Path::new(&parent_path)).unwrap();
        let leaf = format!("sealed-pin-check-{}", uuid::Uuid::new_v4());
        let path = Path::new(&parent_path).join(&leaf);
        let fixture = parent.create_dir(&leaf).unwrap();
        let package = fixture.create_dir("package").unwrap();
        let nested = package.create_dir("nested").unwrap();
        let package_path = path.join("package");
        let mut records = Vec::new();
        for (name, raw) in [
            ("zero", &b""[..]),
            ("manifest.json", &b"{}"[..]),
            ("nested/data", &b"abc"[..]),
        ] {
            let (dir, leaf) = if let Some((_, leaf)) = name.rsplit_once('/') {
                (&nested, leaf)
            } else {
                (&package, name)
            };
            let mut file = dir.create_file(leaf).unwrap();
            file.write_all(raw).unwrap();
            file.sync_all().unwrap();
            dir.seal_file(leaf).unwrap();
            records.push(FileRecord {
                path: name.into(),
                size: raw.len() as u64,
                sha256: hash(raw),
            });
        }
        nested.seal_dir().unwrap();
        package.seal_dir().unwrap();
        let budget = Arc::new(Mutex::new(ScanBudget::default()));
        let journal = SharedJournalBudget::default();
        let original_clone = journal.clone();
        let directory_before = package.require_sealed_directory().unwrap();
        let files_before: Vec<_> = records
            .iter()
            .map(|r| fs::metadata(package_path.join(&r.path)).unwrap())
            .collect();
        sealed_package_tree(&package, &records, &budget, &journal).unwrap();
        assert!(same(
            &directory_before,
            &package.require_sealed_directory().unwrap()
        ));
        for (record, before) in records.iter().zip(files_before) {
            let after = fs::metadata(package_path.join(&record.path)).unwrap();
            assert!(same(&before, &after));
            assert_eq!(
                (
                    after.uid(),
                    after.gid(),
                    after.mode() & 0o7777,
                    after.nlink()
                ),
                (0, 0, 0o400, 1)
            );
        }
        assert!(budget.lock().unwrap().bytes > 0);
        assert!(journal.usage().unwrap().charged_bytes > 0);
        assert_eq!(journal.usage().unwrap(), original_clone.usage().unwrap());
        assert!(package.require_private_directory().is_err());
        assert!(exact_tree(&package, &records, &budget, &journal).is_err());

        for mode in [0o700, 0o555] {
            fs::set_permissions(&package_path, fs::Permissions::from_mode(mode)).unwrap();
            assert!(sealed_package_tree(&package, &records, &budget, &journal).is_err());
        }
        fs::set_permissions(&package_path, fs::Permissions::from_mode(0o500)).unwrap();
        fs::set_permissions(
            package_path.join("nested"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        assert!(sealed_package_tree(&package, &records, &budget, &journal).is_err());
        fs::set_permissions(
            package_path.join("nested"),
            fs::Permissions::from_mode(0o500),
        )
        .unwrap();
        let data = package_path.join("nested/data");
        fs::set_permissions(&data, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(sealed_package_tree(&package, &records, &budget, &journal).is_err());
        fs::set_permissions(&data, fs::Permissions::from_mode(0o400)).unwrap();
        let link = path.join("hardlink");
        fs::hard_link(&data, &link).unwrap();
        assert!(sealed_package_tree(&package, &records, &budget, &journal).is_err());
        fs::remove_file(&link).unwrap();
        fs::rename(&data, &link).unwrap();
        symlink(&link, &data).unwrap();
        assert!(sealed_package_tree(&package, &records, &budget, &journal).is_err());
        fs::remove_file(&data).unwrap();
        fs::rename(&link, &data).unwrap();
        fs::write(&data, b"abd").unwrap();
        assert!(sealed_package_tree(&package, &records, &budget, &journal).is_err());
        fs::write(&data, b"abc").unwrap();
        fs::write(package_path.join("extra"), b"extra").unwrap();
        assert!(sealed_package_tree(&package, &records, &budget, &journal).is_err());
        fs::remove_file(package_path.join("extra")).unwrap();
        let record = records.iter().find(|r| r.path == "nested/data").unwrap();
        assert!(sealed_stream_digest(&mut Cursor::new(b"ab"), record, &budget, &journal).is_err());
        assert!(
            sealed_stream_digest(&mut Cursor::new(b"abcd"), record, &budget, &journal).is_err()
        );
        sealed_package_tree(&package, &records, &budget, &journal).unwrap();

        let ordinary = fixture.create_dir("ordinary").unwrap();
        let ordinary_record = put(&ordinary, "data", b"ordinary", &budget, &journal).unwrap();
        exact_tree(
            &ordinary,
            std::slice::from_ref(&ordinary_record),
            &budget,
            &journal,
        )
        .unwrap();
        assert!(sealed_package_tree(&ordinary, &[ordinary_record], &budget, &journal).is_err());
        // Base package count includes asset originals plus dump/roles/index/manifest.
        assert!(sealed_record_count(crate::MAX_LOGICAL_ASSETS).is_ok());
        assert!(sealed_record_count(crate::MAX_LOGICAL_ASSETS + 4).is_ok());
        assert!(sealed_record_count(crate::MAX_LOGICAL_ASSETS + 5).is_err());
        assert!(sealed_record_count(0).is_err());
        // No large disk file or whole-file Vec: verify a base-sized stream above
        // the unrelated adjunct 64MiB boundary on the same original meters.
        use sha2::{Digest, Sha256};
        let size = 64 * 1024 * 1024 + 1_u64;
        let block = [11_u8; 8192];
        let mut expected = Sha256::new();
        let mut remaining = size;
        while remaining != 0 {
            let count = remaining.min(block.len() as u64) as usize;
            expected.update(&block[..count]);
            remaining -= count as u64;
        }
        let large = FileRecord {
            path: "large-stream".into(),
            size,
            sha256: hex::encode(expected.finalize()),
        };
        let before_large = journal.usage().unwrap().charged_bytes;
        sealed_stream_digest(
            &mut std::io::repeat(11).take(size),
            &large,
            &budget,
            &journal,
        )
        .unwrap();
        assert_eq!(
            journal.usage().unwrap().charged_bytes - before_large,
            2 * size + 8192 + 129
        );
        let exhausted = SharedJournalBudget::default();
        exhausted.charge(512 * 1024 * 1024, 0).unwrap();
        let exhausted_clone = exhausted.clone();
        assert!(sealed_package_tree(&package, &records, &budget, &exhausted).is_err());
        assert!(exhausted_clone.charge(0, 0).is_err());
        assert!(
            sealed_stream_digest(
                &mut std::io::repeat(11).take(size),
                &large,
                &budget,
                &exhausted_clone
            )
            .is_err()
        );
        drop((ordinary, nested, package, fixture));
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn linear_encoding_prepays_output_and_hash_and_exhaustion_is_shared() {
        let value = serde_json::json!({"large":"测\"\\\n".repeat(128*1024),"nested":{"z":null,"a":[true,7]}});
        let expected = serde_json::to_vec(&value).unwrap();
        let n = expected.len() as u64;
        let budget = Arc::new(Mutex::new(ScanBudget::default()));
        let journal = SharedJournalBudget::default();
        journal.charge(512 * 1024 * 1024 - 3 * n, 0).unwrap();
        let raw = typed_bytes(&value, &budget, &journal).unwrap();
        assert_eq!(raw, expected);
        assert_eq!(hash(&raw), hash(&expected));
        assert_eq!(budget.lock().unwrap().bytes, 3 * n);
        for remaining in [n - 1, 3 * n - 1] {
            let budget = Arc::new(Mutex::new(ScanBudget::default()));
            let journal = SharedJournalBudget::default();
            journal.charge(512 * 1024 * 1024 - remaining, 0).unwrap();
            let clone = journal.clone();
            assert!(typed_bytes(&value, &budget, &journal).is_err());
            assert!(clone.charge(0, 0).is_err());
        }
    }
    #[test]
    fn registry_overflow_poisons_the_same_epoch_meter() {
        let budget = Arc::new(Mutex::new(ScanBudget::default()));
        budget.lock().unwrap().bytes = 512 * 1024 * 1024;
        let journal = SharedJournalBudget::default();
        let clone = journal.clone();
        assert!(charge(&budget, &journal, 1, 0).is_err());
        assert!(clone.charge(0, 0).is_err());
    }
}
