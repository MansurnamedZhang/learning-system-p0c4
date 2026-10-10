//! Root's fixed fresh schema lane only. No source gate, target DB, or backup
//! authority is constructed. Every input is independently joined by the outer
//! Root fixture/compile/capture receipt before this exact ignored body runs.
#[tokio::test]
#[ignore = "fresh Root enrolled source and independently pinned held target"]
async fn recovery_invalidates_unexpired_and_expired_source_tokens() {
    crate::restore_preflight::target_binding::controlled_import::full::recovery_tests::leases().await.unwrap();
}
use super::*;
use crate::FileRecord;
use std::{collections::BTreeSet, io::Write, path::Path};

const ROOT: &str = "/var/lib/knowweave-schema";

fn fixture() -> (BackupDir, BackupDir, SchemaContract) {
    let root = BackupDir::open_trusted_private_root(Path::new(ROOT))
        .expect("fresh Root-owned schema mount");
    let input = root.open_dir("capture").expect("fresh actual PG capture");
    let spool = root
        .create_dir(&format!("validation-{}", uuid::Uuid::new_v4()))
        .unwrap();
    (
        input,
        spool,
        SchemaContract::embedded().expect("reviewed actual PG18 schema contract"),
    )
}
async fn validate(
    input: &BackupDir,
    name: &str,
    spool: &BackupDir,
    contract: &SchemaContract,
) -> Result<VerifiedFullImport, BackupError> {
    let mut file = input.open_file(name)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    let mut size = 0;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        if size > super::spool::MAX_DUMP {
            return Err(BackupError::Capacity("test archive"));
        }
        hash.update(&buffer[..n]);
    }
    let record = FileRecord {
        path: "database.dump".into(),
        size,
        sha256: hex::encode(hash.finalize()),
    };
    let frozen = freeze_full_dump(&mut file, &record, spool)?;
    validate_full_dump(frozen, contract, spool).await
}
fn proof(root: &BackupDir, name: &str, value: serde_json::Value) {
    let mut file = root.create_file(name).unwrap();
    file.write_all(&serde_json::to_vec(&value).unwrap())
        .unwrap();
    file.sync_all().unwrap();
    root.sync().unwrap();
}
#[tokio::test]
#[ignore = "fixed fresh Root PG18 schema lane; no restore authority"]
async fn real_full_schema_contract() {
    let (input, spool, contract) = fixture();
    let observer = super::spool::native_observation::FirstVersion::arm().await;
    let baseline = validate(&input, "database.dump", &spool, &contract).await;
    let observed = observer.drain();
    let diagnostic = BackupDir::open_trusted_private_root(Path::new(ROOT))
        .map_err(BackupError::from)
        .and_then(|root| super::spool::native_observation::persist_first(&root, observed.as_ref()));
    let verified = super::spool::native_observation::retain_result(baseline, diagnostic).unwrap();
    let tables: BTreeSet<_> = verified
        .segments()
        .filter_map(|s| match s.kind() {
            SegmentKind::Copy { table } => Some(table.to_owned()),
            _ => None,
        })
        .collect();
    assert_eq!(tables, contract.table_names().map(String::from).collect());
    assert_eq!(tables.len(), 64);
    assert_eq!(
        verified
            .segments()
            .filter(|s| s.kind() == SegmentKind::Schema)
            .count(),
        2
    );
    let root = BackupDir::open_trusted_private_root(Path::new(ROOT)).unwrap();
    let native = root.create_dir("native-supervisor").unwrap();
    let faults = super::spool::native_fault_cases(native).await.unwrap();
    assert_eq!(faults.len(), 5);
    proof(
        &root,
        "full-schema-contract.json",
        serde_json::json!({"classification":"REAL_PG18_SCHEMA_CONTENT_NOT_RESTORE","tables":64,"decoded_sha256":verified.decoded_sha256(),"native_cases":faults}),
    );
}
#[tokio::test]
#[ignore = "fixed fresh Root altered-schema fixture; no restore authority"]
async fn real_full_schema_altered() {
    let (input, spool, contract) = fixture();
    validate(&input, "database.dump", &spool, &contract)
        .await
        .unwrap();
    for file in ["bad-extra.dump", "bad-function.dump", "bad-acl.dump"] {
        assert!(
            validate(&input, file, &spool, &contract).await.is_err(),
            "accepted altered schema: {file}"
        );
    }
    let root = BackupDir::open_trusted_private_root(Path::new(ROOT)).unwrap();
    proof(
        &root,
        "full-schema-altered.json",
        serde_json::json!({"classification":"REAL_PG18_SCHEMA_CONTENT_NOT_RESTORE","rejected":["bad-extra.dump","bad-function.dump","bad-acl.dump"],"target_writes":0}),
    );
}
#[tokio::test]
#[ignore = "fixed fresh Root edge-data fixture; no restore authority"]
async fn real_full_copy_edge_data() {
    let (input, spool, contract) = fixture();
    let verified = validate(&input, "edge.dump", &spool, &contract)
        .await
        .unwrap();
    let mut expected = Vec::new();
    input
        .open_file("edge-copy.bin")
        .unwrap()
        .take(8193)
        .read_to_end(&mut expected)
        .unwrap();
    assert!(expected.len() <= 8192);
    let mut frame = Vec::new();
    let segment = verified
        .segments()
        .find(|s| {
            s.kind()
                == SegmentKind::Copy {
                    table: "composition_revision",
                }
        })
        .expect("edge table");
    segment
        .reader()
        .take(16385)
        .read_to_end(&mut frame)
        .unwrap();
    assert!(frame.len() <= 16384);
    let header = b" FROM stdin;\n";
    let start = frame
        .windows(header.len())
        .position(|b| b == header)
        .unwrap()
        + header.len();
    assert_eq!(
        &frame[start..],
        [expected.as_slice(), b"\\.\n\n\n"].concat()
    );
    for required in [
        "中文".as_bytes(),
        b"\t\\N\t",
        b"\\\\N",
        b"\\n",
        b"\\t",
        b"\\\\connect",
        b"COMMIT;",
    ] {
        assert!(
            expected.windows(required.len()).any(|b| b == required),
            "missing required actual PG edge data"
        );
    }
    let root = BackupDir::open_trusted_private_root(Path::new(ROOT)).unwrap();
    proof(
        &root,
        "full-copy-edge-data.json",
        serde_json::json!({"classification":"REAL_PG18_SCHEMA_CONTENT_NOT_RESTORE","decoded_sha256":verified.decoded_sha256(),"source_copy_sha256":crate::digest(&expected),"unchanged_payload_bytes":expected.len()}),
    );
}

/// Requires both the current source capture fixture and a separately born,
/// pinned three-role target. This body never accepts historical sealed input.
#[tokio::test]
#[ignore = "fresh enrolled source capture and separately pinned full target required"]
async fn full_import_preserves_one_writer_transaction_and_exact_endpoint() {
    crate::restore_preflight::target_binding::controlled_import::full::real_full_import_rehearsal(
        crate::restore_preflight::target_binding::controlled_import::full::FullFault::Success,
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "fresh source capture and independently born full target required"]
async fn full_import_eof_before_commit_leaves_zero_objects() {
    use crate::restore_preflight::target_binding::controlled_import::full::{
        FullFault, real_full_import_rehearsal,
    };
    real_full_import_rehearsal(FullFault::PrecommitEof)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "fresh source capture and independently born full target required"]
async fn full_import_cancel_retains_guards_until_isolation() {
    use crate::restore_preflight::target_binding::controlled_import::full::{
        FullFault, real_full_import_rehearsal,
    };
    real_full_import_rehearsal(FullFault::Cancel).await.unwrap();
}

#[tokio::test]
#[ignore = "fresh source capture and independently born full target required"]
async fn full_import_commit_unknown_is_unusable_and_no_replay() {
    use crate::restore_preflight::target_binding::controlled_import::full::{
        FullFault, real_full_import_rehearsal,
    };
    real_full_import_rehearsal(FullFault::CommitUnknown)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "fresh source capture and independently born full target required"]
async fn full_import_wrong_endpoint_writes_nothing() {
    use crate::restore_preflight::target_binding::controlled_import::full::{
        FullFault, real_full_import_rehearsal,
    };
    real_full_import_rehearsal(FullFault::WrongEndpoint)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "fresh source capture and independently born full target required"]
async fn full_import_bad_dump_dirty_target_or_bad_role_writes_nothing() {
    use crate::restore_preflight::target_binding::controlled_import::full::{
        FullFault, real_full_import_rehearsal,
    };
    real_full_import_rehearsal(FullFault::BadInput)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "fresh source capture and independently born full target required"]
async fn full_assets_corruption_blocks_recovery_receipt() {
    use crate::restore_preflight::target_binding::controlled_import::full::{
        FullFault, real_full_import_rehearsal,
    };
    real_full_import_rehearsal(FullFault::AssetCorrupt)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "fresh source and separately born dedicated negative target"]
async fn full_import_bad_role_writes_nothing() {
    use crate::restore_preflight::target_binding::controlled_import::full::{
        FullFault, real_full_import_rehearsal,
    };
    real_full_import_rehearsal(FullFault::BadRole)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "fresh source and separately born dedicated negative target"]
async fn full_import_dirty_target_writes_nothing() {
    use crate::restore_preflight::target_binding::controlled_import::full::{
        FullFault, real_full_import_rehearsal,
    };
    real_full_import_rehearsal(FullFault::DirtyTarget)
        .await
        .unwrap();
}
