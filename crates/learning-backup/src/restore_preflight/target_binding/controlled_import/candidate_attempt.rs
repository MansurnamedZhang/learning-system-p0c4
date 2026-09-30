//! Candidate import journal. No SQL authority is granted by these records.
use super::{ImportFailure, protocol::WriterIdentity};
use learning_assets::backup_fs::BackupDir;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{self, Read, Write};
use uuid::Uuid;

const CANDIDATE_TYPE: &str = "CONTROLLED_IMPORT_CANDIDATE";
const JOURNAL_VERSION: u32 = 1;
const FIXTURE_VERSION: u32 = 1;
const MAX_ATTEMPT_BYTES: u64 = 4096;

#[derive(Serialize)]
struct AttemptRecord<'a> {
    format_version: u32,
    record_type: &'static str,
    phase: &'static str,
    batch_id: Uuid,
    database: &'a str,
    birth_sha256: String,
    inspection_sha256: String,
    dump_sha256: String,
    raw_sql_sha256: String,
    transformed_sql_sha256: String,
    fixture_version: u32,
    writer_sha256: String,
}

#[derive(Serialize)]
struct CommitIntentRecord<'a> {
    format_version: u32,
    record_type: &'static str,
    phase: &'static str,
    database: &'a str,
    attempt_sha256: String,
    writer_sha256: String,
}

pub(super) struct CandidateAttemptContext {
    batch_id: Uuid,
    database: String,
    birth_sha256: [u8; 32],
    inspection_sha256: [u8; 32],
    dump_sha256: [u8; 32],
    raw_sql_sha256: [u8; 32],
    transformed_sql_sha256: [u8; 32],
    fixture_version: u32,
    writer: WriterIdentity,
}

pub(super) struct DurableAttempt {
    database: String,
    directory_identity: (u64, u64),
    writer: WriterIdentity,
    file_sha256: [u8; 32],
}

pub(super) struct DurableCommitIntent {
    attempt_sha256: [u8; 32],
}

pub(super) trait JournalIo {
    type File: Write;
    type Reader: Read;
    fn identity(&self) -> io::Result<(u64, u64)>;
    fn kind(&self, name: &str) -> io::Result<learning_assets::backup_fs::BackupEntryKind>;
    fn create_file(&self, name: &str) -> io::Result<Self::File>;
    fn open_file(&self, name: &str) -> io::Result<Self::Reader>;
    fn sync_file(&self, file: &Self::File) -> io::Result<()>;
    fn sync_dir(&self) -> io::Result<()>;
}

impl JournalIo for BackupDir {
    type File = std::fs::File;
    type Reader = std::fs::File;
    fn identity(&self) -> io::Result<(u64, u64)> {
        BackupDir::identity(self)
    }
    fn kind(&self, name: &str) -> io::Result<learning_assets::backup_fs::BackupEntryKind> {
        BackupDir::kind(self, name)
    }
    fn create_file(&self, name: &str) -> io::Result<Self::File> {
        BackupDir::create_file(self, name)
    }
    fn open_file(&self, name: &str) -> io::Result<Self::Reader> {
        BackupDir::open_file(self, name)
    }
    fn sync_file(&self, file: &Self::File) -> io::Result<()> {
        file.sync_all()
    }
    fn sync_dir(&self) -> io::Result<()> {
        BackupDir::sync(self)
    }
}

impl CandidateAttemptContext {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        batch_id: Uuid,
        database: String,
        birth_sha256: [u8; 32],
        inspection_sha256: [u8; 32],
        dump_sha256: [u8; 32],
        raw_sql_sha256: [u8; 32],
        transformed_sql_sha256: [u8; 32],
        writer: WriterIdentity,
    ) -> Result<Self, ImportFailure> {
        if batch_id.is_nil() || super::super::super::restore_attempt_name(&database).is_err() {
            return Err(ImportFailure::Identity);
        }
        Ok(Self {
            batch_id,
            database,
            birth_sha256,
            inspection_sha256,
            dump_sha256,
            raw_sql_sha256,
            transformed_sql_sha256,
            fixture_version: FIXTURE_VERSION,
            writer,
        })
    }
}

pub(super) fn persist_attempt(
    dir: &BackupDir,
    context: &CandidateAttemptContext,
) -> Result<DurableAttempt, ImportFailure> {
    persist_attempt_with_io(dir, context)
}

pub(super) fn persist_commit_intent(
    dir: &BackupDir,
    attempt: &DurableAttempt,
    writer: &WriterIdentity,
) -> Result<DurableCommitIntent, ImportFailure> {
    persist_commit_intent_with_io(dir, attempt, writer)
}

pub(super) fn persist_attempt_with_io(
    dir: &impl JournalIo,
    context: &CandidateAttemptContext,
) -> Result<DurableAttempt, ImportFailure> {
    let name = super::super::super::restore_attempt_name(&context.database)
        .map_err(|_| ImportFailure::Identity)?;
    let directory_identity = dir.identity().map_err(|_| ImportFailure::Journal)?;
    reject_existing(dir, &name)?;
    let bytes = serde_json::to_vec(&AttemptRecord {
        format_version: JOURNAL_VERSION,
        record_type: CANDIDATE_TYPE,
        phase: "ATTEMPT",
        batch_id: context.batch_id,
        database: &context.database,
        birth_sha256: hex::encode(context.birth_sha256),
        inspection_sha256: hex::encode(context.inspection_sha256),
        dump_sha256: hex::encode(context.dump_sha256),
        raw_sql_sha256: hex::encode(context.raw_sql_sha256),
        transformed_sql_sha256: hex::encode(context.transformed_sql_sha256),
        fixture_version: context.fixture_version,
        writer_sha256: hex::encode(context.writer.fingerprint_sha256()),
    })
    .map_err(|_| ImportFailure::Journal)?;
    write_and_sync(dir, &name, &bytes)?;
    Ok(DurableAttempt {
        database: context.database.clone(),
        directory_identity,
        writer: context.writer,
        file_sha256: Sha256::digest(&bytes).into(),
    })
}

pub(super) fn persist_commit_intent_with_io(
    dir: &impl JournalIo,
    attempt: &DurableAttempt,
    writer: &WriterIdentity,
) -> Result<DurableCommitIntent, ImportFailure> {
    if dir.identity().map_err(|_| ImportFailure::Journal)? != attempt.directory_identity
        || *writer != attempt.writer
    {
        return Err(ImportFailure::Identity);
    }
    let attempt_name = super::super::super::restore_attempt_name(&attempt.database)
        .map_err(|_| ImportFailure::Identity)?;
    let mut stored_attempt = Vec::new();
    dir.open_file(&attempt_name)
        .map_err(|_| ImportFailure::Journal)?
        .take(MAX_ATTEMPT_BYTES + 1)
        .read_to_end(&mut stored_attempt)
        .map_err(|_| ImportFailure::Journal)?;
    if stored_attempt.len() as u64 > MAX_ATTEMPT_BYTES
        || Sha256::digest(&stored_attempt).as_slice() != attempt.file_sha256
    {
        return Err(ImportFailure::Journal);
    }
    let intent_name = format!("{}.restore.commit-attempt", attempt.database);
    reject_existing(dir, &intent_name)?;
    let bytes = serde_json::to_vec(&CommitIntentRecord {
        format_version: JOURNAL_VERSION,
        record_type: CANDIDATE_TYPE,
        phase: "COMMIT_ATTEMPTED",
        database: &attempt.database,
        attempt_sha256: hex::encode(attempt.file_sha256),
        writer_sha256: hex::encode(writer.fingerprint_sha256()),
    })
    .map_err(|_| ImportFailure::Journal)?;
    write_and_sync(dir, &intent_name, &bytes)?;
    Ok(DurableCommitIntent {
        attempt_sha256: attempt.file_sha256,
    })
}

fn reject_existing(dir: &impl JournalIo, name: &str) -> Result<(), ImportFailure> {
    super::super::super::reject_existing_attempt(dir.kind(name)).map_err(|_| ImportFailure::Journal)
}

fn write_and_sync(dir: &impl JournalIo, name: &str, bytes: &[u8]) -> Result<(), ImportFailure> {
    let mut file = dir.create_file(name).map_err(|_| ImportFailure::Journal)?;
    file.write_all(bytes).map_err(|_| ImportFailure::Journal)?;
    dir.sync_file(&file).map_err(|_| ImportFailure::Journal)?;
    drop(file);
    dir.sync_dir().map_err(|_| ImportFailure::Journal)
}

#[cfg(test)]
mod tests {
    use super::super::protocol::{Nonce, WriterEvent, parse_writer_line};
    use super::*;
    use learning_assets::backup_fs::BackupEntryKind;
    use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

    const DATABASE: &str = "learning_restore_c4_2b8a1252-54d5-48aa-b176-a9586a86bea3";

    #[derive(Clone)]
    enum Entry {
        File(Rc<RefCell<Vec<u8>>>),
        Symlink,
    }
    #[derive(Default)]
    struct State {
        entries: BTreeMap<String, Entry>,
        fail_file_sync: bool,
        fail_dir_sync: bool,
    }
    struct ModelDir {
        identity: (u64, u64),
        state: Rc<RefCell<State>>,
    }
    struct ModelFile(Rc<RefCell<Vec<u8>>>);
    impl Write for ModelFile {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl ModelDir {
        fn new(identity: (u64, u64)) -> Self {
            Self {
                identity,
                state: Rc::new(RefCell::new(State::default())),
            }
        }
        fn entry(&self, name: &str) -> Option<Vec<u8>> {
            match self.state.borrow().entries.get(name) {
                Some(Entry::File(bytes)) => Some(bytes.borrow().clone()),
                _ => None,
            }
        }
    }
    impl JournalIo for ModelDir {
        type File = ModelFile;
        type Reader = io::Cursor<Vec<u8>>;
        fn identity(&self) -> io::Result<(u64, u64)> {
            Ok(self.identity)
        }
        fn kind(&self, name: &str) -> io::Result<BackupEntryKind> {
            match self.state.borrow().entries.get(name) {
                Some(Entry::File(_)) => Ok(BackupEntryKind::File),
                Some(Entry::Symlink) => Ok(BackupEntryKind::Other),
                None => Err(io::Error::from(io::ErrorKind::NotFound)),
            }
        }
        fn create_file(&self, name: &str) -> io::Result<Self::File> {
            let mut state = self.state.borrow_mut();
            if state.entries.contains_key(name) {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
            let bytes = Rc::new(RefCell::new(Vec::new()));
            state
                .entries
                .insert(name.to_owned(), Entry::File(bytes.clone()));
            Ok(ModelFile(bytes))
        }
        fn open_file(&self, name: &str) -> io::Result<Self::Reader> {
            self.entry(name)
                .map(io::Cursor::new)
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
        fn sync_file(&self, _file: &Self::File) -> io::Result<()> {
            if self.state.borrow().fail_file_sync {
                Err(io::Error::other("injected file sync"))
            } else {
                Ok(())
            }
        }
        fn sync_dir(&self) -> io::Result<()> {
            if self.state.borrow().fail_dir_sync {
                Err(io::Error::other("injected directory sync"))
            } else {
                Ok(())
            }
        }
    }

    fn writer(pid: i32) -> WriterIdentity {
        let nonce = Nonce::random().unwrap();
        let line = format!("KW_C4|{}|READY|{pid}|1234567|99\n", nonce.hex());
        let WriterEvent::Ready(writer) = parse_writer_line(line.as_bytes(), &nonce).unwrap() else {
            panic!("READY")
        };
        writer
    }
    fn context() -> CandidateAttemptContext {
        CandidateAttemptContext::new(
            Uuid::parse_str("f994b584-0ea8-4e2d-8246-8994b86b58e5").unwrap(),
            DATABASE.into(),
            [1; 32],
            [2; 32],
            [3; 32],
            [4; 32],
            [5; 32],
            writer(42),
        )
        .unwrap()
    }
    fn attempt_name() -> String {
        format!("{DATABASE}.restore.attempt")
    }
    fn intent_name() -> String {
        format!("{DATABASE}.restore.commit-attempt")
    }

    #[test]
    fn journal_rejects_existing_regular_symlink_and_partial_entry() {
        let fresh = ModelDir::new((1, 1));
        assert!(persist_attempt_with_io(&fresh, &context()).is_ok());
        for existing in [
            Entry::File(Rc::new(RefCell::new(b"previous".to_vec()))),
            Entry::Symlink,
            Entry::File(Rc::new(RefCell::new(b"partial".to_vec()))),
        ] {
            let dir = ModelDir::new((1, 2));
            dir.state
                .borrow_mut()
                .entries
                .insert(attempt_name(), existing);
            let before = dir.entry(&attempt_name());
            assert!(matches!(
                persist_attempt_with_io(&dir, &context()),
                Err(ImportFailure::Journal)
            ));
            assert_eq!(dir.entry(&attempt_name()), before);
            assert!(dir.entry(&intent_name()).is_none());
        }
    }
    #[test]
    fn file_sync_failure_returns_no_durable_permit() {
        let dir = ModelDir::new((1, 2));
        dir.state.borrow_mut().fail_file_sync = true;
        assert!(matches!(
            persist_attempt_with_io(&dir, &context()),
            Err(ImportFailure::Journal)
        ));
        let bytes = dir.entry(&attempt_name()).expect("failed entry retained");
        assert!(!bytes.is_empty());
        assert!(
            !bytes
                .windows(b"receipt_sha256".len())
                .any(|x| x == b"receipt_sha256")
        );
        assert!(dir.entry(&intent_name()).is_none());
    }
    #[test]
    fn directory_sync_failure_preserves_entry() {
        let dir = ModelDir::new((1, 2));
        dir.state.borrow_mut().fail_dir_sync = true;
        assert!(matches!(
            persist_attempt_with_io(&dir, &context()),
            Err(ImportFailure::Journal)
        ));
        let before = dir.entry(&attempt_name()).expect("failed entry retained");
        dir.state.borrow_mut().fail_dir_sync = false;
        assert!(matches!(
            persist_attempt_with_io(&dir, &context()),
            Err(ImportFailure::Journal)
        ));
        assert_eq!(dir.entry(&attempt_name()), Some(before));
    }
    #[test]
    fn commit_intent_binds_attempt_and_writer() {
        let dir = ModelDir::new((1, 2));
        let context = context();
        let attempt = persist_attempt_with_io(&dir, &context).expect("durable attempt");
        assert!(matches!(
            persist_commit_intent_with_io(&dir, &attempt, &writer(43)),
            Err(ImportFailure::Identity)
        ));
        assert!(dir.entry(&intent_name()).is_none());
        let other_dir = ModelDir::new((1, 3));
        assert!(matches!(
            persist_commit_intent_with_io(&other_dir, &attempt, &context.writer),
            Err(ImportFailure::Identity)
        ));
        let original = dir.entry(&attempt_name()).unwrap();
        if let Some(Entry::File(bytes)) = dir.state.borrow().entries.get(&attempt_name()) {
            bytes.borrow_mut().push(b'!');
        }
        assert!(matches!(
            persist_commit_intent_with_io(&dir, &attempt, &context.writer),
            Err(ImportFailure::Journal)
        ));
        if let Some(Entry::File(bytes)) = dir.state.borrow().entries.get(&attempt_name()) {
            *bytes.borrow_mut() = original;
        }
        assert!(dir.entry(&intent_name()).is_none());
        let intent =
            persist_commit_intent_with_io(&dir, &attempt, &context.writer).expect("durable intent");
        assert_eq!(intent.attempt_sha256, attempt.file_sha256);
        let bytes = dir.entry(&intent_name()).expect("intent retained");
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["attempt_sha256"], hex::encode(attempt.file_sha256));
        assert!(json.get("writer_sha256").is_some());
        assert!(json.get("receipt_sha256").is_none());
        assert!(matches!(
            persist_commit_intent_with_io(&dir, &attempt, &context.writer),
            Err(ImportFailure::Journal)
        ));
        assert_eq!(dir.entry(&intent_name()), Some(bytes));

        for fail_file in [true, false] {
            let dir = ModelDir::new((2, if fail_file { 1 } else { 2 }));
            let attempt = persist_attempt_with_io(&dir, &context).unwrap();
            dir.state.borrow_mut().fail_file_sync = fail_file;
            dir.state.borrow_mut().fail_dir_sync = !fail_file;
            assert!(matches!(
                persist_commit_intent_with_io(&dir, &attempt, &context.writer),
                Err(ImportFailure::Journal)
            ));
            let partial = dir
                .entry(&intent_name())
                .expect("intent retained on sync failure");
            dir.state.borrow_mut().fail_file_sync = false;
            dir.state.borrow_mut().fail_dir_sync = false;
            assert!(matches!(
                persist_commit_intent_with_io(&dir, &attempt, &context.writer),
                Err(ImportFailure::Journal)
            ));
            assert_eq!(dir.entry(&intent_name()), Some(partial));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires a separately authorized fresh root-private test parent"]
    fn live_candidate_journal_no_follow_and_fsync() {
        use std::os::unix::fs::MetadataExt;

        let parent = std::env::var_os("KNOWWEAVE_C4_JOURNAL_TEST_PARENT")
            .expect("set a new-batch root-private test parent on the authorized Linux host");
        let parent = BackupDir::open_trusted_private_root(std::path::Path::new(&parent))
            .expect("trusted root-private test parent");
        let child_name = format!("journal-{}", Uuid::new_v4());
        let dir = parent.create_dir(&child_name).expect("fresh private child");
        let child_path =
            std::path::Path::new(&std::env::var_os("KNOWWEAVE_C4_JOURNAL_TEST_PARENT").unwrap())
                .join(&child_name);
        let root_meta = std::fs::symlink_metadata(&child_path).unwrap();
        assert_eq!(root_meta.uid(), 0);
        assert_eq!(root_meta.mode() & 0o777, 0o700);
        let initial_context = context();
        let attempt = persist_attempt(&dir, &initial_context).expect("file and directory synced");
        let meta = dir.open_file(&attempt_name()).unwrap().metadata().unwrap();
        assert_eq!(meta.uid(), 0);
        assert_eq!(meta.mode() & 0o777, 0o600);
        assert_eq!(meta.nlink(), 1);
        let mut bytes = Vec::new();
        dir.open_file(&attempt_name())
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(Sha256::digest(&bytes).as_slice(), attempt.file_sha256);
        persist_commit_intent(&dir, &attempt, &initial_context.writer).expect("intent synced");
        let intent_meta = dir.open_file(&intent_name()).unwrap().metadata().unwrap();
        assert_eq!(intent_meta.uid(), 0);
        assert_eq!(intent_meta.mode() & 0o777, 0o600);

        let mut failed = context();
        failed.database = "learning_restore_c4_65a2db86-839c-4bf0-90e0-375997826c85".into();
        struct FailDir<'a>(&'a BackupDir);
        impl JournalIo for FailDir<'_> {
            type File = std::fs::File;
            type Reader = std::fs::File;
            fn identity(&self) -> io::Result<(u64, u64)> {
                self.0.identity()
            }
            fn kind(&self, name: &str) -> io::Result<BackupEntryKind> {
                self.0.kind(name)
            }
            fn create_file(&self, name: &str) -> io::Result<Self::File> {
                self.0.create_file(name)
            }
            fn open_file(&self, name: &str) -> io::Result<Self::Reader> {
                self.0.open_file(name)
            }
            fn sync_file(&self, file: &Self::File) -> io::Result<()> {
                file.sync_all()
            }
            fn sync_dir(&self) -> io::Result<()> {
                Err(io::Error::other("injected parent sync failure"))
            }
        }
        assert!(matches!(
            persist_attempt_with_io(&FailDir(&dir), &failed),
            Err(ImportFailure::Journal)
        ));
        let failed_name = format!("{}.restore.attempt", failed.database);
        assert_eq!(dir.kind(&failed_name).unwrap(), BackupEntryKind::File);
        assert!(matches!(
            persist_attempt(&dir, &failed),
            Err(ImportFailure::Journal)
        ));

        let mut linked = context();
        linked.database = "learning_restore_c4_98546cf3-9dfb-443f-a86e-107ef73e8a69".into();
        let linked_name = format!("{}.restore.attempt", linked.database);
        std::os::unix::fs::symlink("outside", child_path.join(&linked_name)).unwrap();
        assert!(matches!(
            persist_attempt(&dir, &linked),
            Err(ImportFailure::Journal)
        ));
        assert_eq!(dir.kind(&linked_name).unwrap(), BackupEntryKind::Other);
    }
}
