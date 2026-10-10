//! Audited PG18 synthetic fixture bytes; private to the test-only import model.
use super::ImportFailure;
use sha2::{Digest, Sha256};
use std::io::Read;

const MAX_FIXTURE_BYTES: usize = 65_536;
const TEMPLATE: &[u8] =
    include_bytes!("../../../../tests/fixtures/c4-controlled-import/pg18-fixture.sql.in");
const KEY_TOKEN: &[u8] = b"{{RESTRICT_KEY}}";
// The audited source split is 670 with a 63-byte captured key. The token is
// 16 bytes, so the equivalent position in the checked-in template is 623.
const TEMPLATE_HEADER_END: usize = 623;

fn verify_toc(bytes: &[u8], database: &str) -> Result<(), ImportFailure> {
    let bad = || ImportFailure::Fixture;
    if bytes.len() > MAX_FIXTURE_BYTES || !bytes.is_ascii() || bytes.contains(&b'\r') {
        return Err(bad());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| bad())?;
    let lines: Vec<_> = text.lines().collect();
    if lines.len() != 18 || !text.ends_with('\n') || lines[0] != ";" {
        return Err(bad());
    }
    let date = lines[1]
        .strip_prefix("; Archive created at ")
        .ok_or_else(bad)?;
    if date.len() != 23
        || !date.ends_with(" UTC")
        || date.as_bytes().iter().enumerate().any(|(i, b)| match i {
            4 | 7 => *b != b'-',
            10 | 19 => *b != b' ',
            13 | 16 => *b != b':',
            20..=22 => false,
            _ => !b.is_ascii_digit(),
        })
        || lines[2] != format!(";     dbname: {database}")
    {
        return Err(bad());
    }
    let fixed = [
        ";     TOC Entries: 7",
        ";     Compression: gzip",
        ";     Dump Version: 1.16-0",
        ";     Format: CUSTOM",
        ";     Integer: 4 bytes",
        ";     Offset: 8 bytes",
        ";     Dumped from database version: 18.6 (Debian 18.6-1.pgdg12+2)",
        ";     Dumped by pg_dump version: 18.6 (Debian 18.6-1.pgdg12+2)",
        ";",
        ";",
        "; Selected TOC Entries:",
        ";",
    ];
    if lines[3..15] != fixed {
        return Err(bad());
    }
    let mut oids = Vec::new();
    for (line, catalog, suffix) in [
        (
            lines[15],
            "1259",
            "TABLE public c4_import_probe learning_admin",
        ),
        (
            lines[16],
            "0",
            "TABLE DATA public c4_import_probe learning_admin",
        ),
        (
            lines[17],
            "2606",
            "CONSTRAINT public c4_import_probe c4_import_probe_pkey learning_admin",
        ),
    ] {
        let (id, row) = line.split_once("; ").ok_or_else(bad)?;
        let (cat, row) = row.split_once(' ').ok_or_else(bad)?;
        let (oid, name) = row.split_once(' ').ok_or_else(bad)?;
        for number in [id, oid] {
            let parsed: u32 = number.parse().map_err(|_| bad())?;
            if parsed == 0 || parsed.to_string() != number {
                return Err(bad());
            }
        }
        if cat != catalog || name != suffix {
            return Err(bad());
        }
        oids.push(oid);
    }
    if oids[0] != oids[1] || oids[0] == oids[2] {
        return Err(bad());
    }
    Ok(())
}

#[allow(dead_code)]
pub(super) struct FrozenDump {
    bytes: Box<[u8]>,
    sha256: [u8; 32],
    provenance: Option<FixtureProvenance>,
}

// Constructed only by the Linux fixed producer below after full source birth,
// real capture, no-follow snapshot, TOC and golden validation. No setter exists.
struct ValidatedFreshProducer;
#[allow(dead_code)]
struct FixtureProvenance {
    batch: uuid::Uuid,
    source_batch: uuid::Uuid,
    case: &'static str,
    target_database: String,
    source_birth_sha256: [u8; 32],
    source_database: String,
    source_container: String,
    producer_image: String,
    producer_client: String,
    toc_sha256: [u8; 32],
    snapshot_len: usize,
    snapshot_sha256: [u8; 32],
    _validated: ValidatedFreshProducer,
}

#[allow(dead_code)]
impl FrozenDump {
    pub(super) fn require_provenance(
        &self,
        batch: uuid::Uuid,
        target_database: &str,
        case: &str,
    ) -> Result<(), ImportFailure> {
        let proof = self.provenance.as_ref().ok_or(ImportFailure::Fixture)?;
        if proof.batch != batch
            || proof.source_batch == batch
            || proof.case != case
            || proof.target_database != target_database
            || proof.source_database == target_database
            || !super::super::exact_id(&proof.source_container)
            || proof.producer_image != super::super::PINNED_IMAGE
            || proof.producer_client != "/usr/lib/postgresql/18/bin/pg_dump"
            || proof.snapshot_len != self.bytes.len()
            || proof.snapshot_sha256 != self.sha256
        {
            return Err(ImportFailure::Fixture);
        }
        Ok(())
    }
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(super) fn sha256(&self) -> [u8; 32] {
        self.sha256
    }

    #[cfg(target_os = "linux")]
    pub(super) fn toc_sha256(&self) -> Result<[u8; 32], ImportFailure> {
        self.provenance
            .as_ref()
            .map(|proof| proof.toc_sha256)
            .ok_or(ImportFailure::Fixture)
    }
}

#[allow(dead_code)]
pub(super) struct VerifiedFixtureSql {
    header: Box<[u8]>,
    payload: Box<[u8]>,
    raw_sha256: [u8; 32],
    transformed_sha256: [u8; 32],
}

#[allow(dead_code)]
impl VerifiedFixtureSql {
    pub(super) fn header(&self) -> &[u8] {
        &self.header
    }

    pub(super) fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub(super) fn raw_sha256(&self) -> [u8; 32] {
        self.raw_sha256
    }

    pub(super) fn transformed_sha256(&self) -> [u8; 32] {
        self.transformed_sha256
    }
}

pub(super) fn freeze_dump(
    mut reader: impl Read,
    expected_len: u64,
    expected_sha256: [u8; 32],
) -> Result<FrozenDump, ImportFailure> {
    if expected_len > MAX_FIXTURE_BYTES as u64 {
        return Err(ImportFailure::InputLimit);
    }
    let mut bytes = Vec::with_capacity(expected_len as usize);
    let mut buffer = [0u8; 8192];
    loop {
        let remaining = MAX_FIXTURE_BYTES + 1 - bytes.len();
        let read_limit = remaining.min(buffer.len());
        let read = reader
            .read(&mut buffer[..read_limit])
            .map_err(|_| ImportFailure::Io)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > MAX_FIXTURE_BYTES {
            return Err(ImportFailure::InputLimit);
        }
    }
    if bytes.len() as u64 != expected_len
        || !bytes.starts_with(b"PGDMP")
        || Sha256::digest(&bytes).as_slice() != expected_sha256
    {
        return Err(ImportFailure::Fixture);
    }
    Ok(FrozenDump {
        bytes: bytes.into_boxed_slice(),
        sha256: expected_sha256,
        provenance: None,
    })
}

// Binary pg_dump output cannot use the line protocol. This fixed-command
// collector bounds both pipes while reading and always reaps before returning.
// Its only callers run in the ownership task below or the live-case owner.
#[cfg(target_os = "linux")]
pub(super) async fn fixed_bytes(
    fixed: super::commands::FixedImportCommand,
    input: &[u8],
    writer: bool,
) -> Result<Vec<u8>, ImportFailure> {
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
    async fn limited(
        mut pipe: impl AsyncRead + Unpin,
        cap: usize,
        error: ImportFailure,
    ) -> Result<Vec<u8>, ImportFailure> {
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let n = pipe
                .read(&mut buffer)
                .await
                .map_err(|_| ImportFailure::Io)?;
            if n == 0 {
                return Ok(bytes);
            }
            if n > cap.saturating_sub(bytes.len()) {
                return Err(error);
            }
            bytes.extend_from_slice(&buffer[..n]);
        }
    }
    super::super::linux::trusted_docker_path().map_err(|_| ImportFailure::Identity)?;
    let deadline = Instant::now() + Duration::from_secs(if writer { 45 } else { 15 });
    let mut child = tokio::process::Command::new("/usr/bin/docker")
        .args(&fixed.argv()[1..])
        .env_clear()
        .env("DOCKER_HOST", "unix:///var/run/docker.sock")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| ImportFailure::Io)?;
    let mut stdin = child.stdin.take().ok_or(ImportFailure::Io)?;
    let stdout = child.stdout.take().ok_or(ImportFailure::Io)?;
    let stderr = child.stderr.take().ok_or(ImportFailure::Io)?;
    let result = tokio::time::timeout_at(deadline.into(), async {
        let (_, out, err, status) = tokio::try_join!(
            async {
                stdin
                    .write_all(input)
                    .await
                    .map_err(|_| ImportFailure::Io)?;
                stdin.shutdown().await.map_err(|_| ImportFailure::Io)?;
                drop(stdin);
                Ok::<_, ImportFailure>(())
            },
            limited(
                stdout,
                if writer { 8192 } else { 65536 },
                ImportFailure::StdoutLimit
            ),
            limited(stderr, 8192, ImportFailure::StderrLimit),
            async { child.wait().await.map_err(|_| ImportFailure::Io) }
        )?;
        if !status.success() {
            return Err(ImportFailure::Exit);
        }
        if !err.is_empty() {
            return Err(ImportFailure::Stderr);
        }
        Ok(out)
    })
    .await
    .unwrap_or(Err(ImportFailure::Deadline));
    if result.is_err() {
        let _ = child.start_kill();
        child.wait().await.map_err(|_| ImportFailure::Io)?;
    }
    result
}

#[cfg(target_os = "linux")]
pub(super) async fn capture_fresh_fixture(
    config: super::super::super::RestorePreflightConfig,
    target_database: String,
    case: &'static str,
    snapshot_root: std::path::PathBuf,
) -> Result<FrozenDump, ImportFailure> {
    // Dropping the waiter never abandons source guard/original lease/process.
    tokio::spawn(async move {
        use super::super as binding;
        use super::super::super as preflight;
        use super::commands::FixedImportCommand;
        use binding::child_attestation::{Isolation, linux_child};
        use learning_assets::backup_fs::BackupDir;
        use std::{io::Write, os::unix::fs::MetadataExt, time::{Duration, Instant}};
        let mut guard = binding::acquire_for_import_source(&config).map_err(|_| ImportFailure::Identity)?;
        let mut lease = None;
        let result = async {
            if config.expected_database == target_database { return Err(ImportFailure::Identity); }
            let batch_root=config.control_root.ancestors().nth(4).ok_or(ImportFailure::Identity)?;
            if snapshot_root!=batch_root.join("artifacts") {return Err(ImportFailure::Identity);}
            FixedImportCommand::writer(&guard.claim.container_id, &target_database)?;
            let source_batch = uuid::Uuid::parse_str(config.expected_database.strip_prefix("learning_restore_c4_")
                .ok_or(ImportFailure::Identity)?).map_err(|_| ImportFailure::Identity)?;
            let batch = uuid::Uuid::parse_str(target_database.strip_prefix("learning_restore_c4_")
                .ok_or(ImportFailure::Identity)?).map_err(|_| ImportFailure::Identity)?;
            let admin = super::live_tests::control_pool(&config, &guard)?.await?;
            let deadline = Instant::now() + Duration::from_secs(45);
            let (original, pid, oid) = tokio::time::timeout_at(deadline.into(),preflight::begin_sql_session(&admin)).await
                .map_err(|_|ImportFailure::Deadline)?.map_err(|_| ImportFailure::Session)?;
            lease = Some(original);
            let lease = lease.as_mut().ok_or(ImportFailure::Session)?;
            let control = BackupDir::open_trusted_private_root(&config.control_root).map_err(|_| ImportFailure::Identity)?;
            let assets = BackupDir::open_trusted_private_root(&config.asset_root).map_err(|_| ImportFailure::Identity)?;
            linux_child::candidate_recheck(&mut guard, lease, pid, oid, deadline).await.map_err(super::linux::failure)?;
            tokio::time::timeout_at(deadline.into(),async {
                preflight::target_facts(lease.lease_mut(), assets.list().map_err(|_| ImportFailure::Identity)?.len())
                    .await.map_err(|_| ImportFailure::Identity)?.validate().map_err(|_| ImportFailure::Identity)?;
                preflight::verify_import_source_birth(lease.lease_mut(), &config, &control, &assets)
                    .await.map_err(|_| ImportFailure::Identity)
            }).await.map_err(|_|ImportFailure::Deadline)??;
            let pin = option_env!("KNOWWEAVE_C4_IMPORT_SOURCE_BIRTH_SHA256").ok_or(ImportFailure::Identity)?;
            let source_birth_sha256: [u8;32] = hex::decode(pin).map_err(|_| ImportFailure::Identity)?
                .try_into().map_err(|_| ImportFailure::Identity)?;
            let cid = &guard.claim.container_id;
            // Existing version grammar validates pg_restore; actual producer version is
            // independently checked in the complete pg_dump TOC below.
            let mut version = FixedImportCommand::decoder(cid)?.argv()[1..12].to_vec();
            version.push("--version".into());
            linux_child::candidate_version(&version, deadline).await.map_err(super::linux::failure)?;
            let fixture = b"CREATE TABLE public.c4_import_probe (id integer NOT NULL,label text NOT NULL);\nALTER TABLE ONLY public.c4_import_probe ADD CONSTRAINT c4_import_probe_pkey PRIMARY KEY (id);\nINSERT INTO public.c4_import_probe(id,label) VALUES (1,'alpha'),(2,'beta');\n";
            if !fixed_bytes(FixedImportCommand::writer(cid, &config.expected_database)?, fixture, true).await?.is_empty() {
                return Err(ImportFailure::Fixture);
            }
            let rows = fixed_bytes(FixedImportCommand::writer(cid, &config.expected_database)?,
                b"SELECT id::text || '|' || label FROM public.c4_import_probe ORDER BY id;\n", true).await?;
            if rows != b"1|alpha\n2|beta\n" { return Err(ImportFailure::Fixture); }
            let produced = fixed_bytes(FixedImportCommand::producer(cid, &config.expected_database)?, &[], false).await?;
            let snapshots = BackupDir::open_trusted_private_root(&snapshot_root).map_err(|_| ImportFailure::Identity)?;
            let mut saved = snapshots.create_file("fixture.dump").map_err(|_| ImportFailure::Io)?;
            saved.write_all(&produced).map_err(|_| ImportFailure::Io)?;
            saved.sync_all().map_err(|_| ImportFailure::Io)?;
            snapshots.sync().map_err(|_| ImportFailure::Io)?;
            drop(saved);
            let opened = snapshots.open_file("fixture.dump").map_err(|_| ImportFailure::Identity)?;
            let meta = opened.metadata().map_err(|_| ImportFailure::Identity)?;
            if meta.uid() != 0 || meta.mode() & 0o777 != 0o600 || meta.nlink() != 1 || !meta.is_file() {
                return Err(ImportFailure::Identity);
            }
            let mut dump = freeze_dump(opened, produced.len() as u64, Sha256::digest(&produced).into())?;
            let toc = fixed_bytes(FixedImportCommand::toc(cid)?, dump.bytes(), false).await?;
            verify_toc(&toc, &config.expected_database)?;
            let decoded = fixed_bytes(FixedImportCommand::decoder(cid)?, dump.bytes(), false).await?;
            verify_fixture_sql(&decoded)?;
            for (name, bytes) in [("pg_restore-list.txt", &toc), ("decoded.sql", &decoded)] {
                let mut file = snapshots.create_file(name).map_err(|_| ImportFailure::Io)?;
                file.write_all(bytes).map_err(|_| ImportFailure::Io)?;
                file.sync_all().map_err(|_| ImportFailure::Io)?;
            }
            snapshots.sync().map_err(|_| ImportFailure::Io)?;
            linux_child::candidate_recheck(&mut guard, lease, pid, oid,
                Instant::now() + Duration::from_secs(10)).await.map_err(super::linux::failure)?;
            dump.provenance = Some(FixtureProvenance {
                batch, source_batch, case, target_database,
                source_database: config.expected_database, source_container: guard.claim.container_id.clone(),
                producer_image: binding::PINNED_IMAGE.into(), producer_client: "/usr/lib/postgresql/18/bin/pg_dump".into(),
                source_birth_sha256, toc_sha256: Sha256::digest(&toc).into(),
                snapshot_len: dump.bytes.len(), snapshot_sha256: dump.sha256, _validated: ValidatedFreshProducer,
            });
            Ok(dump)
        }.await;
        guard.child_usable.set(false);
        let isolation = linux_child::candidate_quarantine(&guard.claim).await;
        drop(lease);
        drop(guard);
        if isolation != Isolation::Stopped { return Err(ImportFailure::UnconfirmedIsolation); }
        result
    }).await.map_err(|_| ImportFailure::UnconfirmedIsolation)?
}

pub(super) fn verify_fixture_sql(decoded: &[u8]) -> Result<VerifiedFixtureSql, ImportFailure> {
    if decoded.len() > MAX_FIXTURE_BYTES {
        return Err(ImportFailure::InputLimit);
    }
    let token_positions: Vec<usize> = TEMPLATE
        .windows(KEY_TOKEN.len())
        .enumerate()
        .filter_map(|(index, window)| (window == KEY_TOKEN).then_some(index))
        .collect();
    let [first, second] = token_positions.as_slice() else {
        return Err(ImportFailure::Fixture);
    };
    let prefix = &TEMPLATE[..*first];
    let middle = &TEMPLATE[first + KEY_TOKEN.len()..*second];
    let suffix = &TEMPLATE[second + KEY_TOKEN.len()..];
    let remaining = decoded.strip_prefix(prefix).ok_or(ImportFailure::Fixture)?;
    let key_len = remaining
        .iter()
        .take_while(|byte| byte.is_ascii_alphanumeric())
        .count();
    if !(1..=128).contains(&key_len) {
        return Err(ImportFailure::Fixture);
    }
    let (key, remaining) = remaining.split_at(key_len);
    let remaining = remaining
        .strip_prefix(middle)
        .ok_or(ImportFailure::Fixture)?;
    let remaining = remaining.strip_prefix(key).ok_or(ImportFailure::Fixture)?;
    if remaining != suffix {
        return Err(ImportFailure::Fixture);
    }

    let header_end = TEMPLATE_HEADER_END - KEY_TOKEN.len() + key_len;
    let (raw_header, payload) = decoded.split_at(header_end);
    let replacements: [(&[u8], &[u8]); 4] = [
        (
            b"SET statement_timeout = 0;\n",
            b"SET LOCAL statement_timeout = '10000ms';\n",
        ),
        (
            b"SET lock_timeout = 0;\n",
            b"SET LOCAL lock_timeout = '5000ms';\n",
        ),
        (
            b"SET idle_in_transaction_session_timeout = 0;\n",
            b"SET LOCAL idle_in_transaction_session_timeout = '30000ms';\n",
        ),
        (
            b"SET transaction_timeout = 0;\n",
            b"SET LOCAL transaction_timeout = '60000ms';\n",
        ),
    ];
    let mut header = raw_header.to_vec();
    for (original, replacement) in replacements {
        let matches: Vec<usize> = header
            .windows(original.len())
            .enumerate()
            .filter_map(|(index, window)| (window == original).then_some(index))
            .collect();
        let [offset] = matches.as_slice() else {
            return Err(ImportFailure::Fixture);
        };
        header.splice(
            *offset..offset + original.len(),
            replacement.iter().copied(),
        );
    }
    let raw_sha256 = Sha256::digest(decoded).into();
    let mut transformed = header.clone();
    transformed.extend_from_slice(payload);
    let transformed_sha256 = Sha256::digest(&transformed).into();
    Ok(VerifiedFixtureSql {
        header: header.into_boxed_slice(),
        payload: payload.to_vec().into_boxed_slice(),
        raw_sha256,
        transformed_sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::fs::OpenOptions;
    use std::io::{Cursor, Error, Seek, SeekFrom, Write};

    const CAPTURE_KEY: &[u8] = b"XUiXOMjlMScOdk9ZzeQOcY8XFfEhfRYfmX6zxd5b98iPfVsmzTgJE32K1SuOOui";

    #[test]
    fn toc_requires_complete_fixed_pg18_shape_and_matching_source() {
        let db = "learning_restore_c4_2b8a1252-54d5-48aa-b176-a9586a86bea3";
        let toc = format!(
            ";\n; Archive created at 2026-10-01 00:19:15 UTC\n;     dbname: {db}\n;     TOC Entries: 7\n;     Compression: gzip\n;     Dump Version: 1.16-0\n;     Format: CUSTOM\n;     Integer: 4 bytes\n;     Offset: 8 bytes\n;     Dumped from database version: 18.6 (Debian 18.6-1.pgdg12+2)\n;     Dumped by pg_dump version: 18.6 (Debian 18.6-1.pgdg12+2)\n;\n;\n; Selected TOC Entries:\n;\n219; 1259 16387 TABLE public c4_import_probe learning_admin\n3373; 0 16387 TABLE DATA public c4_import_probe learning_admin\n3225; 2606 16395 CONSTRAINT public c4_import_probe c4_import_probe_pkey learning_admin\n"
        );
        assert_eq!(verify_toc(toc.as_bytes(), db), Ok(()));
        for changed in [
            toc.replace("TOC Entries: 7", "TOC Entries: 8"),
            toc.replace("TABLE DATA", "BLOB"),
            toc.replace("18.6", "18.5"),
            toc.replace("public c4_import_probe", "private c4_import_probe"),
            toc.replace("3373; 0 16387", "3373; 0 16388"),
            format!("{toc}4000; 0 0 BLOB - 5 learning_admin\n"),
            toc.replace("learning_admin", "postgres"),
            toc.replace(db, "other"),
        ] {
            assert_eq!(
                verify_toc(changed.as_bytes(), db),
                Err(ImportFailure::Fixture)
            );
        }
    }

    #[test]
    fn byte_checked_dump_has_no_fresh_producer_authority() {
        let bytes = b"PGDMParbitrary-caller-bytes";
        let dump = freeze_dump(Cursor::new(bytes), bytes.len() as u64, digest(bytes)).unwrap();
        assert_eq!(
            dump.require_provenance(
                uuid::Uuid::new_v4(),
                "learning_restore_c4_2b8a1252-54d5-48aa-b176-a9586a86bea3",
                "success"
            ),
            Err(ImportFailure::Fixture)
        );
    }

    fn digest(bytes: &[u8]) -> [u8; 32] {
        Sha256::digest(bytes).into()
    }

    fn captured(key: &[u8]) -> Vec<u8> {
        let mut sql = Vec::new();
        let first = TEMPLATE
            .windows(KEY_TOKEN.len())
            .position(|part| part == KEY_TOKEN)
            .unwrap();
        let second = TEMPLATE[first + KEY_TOKEN.len()..]
            .windows(KEY_TOKEN.len())
            .position(|part| part == KEY_TOKEN)
            .unwrap()
            + first
            + KEY_TOKEN.len();
        sql.extend_from_slice(&TEMPLATE[..first]);
        sql.extend_from_slice(key);
        sql.extend_from_slice(&TEMPLATE[first + KEY_TOKEN.len()..second]);
        sql.extend_from_slice(key);
        sql.extend_from_slice(&TEMPLATE[second + KEY_TOKEN.len()..]);
        sql
    }

    #[test]
    fn snapshot_stays_fixed_after_same_inode_rewrite() {
        let original = b"PGDMPoriginal-synthetic-dump";
        let path = std::env::temp_dir().join(format!("p0c4-snapshot-{}", uuid::Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.write_all(original).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let frozen = freeze_dump(&mut file, original.len() as u64, digest(original)).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(b"PGDMPchanged!-synthetic-dump").unwrap();
        file.sync_all().unwrap();
        assert_eq!(frozen.bytes(), original);
        assert_eq!(frozen.sha256(), digest(original));
        drop(file);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn fixture_budget_accepts_65536_rejects_65537() {
        let mut allowed = vec![b'a'; 65_536];
        allowed[..5].copy_from_slice(b"PGDMP");
        assert_eq!(
            freeze_dump(Cursor::new(&allowed), 65_536, digest(&allowed))
                .unwrap()
                .bytes()
                .len(),
            65_536
        );
        allowed.push(b'a');
        assert!(matches!(
            freeze_dump(Cursor::new(&allowed), 65_537, digest(&allowed)),
            Err(ImportFailure::InputLimit)
        ));
        assert!(matches!(
            freeze_dump(Cursor::new(&allowed), 65_536, digest(&allowed)),
            Err(ImportFailure::InputLimit)
        ));
    }

    #[test]
    fn snapshot_rejects_short_invalid_and_io_reader() {
        let bytes = b"PGDMPsmall";
        assert!(matches!(
            freeze_dump(Cursor::new(bytes), 11, digest(bytes)),
            Err(ImportFailure::Fixture)
        ));
        assert!(matches!(
            freeze_dump(Cursor::new(b"WRONGsmall"), 10, digest(b"WRONGsmall")),
            Err(ImportFailure::Fixture)
        ));
        assert!(matches!(
            freeze_dump(Cursor::new(bytes), 10, [0; 32]),
            Err(ImportFailure::Fixture)
        ));
        struct FailingReader;
        impl Read for FailingReader {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(Error::other("redacted test error"))
            }
        }
        assert!(matches!(
            freeze_dump(FailingReader, 10, digest(bytes)),
            Err(ImportFailure::Io)
        ));
    }

    #[test]
    fn audited_golden_accepts_actual_and_key_length_bounds() {
        let original = captured(CAPTURE_KEY);
        assert_eq!(original.len(), 1308);
        assert_eq!(
            hex::encode(digest(&original)),
            "e2c252bfa5d44c5ed9133dcdb7fd8344e0a2471489d0ee410edb6c09baabd44a"
        );
        let verified = verify_fixture_sql(&original).unwrap();
        assert_eq!(verified.raw_sha256(), digest(&original));
        assert_eq!(verified.payload(), &original[670..]);
        for key in [vec![b'A'], vec![b'z'; 128]] {
            let sql = captured(&key);
            let verified = verify_fixture_sql(&sql).unwrap();
            assert_eq!(verified.payload(), &sql[670 + key.len() - 63..]);
        }
        for key in [vec![], vec![b'B'; 129], vec![b'_'], vec![0xff]] {
            assert!(matches!(
                verify_fixture_sql(&captured(&key)),
                Err(ImportFailure::Fixture)
            ));
        }
    }

    #[test]
    fn golden_rejects_key_mismatch_multiple_markers_and_non_key_changes() {
        let original = captured(CAPTURE_KEY);
        let mut mismatch = original.clone();
        mismatch[1243] = b'Z';
        assert!(matches!(
            verify_fixture_sql(&mismatch),
            Err(ImportFailure::Fixture)
        ));
        for needle in [
            b"-- PostgreSQL".as_slice(),
            b"1\talpha".as_slice(),
            b"-- Name:".as_slice(),
        ] {
            let mut changed = original.clone();
            let offset = changed
                .windows(needle.len())
                .position(|part| part == needle)
                .unwrap();
            changed[offset] = b'!';
            assert!(matches!(
                verify_fixture_sql(&changed),
                Err(ImportFailure::Fixture)
            ));
        }
        let mut duplicated = original.clone();
        duplicated.extend_from_slice(b"\\restrict extra\n");
        assert!(matches!(
            verify_fixture_sql(&duplicated),
            Err(ImportFailure::Fixture)
        ));
        assert!(matches!(
            verify_fixture_sql(&original[..original.len() - 1]),
            Err(ImportFailure::Fixture)
        ));
        assert!(matches!(
            verify_fixture_sql(&[original.as_slice(), b"\n"].concat()),
            Err(ImportFailure::Fixture)
        ));
        assert!(matches!(
            verify_fixture_sql(&original.replace_lf_with_crlf()),
            Err(ImportFailure::Fixture)
        ));
    }

    #[test]
    fn golden_rejects_transaction_reconnect_lo_and_early_unrestrict() {
        let original = captured(CAPTURE_KEY);
        for extra in [
            b"COMMIT;\n".as_slice(),
            b"\\connect other\n".as_slice(),
            b"SELECT lo_create(1);\n".as_slice(),
            b"\\unrestrict early\n".as_slice(),
        ] {
            let mut changed = original.clone();
            changed.splice(741..741, extra.iter().copied());
            assert!(matches!(
                verify_fixture_sql(&changed),
                Err(ImportFailure::Fixture)
            ));
        }
    }

    #[test]
    fn header_is_ddl_free_and_timeouts_are_local() {
        let original = captured(CAPTURE_KEY);
        let verified = verify_fixture_sql(&original).unwrap();
        let header = std::str::from_utf8(verified.header()).unwrap();
        let payload = std::str::from_utf8(verified.payload()).unwrap();
        assert!(!header.contains("CREATE TABLE"));
        assert!(!header.contains("COPY public"));
        assert!(header.ends_with("SET default_table_access_method = heap;\n\n"));
        assert!(payload.starts_with("--\n-- Name: c4_import_probe; Type: TABLE"));
        assert!(payload.contains("CREATE TABLE public.c4_import_probe"));
        assert!(payload.contains(
            "COPY public.c4_import_probe (id, label) FROM stdin;\n1\talpha\n2\tbeta\n\\.\n"
        ));
        assert!(!payload.contains("COMMIT"));
        for expected in [
            "SET LOCAL statement_timeout = '10000ms';\n",
            "SET LOCAL lock_timeout = '5000ms';\n",
            "SET LOCAL idle_in_transaction_session_timeout = '30000ms';\n",
            "SET LOCAL transaction_timeout = '60000ms';\n",
        ] {
            assert!(header.contains(expected), "missing {expected}");
        }
        assert!(!header.contains("SET statement_timeout = 0;"));
        assert_eq!(
            header
                .matches("SELECT pg_catalog.set_config('search_path', '', false);\n")
                .count(),
            1
        );
        let transformed = [verified.header(), verified.payload()].concat();
        assert_eq!(verified.transformed_sha256(), digest(&transformed));
        assert_eq!(
            hex::encode(verified.transformed_sha256()),
            "d52fadf2e5d841b57166cd9d6f281034b3b16264b66a677b55aeaab8fcbce138"
        );
    }

    trait CrLf {
        fn replace_lf_with_crlf(&self) -> Vec<u8>;
    }
    impl CrLf for [u8] {
        fn replace_lf_with_crlf(&self) -> Vec<u8> {
            let mut result = Vec::new();
            for byte in self {
                if *byte == b'\n' {
                    result.push(b'\r');
                }
                result.push(*byte);
            }
            result
        }
    }
}
