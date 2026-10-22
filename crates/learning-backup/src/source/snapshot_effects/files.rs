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
