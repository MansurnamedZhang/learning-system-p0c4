//! Independent build-pinned identity of the source control root.
use crate::{BackupError, maintenance::valid_c4_database};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::{Uuid, Variant, Version};

const MAX_BINDING_BYTES: usize = 4096;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceBinding {
    format_version: u32,
    capability: String,
    binding_id: String,
    control_path: String,
    control_dev: u64,
    control_ino: u64,
    database: String,
    database_oid: u64,
    system_identifier: String,
}

fn canonical_path(path: &str) -> bool {
    path.starts_with('/')
        && path != "/"
        && !path.contains('\0')
        && path[1..]
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn private_binding_file(uid: u32, mode: u32, links: u64, length: u64) -> bool {
    uid == 0
        && mode & 0o7777 == 0o600
        && links == 1
        && length > 0
        && length <= MAX_BINDING_BYTES as u64
}

fn parse_pinned(bytes: &[u8], pin: Option<&str>) -> Result<SourceBinding, BackupError> {
    let pin = pin.ok_or(BackupError::Invalid(
        "source binding compile-time pin missing",
    ))?;
    if pin.len() != 64
        || !pin
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || bytes.is_empty()
        || bytes.len() > MAX_BINDING_BYTES
        || format!("{:x}", Sha256::digest(bytes)) != pin
    {
        return Err(BackupError::Invalid("source binding digest or length"));
    }
    let record: SourceBinding = serde_json::from_slice(bytes)?;
    // Value object serialization sorts keys, independently of struct order.
    if serde_json::to_vec(&serde_json::to_value(&record)?)? != bytes {
        return Err(BackupError::Invalid("source binding noncanonical bytes"));
    }
    let id = Uuid::parse_str(&record.binding_id)
        .map_err(|_| BackupError::Invalid("source binding UUID"))?;
    let system_identifier = record
        .system_identifier
        .parse::<u64>()
        .map_err(|_| BackupError::Invalid("source binding system identifier"))?;
    if record.format_version != 1
        || record.capability != "source_control_binding_v1"
        || id.to_string() != record.binding_id
        || id.get_variant() != Variant::RFC4122
        || id.get_version() != Some(Version::Random)
        || !canonical_path(&record.control_path)
        || record.control_dev == 0
        || record.control_ino == 0
        || !valid_c4_database(&record.database)
        || record.database_oid == 0
        || record.database_oid > u64::from(u32::MAX)
        || system_identifier == 0
        || system_identifier.to_string() != record.system_identifier
    {
        return Err(BackupError::Invalid("source binding identity fields"));
    }
    Ok(record)
}

#[cfg(target_os = "linux")]
use super::admission::SourceAdmission;
#[cfg(target_os = "linux")]
use learning_assets::backup_fs::BackupDir;
#[cfg(target_os = "linux")]
use std::{io::Read, os::unix::fs::MetadataExt, path::Path};

#[cfg(target_os = "linux")]
fn read_control_root(path: &Path) -> Result<(BackupDir, Vec<u8>), BackupError> {
    let path_text = path
        .to_str()
        .ok_or(BackupError::Invalid("source binding control path UTF-8"))?;
    if !canonical_path(path_text) {
        return Err(BackupError::Invalid("source binding control path"));
    }
    let root = BackupDir::open_trusted_private_root(path)?;
    let file = root.open_file("source-binding.json")?;
    let metadata = file.metadata()?;
    if !private_binding_file(
        metadata.uid(),
        metadata.mode(),
        metadata.nlink(),
        metadata.len(),
    ) {
        return Err(BackupError::Invalid("source binding private file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BINDING_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BINDING_BYTES {
        return Err(BackupError::Invalid("source binding file length"));
    }
    Ok((root, bytes))
}

#[cfg(target_os = "linux")]
fn verify_root_identity(
    record: &SourceBinding,
    path: &Path,
    root: &BackupDir,
) -> Result<(), BackupError> {
    if path.to_str() != Some(record.control_path.as_str())
        || root.identity()? != (record.control_dev, record.control_ino)
    {
        return Err(BackupError::Invalid("source binding control root identity"));
    }
    Ok(())
}

/// All SQL identity observations use the session that already owns admission.
/// The caller retains this root for every scan and journal/recovery write.
#[cfg(target_os = "linux")]
pub(super) async fn admit_control_root(
    admission: &mut SourceAdmission,
    path: &Path,
) -> Result<BackupDir, BackupError> {
    let (root, bytes) = read_control_root(path)?;
    let record = parse_pinned(
        &bytes,
        option_env!("KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256"),
    )?;
    verify_root_identity(&record, path, &root)?;
    let (database, database_oid, system_identifier): (String, i64, String) = sqlx::query_as(
        "SELECT current_database()::text, oid::bigint, (pg_catalog.pg_control_system()).system_identifier::text FROM pg_catalog.pg_database WHERE datname=current_database()",
    )
    .fetch_one(admission.connection())
    .await?;
    if database != record.database
        || database != admission.database()
        || database_oid <= 0
        || database_oid as u64 != record.database_oid
        || system_identifier != record.system_identifier
    {
        return Err(BackupError::Invalid("source binding database identity"));
    }
    Ok(root)
}

/// Future root-driver fixtures must independently issue/pin before compilation;
/// this helper only consumes that authority, never creates or registers it.
#[cfg(all(test, target_os = "linux"))]
pub(super) fn preissued_fixture(root_path: &Path) -> (BackupDir, Vec<u8>) {
    let (root, bytes) =
        read_control_root(root_path).expect("independently preissued binding required");
    let record = parse_pinned(
        &bytes,
        option_env!("KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256"),
    )
    .expect("fixture requires independent binding pin before compilation");
    verify_root_identity(&record, root_path, &root).expect("preissued fixture control identity");
    assert_eq!(
        root.list().unwrap(),
        vec!["source-binding.json"],
        "fresh control contains only independent binding"
    );
    (root, bytes)
}

#[cfg(all(test, target_os = "linux"))]
#[derive(Debug, PartialEq, Eq)]
pub(super) struct FixtureBindingInventory {
    root: (u32, u32, u64, u64),
    entries: Vec<String>,
    file: (u32, u32, u64, u64, u64),
    bytes: Vec<u8>,
}

#[cfg(all(test, target_os = "linux"))]
pub(super) fn fixture_inventory(path: &Path) -> Result<FixtureBindingInventory, BackupError> {
    let (root, bytes) = read_control_root(path)?;
    let root_meta = std::fs::symlink_metadata(path)?;
    let file_meta = root.open_file("source-binding.json")?.metadata()?;
    Ok(FixtureBindingInventory {
        root: (
            root_meta.uid(),
            root_meta.mode(),
            root_meta.dev(),
            root_meta.ino(),
        ),
        entries: root.list()?,
        file: (
            file_meta.uid(),
            file_meta.mode(),
            file_meta.dev(),
            file_meta.ino(),
            file_meta.nlink(),
        ),
        bytes,
    })
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    // Literal, independently specified canonical v1 record; test pins are pure
    // verifier inputs and do not replace the public entry's compiled authority.
    const CANONICAL: &str = concat!(
        "{\"binding_id\":\"11111111-1111-4111-8111-111111111111\",",
        "\"capability\":\"source_control_binding_v1\",\"control_dev\":1,",
        "\"control_ino\":2,\"control_path\":\"/var/lib/knowweave-source/control\",",
        "\"database\":\"learning_backup_c4_task3_22222222-2222-4222-8222-222222222222\",",
        "\"database_oid\":3,\"format_version\":1,\"system_identifier\":\"4\"}"
    );

    fn verify(bytes: &[u8]) -> Result<SourceBinding, BackupError> {
        let pin = format!("{:x}", Sha256::digest(bytes));
        parse_pinned(bytes, Some(&pin))
    }

    #[test]
    fn accepts_independently_specified_canonical_record() {
        let record = verify(CANONICAL.as_bytes()).unwrap();
        assert_eq!(record.database_oid, 3);
        assert_eq!(record.system_identifier, "4");
    }

    #[test]
    fn refuses_missing_malformed_or_mismatched_pin_and_size() {
        assert!(parse_pinned(CANONICAL.as_bytes(), None).is_err());
        for pin in ["", "a", &"A".repeat(64), &"0".repeat(64)] {
            assert!(parse_pinned(CANONICAL.as_bytes(), Some(pin)).is_err());
        }
        assert!(verify(&[]).is_err());
        assert!(verify(&vec![b' '; MAX_BINDING_BYTES + 1]).is_err());
    }

    #[test]
    fn refuses_non_root_wide_mode_linked_or_unbounded_file_metadata() {
        assert!(private_binding_file(0, 0o100600, 1, 4096));
        for (uid, mode, links, length) in [
            (1000, 0o600, 1, 1),
            (0, 0o640, 1, 1),
            (0, 0o660, 1, 1),
            (0, 0o4600, 1, 1),
            (0, 0o600, 2, 1),
            (0, 0o600, 1, 0),
            (0, 0o600, 1, 4097),
        ] {
            assert!(!private_binding_file(uid, mode, links, length));
        }
    }

    #[test]
    fn refuses_noncanonical_json_and_unknown_or_duplicate_fields() {
        let reordered = CANONICAL.replacen(
            "{\"binding_id\":\"11111111-1111-4111-8111-111111111111\",\"capability\":\"source_control_binding_v1\",",
            "{\"capability\":\"source_control_binding_v1\",\"binding_id\":\"11111111-1111-4111-8111-111111111111\",",
            1,
        );
        for bytes in [
            format!("{CANONICAL}\n"),
            CANONICAL.replacen(':', ": ", 1),
            reordered,
            CANONICAL.replacen('{', "{\"extra\":0,", 1),
            CANONICAL.replacen('{', "{\"format_version\":1,", 1),
            CANONICAL.replace("\"format_version\":1", "\"format_version\":1.0"),
            CANONICAL.replace("\"control_dev\":1,", ""),
        ] {
            assert!(verify(bytes.as_bytes()).is_err());
        }
    }

    #[test]
    fn refuses_unsupported_or_noncanonical_identity_fields() {
        let cases = [
            ("\"format_version\":1", "\"format_version\":2"),
            ("source_control_binding_v1", "source_control_binding_v2"),
            (
                "11111111-1111-4111-8111-111111111111",
                "11111111-1111-1111-8111-111111111111",
            ),
            (
                "11111111-1111-4111-8111-111111111111",
                "11111111-1111-4111-7111-111111111111",
            ),
            ("\"control_dev\":1", "\"control_dev\":0"),
            ("\"control_ino\":2", "\"control_ino\":0"),
            ("\"database_oid\":3", "\"database_oid\":0"),
            ("\"database_oid\":3", "\"database_oid\":4294967296"),
            ("\"control_dev\":1", "\"control_dev\":18446744073709551616"),
            ("\"system_identifier\":\"4\"", "\"system_identifier\":\"0\""),
            (
                "\"system_identifier\":\"4\"",
                "\"system_identifier\":\"04\"",
            ),
            (
                "\"system_identifier\":\"4\"",
                "\"system_identifier\":\"18446744073709551616\"",
            ),
            (
                "learning_backup_c4_task3_22222222-2222-4222-8222-222222222222",
                "learning",
            ),
        ];
        for (from, to) in cases {
            assert!(verify(CANONICAL.replace(from, to).as_bytes()).is_err());
        }
        for path in [
            "relative",
            "/",
            "/var//control",
            "/var/./control",
            "/var/../control",
            "/var/control/",
        ] {
            assert!(
                verify(
                    CANONICAL
                        .replace("/var/lib/knowweave-source/control", path)
                        .as_bytes()
                )
                .is_err()
            );
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod filesystem_tests {
    use super::*;
    use crate::{GatePhase, SourceGateJournal};
    use std::{
        fs::{DirBuilder, OpenOptions, Permissions},
        io::Write,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt, symlink},
        path::PathBuf,
    };

    struct Fixture {
        parent: PathBuf,
        control: PathBuf,
    }

    impl Fixture {
        fn new() -> Option<Self> {
            // The acceptance builder's only writable, trusted test area.
            // Never use the enrolled source-control volume for library tests.
            if unsafe { libc::geteuid() } != 0 || !Path::new("/target").is_dir() {
                return None;
            }
            let parent = Path::new("/target").join(format!("c4-binding-fs-{}", Uuid::new_v4()));
            DirBuilder::new().mode(0o700).create(&parent).unwrap();
            let control = parent.join("control");
            DirBuilder::new().mode(0o700).create(&control).unwrap();
            BackupDir::open_trusted_private_root(&control).unwrap();
            Some(Self { parent, control })
        }

        fn issue_test_record(&self) -> Vec<u8> {
            let root = BackupDir::open_trusted_private_root(&self.control).unwrap();
            let (dev, ino) = root.identity().unwrap();
            let bytes = serde_json::to_vec(&serde_json::json!({
                "format_version":1,"capability":"source_control_binding_v1",
                "binding_id":"11111111-1111-4111-8111-111111111111",
                "control_path":self.control.to_str().unwrap(),"control_dev":dev,"control_ino":ino,
                "database":"learning_backup_c4_task3_22222222-2222-4222-8222-222222222222",
                "database_oid":3,"system_identifier":"4"
            }))
            .unwrap();
            let mut file = root.create_file("source-binding.json").unwrap();
            file.write_all(&bytes).unwrap();
            file.sync_all().unwrap();
            bytes
        }

        fn test_record(bytes: &[u8]) -> SourceBinding {
            // Pure verifier injection for filesystem unit tests only. The
            // four live public gates use exclusively the independent build pin.
            let pin = format!("{:x}", Sha256::digest(bytes));
            parse_pinned(bytes, Some(&pin)).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            assert_eq!(self.parent.parent(), Some(Path::new("/target")));
            assert!(
                self.parent
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("c4-binding-fs-")
            );
            std::fs::remove_dir_all(&self.parent).unwrap();
        }
    }

    #[test]
    fn refuses_missing_linked_wide_oversized_or_non_file_binding() {
        let Some(fixture) = Fixture::new() else {
            return;
        };
        assert!(read_control_root(&fixture.control).is_err());
        let bytes = fixture.issue_test_record();
        let binding_path = fixture.control.join("source-binding.json");
        let (root, actual) = read_control_root(&fixture.control).unwrap();
        assert_eq!(actual, bytes);
        verify_root_identity(&Fixture::test_record(&bytes), &fixture.control, &root).unwrap();
        std::fs::set_permissions(&binding_path, Permissions::from_mode(0o640)).unwrap();
        assert!(read_control_root(&fixture.control).is_err());
        std::fs::set_permissions(&binding_path, Permissions::from_mode(0o600)).unwrap();
        let linked = fixture.parent.join("hard-link");
        std::fs::hard_link(&binding_path, &linked).unwrap();
        assert!(read_control_root(&fixture.control).is_err());
        std::fs::remove_file(&linked).unwrap();
        std::fs::rename(&binding_path, &linked).unwrap();
        symlink(&linked, &binding_path).unwrap();
        assert!(read_control_root(&fixture.control).is_err());
        std::fs::remove_file(&binding_path).unwrap();
        DirBuilder::new().mode(0o700).create(&binding_path).unwrap();
        assert!(read_control_root(&fixture.control).is_err());
        std::fs::remove_dir(&binding_path).unwrap();
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&binding_path)
            .unwrap();
        file.write_all(&vec![b'x'; MAX_BINDING_BYTES + 1]).unwrap();
        file.sync_all().unwrap();
        assert!(read_control_root(&fixture.control).is_err());
        assert_eq!(
            BackupDir::open_trusted_private_root(&fixture.control)
                .unwrap()
                .list()
                .unwrap(),
            vec!["source-binding.json"]
        );
    }

    #[test]
    fn refuses_untrusted_ancestors_symlinks_and_wide_leaf() {
        let Some(fixture) = Fixture::new() else {
            return;
        };
        fixture.issue_test_record();
        std::fs::set_permissions(&fixture.parent, Permissions::from_mode(0o770)).unwrap();
        assert!(read_control_root(&fixture.control).is_err());
        std::fs::set_permissions(&fixture.parent, Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&fixture.control, Permissions::from_mode(0o750)).unwrap();
        assert!(read_control_root(&fixture.control).is_err());
        std::fs::set_permissions(&fixture.control, Permissions::from_mode(0o700)).unwrap();
        let alias = fixture.parent.join("alias");
        symlink(&fixture.control, &alias).unwrap();
        assert!(read_control_root(&alias).is_err());
        assert!(read_control_root(&fixture.parent.join("control/.")).is_err());
        assert!(read_control_root(&fixture.parent.join("control/../control")).is_err());
        assert_eq!(
            BackupDir::open_trusted_private_root(&fixture.control)
                .unwrap()
                .list()
                .unwrap(),
            vec!["source-binding.json"]
        );
    }

    #[test]
    fn copied_record_and_original_path_with_replaced_inode_are_refused() {
        let Some(fixture) = Fixture::new() else {
            return;
        };
        let bytes = fixture.issue_test_record();
        let record = Fixture::test_record(&bytes);
        let alternate = fixture.parent.join("alternate");
        DirBuilder::new().mode(0o700).create(&alternate).unwrap();
        let root = BackupDir::open_trusted_private_root(&alternate).unwrap();
        let mut file = root.create_file("source-binding.json").unwrap();
        file.write_all(&bytes).unwrap();
        assert!(verify_root_identity(&record, &alternate, &root).is_err());
        let moved = fixture.parent.join("old-control");
        std::fs::rename(&fixture.control, &moved).unwrap();
        DirBuilder::new()
            .mode(0o700)
            .create(&fixture.control)
            .unwrap();
        let replacement = BackupDir::open_trusted_private_root(&fixture.control).unwrap();
        let mut file = replacement.create_file("source-binding.json").unwrap();
        file.write_all(&bytes).unwrap();
        assert!(verify_root_identity(&record, &fixture.control, &replacement).is_err());
        assert_eq!(replacement.list().unwrap(), vec!["source-binding.json"]);
    }

    #[test]
    fn renamed_bound_root_retains_scan_start_and_recovery_authority() {
        let Some(fixture) = Fixture::new() else {
            return;
        };
        let bytes = fixture.issue_test_record();
        let (root, _) = read_control_root(&fixture.control).unwrap();
        verify_root_identity(&Fixture::test_record(&bytes), &fixture.control, &root).unwrap();
        let old_id = Uuid::new_v4();
        SourceGateJournal::start_in(&root, old_id).unwrap();
        let moved = fixture.parent.join("retained-control");
        std::fs::rename(&fixture.control, &moved).unwrap();
        DirBuilder::new()
            .mode(0o700)
            .create(&fixture.control)
            .unwrap();
        let replacement = BackupDir::open_trusted_private_root(&fixture.control).unwrap();
        // Mutation caught: scan or recover follows the now-replaced pathname.
        assert!(super::super::ensure_no_unfinished_journal(&root, &fixture.control).is_err());
        assert_eq!(
            SourceGateJournal::recover_in(&root, old_id)
                .unwrap()
                .record()
                .phase(),
            GatePhase::Intent
        );
        let new_id = Uuid::new_v4();
        SourceGateJournal::start_in(&root, new_id).unwrap();
        let recovery = root
            .create_dir(&format!("{old_id}.release-recovery"))
            .unwrap();
        let mut file = recovery.create_file("closed.json").unwrap();
        file.write_all(b"synthetic-handle-relative-evidence")
            .unwrap();
        file.sync_all().unwrap();
        root.sync().unwrap();
        assert!(replacement.list().unwrap().is_empty());
        assert!(
            moved
                .join(format!("{new_id}.control/intent.json"))
                .is_file()
        );
        assert_eq!(
            std::fs::read(moved.join(format!("{old_id}.release-recovery/closed.json"))).unwrap(),
            b"synthetic-handle-relative-evidence"
        );
    }
}
