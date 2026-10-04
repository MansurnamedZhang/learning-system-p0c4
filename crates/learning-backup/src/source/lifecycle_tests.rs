use super::lifecycle::*;
use crate::{GatePhase, SourceGateRecord};
use uuid::Uuid;
const ID: &str = "11111111-1111-4111-8111-111111111111";
const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
fn id() -> Uuid {
    Uuid::parse_str(ID).unwrap()
}
fn record() -> SourceGateRecord {
    SourceGateRecord::new(id())
}
fn ready() -> Vec<u8> {
    serde_json::to_vec(&Ready::new(id(), A, &record()).unwrap()).unwrap()
}

#[test]
fn seven_phase_literal_legacy_bytes_allow_unequal_historical_digests() {
    let phases = [
        (GatePhase::Intent, "intent"),
        (GatePhase::Closed, "closed"),
        (GatePhase::Drained, "drained"),
        (GatePhase::DumpAndIndexDurable, "dump_and_index_durable"),
        (GatePhase::PinsDurable, "pins_durable"),
        (GatePhase::ReleaseReady, "release_ready"),
        (GatePhase::Released, "released"),
    ];
    let mut value = record();
    for (n, (phase, name)) in phases.into_iter().enumerate() {
        if n > 0 {
            value
                .advance(
                    phase,
                    match n {
                        3 => Some(A),
                        4 => Some(B),
                        _ => None,
                    },
                )
                .unwrap();
        }
        let source = if n >= 3 {
            format!("\"{A}\"")
        } else {
            "null".into()
        };
        let pins = if n >= 4 {
            format!("\"{B}\"")
        } else {
            "null".into()
        };
        let bytes = format!(
            "{{\"backup_id\":\"{ID}\",\"phase\":\"{name}\",\"dump_and_index_sha256\":{source},\"pins_sha256\":{pins}}}"
        );
        let decoded: SourceGateRecord = serde_json::from_str(&bytes).unwrap();
        decoded.validate_for(id()).unwrap();
        assert_eq!(serde_json::to_vec(&value).unwrap(), bytes.as_bytes());
        assert_eq!(decoded, value);
    }
}
#[test]
fn finish_rejects_early_phases_and_any_abandonment_decision() {
    for phase in [GatePhase::Intent, GatePhase::Closed, GatePhase::Drained] {
        assert!(!finish_eligible(phase, false));
    }
    for phase in [
        GatePhase::DumpAndIndexDurable,
        GatePhase::PinsDurable,
        GatePhase::ReleaseReady,
        GatePhase::Released,
    ] {
        assert!(finish_eligible(phase, false));
        assert!(!finish_eligible(phase, true));
    }
}
#[test]
fn ready_exact_bytes_and_terminal_hash_link_are_authority() {
    let bytes = ready();
    let journal_sha = "ed179aebbf1556d50d4497c13051a3eec7597f434d7a6ed8be99d512d82c04b0";
    let expected = format!(
        "{{\"format_version\":1,\"capability\":\"source_abandonment_v1\",\"backup_id\":\"{ID}\",\"action\":\"abandon\",\"source_binding_sha256\":\"{A}\",\"journal_phase\":\"intent\",\"journal_record_sha256\":\"{journal_sha}\",\"retained_artifacts\":\"keep_all\",\"state\":\"release_ready\"}}"
    );
    assert_eq!(bytes, expected.as_bytes());
    parse_ready(&bytes, id(), A, &record()).unwrap();
    let terminal = serde_json::to_vec(&Terminal::new(id(), A, &bytes)).unwrap();
    let expected = format!(
        "{{\"format_version\":1,\"capability\":\"source_abandonment_v1\",\"backup_id\":\"{ID}\",\"action\":\"abandon\",\"source_binding_sha256\":\"{A}\",\"ready_sha256\":\"{}\",\"state\":\"abandoned\"}}",
        "f3b0e287617fab66ecf3cdecc0b12a6b6a93e8c839b9e3b7ef1f7653486cc9e6"
    );
    assert_eq!(terminal, expected.as_bytes());
    parse_terminal(&terminal, id(), A, &bytes).unwrap();
    assert!(parse_terminal(&terminal, id(), B, &bytes).is_err());
    assert!(parse_terminal(&terminal, id(), A, b"changed").is_err());
}
#[test]
fn ready_rejects_noncanonical_unknown_duplicate_and_foreign_authority() {
    let bytes = String::from_utf8(ready()).unwrap();
    for bad in [
        format!("{bytes}\n"),
        bytes.replacen(':', ": ", 1),
        bytes.replacen('{', "{\"unknown\":0,", 1),
        bytes.replacen('{', "{\"format_version\":1,", 1),
        bytes.replace("keep_all", "delete"),
        bytes.replace("abandon\"", "finish\""),
        bytes.replace("release_ready", "abandoned"),
        bytes.replace(ID, "00000000-0000-0000-0000-000000000000"),
        bytes.replace(A, &A.to_uppercase()),
    ] {
        assert!(parse_ready(bad.as_bytes(), id(), A, &record()).is_err());
    }
    assert!(parse_ready(bytes.as_bytes(), Uuid::new_v4(), A, &record()).is_err());
    assert!(parse_ready(bytes.as_bytes(), id(), B, &record()).is_err());
    let mut changed = record();
    changed.advance(GatePhase::Closed, None).unwrap();
    assert!(parse_ready(bytes.as_bytes(), id(), A, &changed).is_err());
}
#[test]
fn orphan_terminal_and_partial_ready_cannot_resolve_attempt() {
    let bytes = ready();
    let terminal = serde_json::to_vec(&Terminal::new(id(), A, &bytes)).unwrap();
    assert!(decision(None, Some(&terminal), id(), A, &record()).is_err());
    assert!(decision(Some(b"{"), None, id(), A, &record()).is_err());
    assert_eq!(
        decision(None, None, id(), A, &record()).unwrap(),
        Decision::Pending
    );
    assert_eq!(
        decision(Some(&bytes), None, id(), A, &record()).unwrap(),
        Decision::Ready
    );
    assert_eq!(
        decision(Some(&bytes), Some(&terminal), id(), A, &record()).unwrap(),
        Decision::Abandoned
    );
    let mut released = record();
    for (phase, proof) in [
        (GatePhase::Closed, None),
        (GatePhase::Drained, None),
        (GatePhase::DumpAndIndexDurable, Some(A)),
        (GatePhase::PinsDurable, Some(A)),
        (GatePhase::ReleaseReady, None),
        (GatePhase::Released, None),
    ] {
        released.advance(phase, proof).unwrap();
    }
    assert!(Ready::new(id(), A, &released).is_err());
}
#[test]
fn only_canonical_v4_temporary_names_are_non_authoritative() {
    assert!(temp_name(".tmp-11111111-1111-4111-8111-111111111111"));
    for name in [
        "ready.json",
        ".tmp-",
        ".tmp-11111111-1111-1111-8111-111111111111",
        ".tmp-11111111-1111-4111-7111-111111111111",
        ".tmp-00000000-0000-0000-0000-000000000000",
        ".tmp-AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA",
        ".tmp-../x",
    ] {
        assert!(!temp_name(name));
    }
}
#[cfg(not(target_os = "linux"))]
#[tokio::test]
async fn public_lifecycle_apis_refuse_non_linux_before_connecting() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://learning_admin@localhost/unused")
        .unwrap();
    let config = super::SourceBackupConfig {
        backup_id: id(),
        expected_database: "unused".into(),
        expected_compose_project: "unused".into(),
        isolation_attestation: "unused".into(),
        control_root: "unused".into(),
        local_pin_root: "unused".into(),
        pg_dump_executable: "unused".into(),
        pgpassfile: "unused".into(),
        pg_host: "unused".into(),
        pg_port: 0,
        drain_timeout: std::time::Duration::ZERO,
    };
    for error in [
        crate::finish_source_backup(&pool, &config)
            .await
            .unwrap_err(),
        crate::abandon_source_backup(&pool, &config)
            .await
            .unwrap_err(),
    ] {
        assert!(
            matches!(error,crate::BackupError::Io(ref e) if e.kind()==std::io::ErrorKind::Unsupported)
        );
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DriverAcknowledgment {
    backup_id: Uuid,
    result: String,
}
fn parse_driver_ack(bytes: &[u8], expected: Uuid) -> Result<(), crate::BackupError> {
    if expected.is_nil() || bytes.is_empty() || bytes.len() > 4096 {
        return Err(crate::BackupError::Invalid(
            "driver acknowledgment length or attempt",
        ));
    }
    let value: DriverAcknowledgment = serde_json::from_slice(bytes)?;
    if value.backup_id != expected
        || value.result != "fresh_isolation_ready"
        || serde_json::to_vec(&value)? != bytes
    {
        return Err(crate::BackupError::Invalid(
            "driver acknowledgment canonical identity",
        ));
    }
    Ok(())
}
#[test]
fn driver_ack_accepts_only_exact_canonical_schema_and_attempt() {
    let canonical =
        br#"{"backup_id":"11111111-1111-4111-8111-111111111111","result":"fresh_isolation_ready"}"#;
    parse_driver_ack(canonical, id()).unwrap();
    let value = std::str::from_utf8(canonical).unwrap();
    for bad in [
        value.replacen('{', "{\"unknown\":0,", 1),
        value.replacen(
            '{',
            "{\"backup_id\":\"22222222-2222-4222-8222-222222222222\",",
            1,
        ),
        value.replacen('{', "{\"result\":\"wrong\",", 1),
        value.replace("fresh_isolation_ready", "wrong"),
        value.replace(ID, "00000000-0000-0000-0000-000000000000"),
        value.replacen(':', ": ", 1),
        format!("{value}\n"),
        format!("{{\"result\":\"fresh_isolation_ready\",\"backup_id\":\"{ID}\"}}"),
        value.replace("\"result\":\"fresh_isolation_ready\"", "\"result\":null"),
        value.replace(",\"result\":\"fresh_isolation_ready\"", ""),
    ] {
        assert!(
            parse_driver_ack(bad.as_bytes(), id()).is_err(),
            "malformed ack accepted"
        );
    }
    assert!(parse_driver_ack(canonical, Uuid::new_v4()).is_err());
    assert!(parse_driver_ack(canonical, Uuid::nil()).is_err());
    assert!(parse_driver_ack(b"", id()).is_err());
    assert!(parse_driver_ack(&vec![b' '; 4097], id()).is_err());
    let letter_id = Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap();
    let upper = value.replace(ID, "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA");
    assert!(parse_driver_ack(upper.as_bytes(), letter_id).is_err());
}

// Root-driver live entries are deliberately ignored in ordinary library tests.
// No missing environment/root/proof condition returns success.
#[cfg(target_os = "linux")]
type FixtureAction = (&'static str, Box<dyn FnOnce()>);
#[cfg(target_os = "linux")]
thread_local! {
    static FAULT: std::cell::RefCell<Option<(&'static str,usize)>> = const {std::cell::RefCell::new(None)};
    static PAUSE: std::cell::Cell<Option<&'static str>> = const {std::cell::Cell::new(None)};
    static ENTERED: std::cell::Cell<bool> = const {std::cell::Cell::new(false)};
    static ACTION: std::cell::RefCell<Option<FixtureAction>> = const {std::cell::RefCell::new(None)};
}
#[cfg(target_os = "linux")]
pub(crate) fn hook(point: &'static str) -> Result<(), crate::BackupError> {
    ACTION.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.as_ref().is_some_and(|(name, _)| *name == point) {
            let (_, action) = slot.take().unwrap();
            drop(slot);
            action();
        }
    });
    FAULT.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some((name, skip)) = slot.as_mut()
            && *name == point
        {
            if *skip > 0 {
                *skip -= 1;
            } else {
                slot.take();
                return Err(crate::BackupError::Invalid(
                    "controlled lifecycle test fault",
                ));
            }
        }
        Ok(())
    })
}
#[cfg(target_os = "linux")]
pub(crate) async fn checkpoint(point: &'static str) {
    let pause = PAUSE.with(|value| value.get() == Some(point));
    if pause {
        ENTERED.with(|value| value.set(true));
        std::future::pending::<()>().await;
    }
}
#[cfg(target_os = "linux")]
fn fault(point: &'static str, skip: usize) {
    FAULT.with(|slot| *slot.borrow_mut() = Some((point, skip)));
}
#[cfg(target_os = "linux")]
mod live {
    use super::*;
    use crate::source::*;
    use learning_assets::backup_fs::BackupDir;
    use learning_assets::{FsAssetStore, UploadDeclaration};
    use sqlx::{Connection, PgConnection, postgres::PgPoolOptions};
    use std::{
        fs,
        io::Write,
        os::unix::fs::MetadataExt,
        path::{Path, PathBuf},
        time::{Duration, Instant},
    };
    fn env(key: &str) -> String {
        std::env::var(format!("TEST_C4_LIFECYCLE_{key}"))
            .expect("required lifecycle fixture environment")
    }
    fn path(key: &str) -> PathBuf {
        PathBuf::from(env(key))
    }
    fn private_file(path: &Path) -> Vec<u8> {
        let m = fs::symlink_metadata(path).expect("private fixture file");
        assert!(m.is_file() && !m.file_type().is_symlink());
        assert_eq!(m.uid(), 0);
        assert_eq!(m.mode() & 0o7777, 0o600);
        assert_eq!(m.nlink(), 1);
        assert!(m.len() > 0 && m.len() <= 4096);
        fs::read(path).unwrap()
    }
    pub(super) struct Fixture {
        pub pool: sqlx::PgPool,
        pub config: SourceBackupConfig,
        pub assets: FsAssetStore,
        pub originals: Vec<(PathBuf, Vec<u8>)>,
        pub binding: Vec<u8>,
        pub proof_root: PathBuf,
        ready_rows: Vec<crate::AssetRow>,
        linked_ready: (Uuid, Uuid),
        rejected_nonready: crate::AssetRow,
    }
    impl Fixture {
        pub async fn new() -> Self {
            assert_eq!(
                unsafe { libc::geteuid() },
                0,
                "actual root fixture required"
            );
            let database = env("DATABASE");
            assert!(crate::maintenance::valid_c4_database(&database));
            let dsn = String::from_utf8(private_file(&path("ADMIN_DSN_FILE"))).unwrap();
            let pool = PgPoolOptions::new()
                .max_connections(1)
                .min_connections(0)
                .connect(dsn.trim())
                .await
                .expect("fresh management PG18 fixture");
            let control = path("CONTROL_ROOT");
            let (_, binding) = binding::preissued_fixture(&control);
            let pin = path("PIN_ROOT");
            let asset = path("ASSETS_ROOT");
            let stage = path("ASSET_STAGING_ROOT");
            let proof_root = path("PROOF_ROOT");
            for root in [&pin, &asset, &stage] {
                let handle = BackupDir::open_trusted_private_root(root).unwrap();
                assert!(
                    handle.list().unwrap().is_empty(),
                    "fresh fixture path required"
                );
            }
            BackupDir::open_trusted_private_root(&proof_root).unwrap();
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM public.asset")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(count, 0, "fresh migrated asset database required");
            let actor = Uuid::new_v4();
            let spaces = [Uuid::new_v4(), Uuid::new_v4()];
            sqlx::query("INSERT INTO public.app_user(id) VALUES($1)")
                .bind(actor)
                .execute(&pool)
                .await
                .unwrap();
            for space in spaces {
                sqlx::query("INSERT INTO public.space(id,owner_id) VALUES($1,$2)")
                    .bind(space)
                    .bind(actor)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            let assets = FsAssetStore::new(asset.clone(), stage).unwrap();
            let mut originals = Vec::new();
            let mut ready_rows = Vec::new();
            for (n, value, owners) in [
                (
                    0,
                    b"C4 duplicate original bytes".as_slice(),
                    vec![spaces[0], spaces[1]],
                ),
                (
                    1,
                    b"C4 unreferenced ready original".as_slice(),
                    vec![spaces[0]],
                ),
                (
                    2,
                    b"C4 second-space distinct ready original".as_slice(),
                    vec![spaces[1]],
                ),
            ] {
                let input = proof_root.join(format!("fixture-input-{n}.bin"));
                let root = BackupDir::open_trusted_private_root(&proof_root).unwrap();
                let mut file = root.create_file(&format!("fixture-input-{n}.bin")).unwrap();
                file.write_all(value).unwrap();
                file.sync_all().unwrap();
                root.sync().unwrap();
                let blob = assets
                    .put_from_file(
                        Uuid::new_v4(),
                        &input,
                        UploadDeclaration {
                            expected_size_bytes: value.len() as u64,
                            max_size_bytes: value.len() as u64,
                        },
                    )
                    .unwrap();
                for space in owners {
                    let row = crate::AssetRow {
                        space_id: space,
                        id: Uuid::new_v4(),
                        sha256: blob.sha256().into(),
                        byte_size: blob.size_bytes() as i64,
                        storage_key: blob.storage_key().into(),
                    };
                    sqlx::query("INSERT INTO public.asset(space_id,id,sha256,byte_size,storage_key,media_type,original_file_name,status) VALUES($1,$2,$3,$4,$5,'application/octet-stream','original.bin','ready')")
                        .bind(row.space_id).bind(row.id).bind(&row.sha256).bind(row.byte_size).bind(&row.storage_key).execute(&pool).await.unwrap();
                    ready_rows.push(row);
                }
                originals.push((asset.join(blob.storage_key()), value.to_vec()));
            }
            // Genuine relationship from migrations/0009_assets.sql and catalog_pg.rs.
            let linked_ready = (ready_rows[0].space_id, ready_rows[0].id);
            let resource = Uuid::new_v4();
            let version = Uuid::new_v4();
            sqlx::query("INSERT INTO public.resource(space_id,id,display_name) VALUES($1,$2,'linked lifecycle fixture')").bind(linked_ready.0).bind(resource).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO public.resource_version(space_id,resource_id,id,asset_id,version_no) VALUES($1,$2,$3,$4,1)")
                .bind(linked_ready.0).bind(resource).bind(version).bind(linked_ready.1).execute(&pool).await.unwrap();
            ready_rows.sort_by_key(|row| (row.space_id, row.id));
            // Current genuine schema supports only status=ready. Do not weaken
            // CHECK to manufacture persisted nonready exclusion evidence.
            let absent_value = b"C4 absent nonready candidate original";
            let absent_sha = crate::digest(absent_value);
            let rejected_nonready = crate::AssetRow {
                space_id: spaces[1],
                id: Uuid::new_v4(),
                sha256: absent_sha.clone(),
                byte_size: absent_value.len() as i64,
                storage_key: format!("sha256/{}/{}", &absent_sha[..2], absent_sha),
            };
            assert!(!asset.join(&rejected_nonready.storage_key).exists());
            let before_count: i64 = sqlx::query_scalar("SELECT count(*) FROM public.asset")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(before_count, 4);
            let rejection=sqlx::query("INSERT INTO public.asset(space_id,id,sha256,byte_size,storage_key,media_type,original_file_name,status) VALUES($1,$2,$3,$4,$5,'application/octet-stream','absent.bin','pending')")
                .bind(rejected_nonready.space_id).bind(rejected_nonready.id).bind(&rejected_nonready.sha256).bind(rejected_nonready.byte_size).bind(&rejected_nonready.storage_key).execute(&pool).await.unwrap_err();
            let database_error = rejection
                .as_database_error()
                .expect("genuine nonready CHECK rejection");
            assert_eq!(database_error.code().as_deref(), Some("23514"));
            assert_eq!(database_error.constraint(), Some("asset_status_check"));
            let after_count: i64 = sqlx::query_scalar("SELECT count(*) FROM public.asset")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(
                after_count, before_count,
                "rejected insert changed ready row inventory"
            );
            let rejected_count: i64 =
                sqlx::query_scalar("SELECT count(*) FROM public.asset WHERE space_id=$1 AND id=$2")
                    .bind(rejected_nonready.space_id)
                    .bind(rejected_nonready.id)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(rejected_count, 0);
            let id = Uuid::new_v4();
            let config = SourceBackupConfig {
                backup_id: id,
                expected_database: database,
                expected_compose_project: env("COMPOSE_PROJECT"),
                isolation_attestation: proof_root.join(format!("isolation-{id}.json")),
                control_root: control,
                local_pin_root: pin,
                pg_dump_executable: "/usr/lib/postgresql/18/bin/pg_dump".into(),
                pgpassfile: path("PGPASSFILE"),
                pg_host: "localhost".into(),
                pg_port: 5432,
                drain_timeout: Duration::from_secs(15),
            };
            let mut fixture = Self {
                pool,
                config,
                assets,
                originals,
                binding,
                proof_root,
                ready_rows,
                linked_ready,
                rejected_nonready,
            };
            fixture.refresh().await;
            fixture
        }
        pub async fn refresh(&mut self) {
            // Driver consumes this request and re-observes real namespace. No
            // attestation/inspection/driver lock is ever minted by the Rust test.
            let root = BackupDir::open_trusted_private_root(&self.proof_root).unwrap();
            let request = format!("driver-request-{}.json", Uuid::new_v4());
            let value=serde_json::to_vec(&serde_json::json!({"format_version":1,"backup_id":self.config.backup_id,"database":self.config.expected_database,"compose_project":self.config.expected_compose_project,"operation":"refresh_isolation","request_id":request.trim_start_matches("driver-request-").trim_end_matches(".json")})).unwrap();
            assert!(value.len() <= 4096, "bounded driver request");
            let mut file = root.create_file(&request).unwrap();
            file.write_all(&value).unwrap();
            file.sync_all().unwrap();
            root.sync().unwrap();
            let ack = request.replace("driver-request-", "driver-ready-");
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if self.proof_root.join(&ack).exists() {
                    let bytes = private_file(&self.proof_root.join(&ack));
                    parse_driver_ack(&bytes, self.config.backup_id)
                        .expect("exact canonical driver acknowledgment");
                    verify_isolation_attestation(&self.config, true)
                        .expect("independent live driver proof");
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "driver did not refresh live proof"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        pub async fn next(&mut self) {
            self.config.backup_id = Uuid::new_v4();
            self.config.isolation_attestation = self
                .proof_root
                .join(format!("isolation-{}.json", self.config.backup_id));
            self.refresh().await;
        }
        pub fn root(&self) -> BackupDir {
            BackupDir::open_trusted_private_root(&self.config.control_root).unwrap()
        }
        pub fn journal(&self) -> crate::SourceGateJournal {
            crate::SourceGateJournal::recover_in(&self.root(), self.config.backup_id).unwrap()
        }
        pub fn legacy_bytes(&self) -> Vec<(String, Vec<u8>)> {
            let dir = self
                .root()
                .open_dir(&format!("{}.control", self.config.backup_id))
                .unwrap();
            let mut entries = dir.list().unwrap();
            entries.sort();
            entries
                .into_iter()
                .map(|name| {
                    let value = fs::read(
                        self.config
                            .control_root
                            .join(format!("{}.control", self.config.backup_id))
                            .join(&name),
                    )
                    .unwrap();
                    (name, value)
                })
                .collect()
        }
        pub async fn acl(&self) -> bool {
            sqlx::query_scalar(
                "SELECT has_database_privilege('learning_runtime',current_database(),'CONNECT')",
            )
            .fetch_one(&self.pool)
            .await
            .unwrap()
        }
        pub async fn settle(&self) {
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                match SourceAdmission::try_acquire(&self.pool, &self.config.expected_database).await
                {
                    Ok(mut admission) => {
                        let facts = inspect_gate(admission.connection()).await.unwrap();
                        admission.close().await.unwrap();
                        if facts.other_sessions == 0 {
                            break;
                        }
                    }
                    Err(crate::BackupError::Invalid("source maintenance admission busy")) => {}
                    Err(error) => panic!("unexpected settle admission error: {error}"),
                }
                assert!(
                    Instant::now() < deadline,
                    "old admitted session did not disappear"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        pub async fn capture_stop(&mut self, point: &'static str, phase: GatePhase) {
            fault(point, 0);
            assert!(
                prepare_source_backup(&self.pool, &self.assets, &self.config)
                    .await
                    .is_err()
            );
            assert_eq!(self.journal().record().phase(), phase);
            self.settle().await;
            self.refresh().await;
        }
        pub async fn retained(&self, pin: &SourceLocalPin) {
            self.retained_at(pin, &self.config.control_root, &self.config.local_pin_root)
                .await;
        }
        pub async fn retained_at(&self, pin: &SourceLocalPin, control: &Path, pin_root: &Path) {
            #[derive(serde::Serialize, serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct FixtureIndex {
                format_version: u32,
                assets: Vec<crate::AssetRow>,
            }
            let rows:Vec<(Uuid,Uuid,String,i64,String)>=sqlx::query_as("SELECT space_id,id,sha256,byte_size,storage_key FROM public.asset WHERE status='ready' ORDER BY space_id,id").fetch_all(&self.pool).await.unwrap();
            let actual_ready = rows
                .into_iter()
                .map(
                    |(space_id, id, sha256, byte_size, storage_key)| crate::AssetRow {
                        space_id,
                        id,
                        sha256,
                        byte_size,
                        storage_key,
                    },
                )
                .collect::<Vec<_>>();
            assert_eq!(
                actual_ready, self.ready_rows,
                "actual DB ready inventory changed"
            );
            assert_eq!(actual_ready.len(), 4);
            assert_eq!(
                actual_ready
                    .iter()
                    .map(|row| row.space_id)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                2
            );
            let linked: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM public.resource_version WHERE space_id=$1 AND asset_id=$2",
            )
            .bind(self.linked_ready.0)
            .bind(self.linked_ready.1)
            .fetch_one(&self.pool)
            .await
            .unwrap();
            assert_eq!(
                linked, 1,
                "real linked ready fixture lost its resource_version"
            );
            let unlinked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.asset a WHERE a.status='ready' AND NOT EXISTS(SELECT 1 FROM public.resource_version v WHERE v.space_id=a.space_id AND v.asset_id=a.id) AND NOT EXISTS(SELECT 1 FROM public.block_asset_use u WHERE u.space_id=a.space_id AND u.asset_id=a.id))").fetch_one(&self.pool).await.unwrap();
            assert!(
                unlinked,
                "at least one genuinely unlinked ready asset required"
            );
            let excluded: i64 =
                sqlx::query_scalar("SELECT count(*) FROM public.asset WHERE space_id=$1 AND id=$2")
                    .bind(self.rejected_nonready.space_id)
                    .bind(self.rejected_nonready.id)
                    .fetch_one(&self.pool)
                    .await
                    .unwrap();
            assert_eq!(
                excluded, 0,
                "rejected nonready candidate unexpectedly persisted"
            );
            assert!(
                !path("ASSETS_ROOT")
                    .join(&self.rejected_nonready.storage_key)
                    .exists()
            );
            let sealed = pin_root.join(format!("{}.sealed", self.config.backup_id));
            let index_bytes = fs::read(sealed.join("asset-index.json")).unwrap();
            let index: FixtureIndex = serde_json::from_slice(&index_bytes).unwrap();
            assert_eq!(index.format_version, 1);
            assert_eq!(
                serde_json::to_vec(&index).unwrap(),
                index_bytes,
                "logical index is not canonical"
            );
            assert_eq!(
                index.assets, actual_ready,
                "full logical index differs from actual DB ready rows"
            );
            assert!(
                !index
                    .assets
                    .iter()
                    .any(|row| row.space_id == self.rejected_nonready.space_id
                        && row.id == self.rejected_nonready.id)
            );
            pin.manifest().validate_with_index(&index_bytes).unwrap();
            assert_eq!(
                fs::read(sealed.join("manifest.json")).unwrap(),
                pin.manifest().canonical_bytes().unwrap()
            );
            assert_eq!(
                fs::read(
                    control.join(format!("{}.source/asset-index.json", self.config.backup_id))
                )
                .unwrap(),
                index_bytes
            );
            assert_eq!(pin.manifest().logical_asset_count, 4);
            assert_eq!(pin.manifest().unique_asset_bytes, 96);
            assert_eq!(self.originals.len(), 3);
            assert_eq!(
                self.originals
                    .iter()
                    .map(|(_, value)| value.len() as u64)
                    .sum::<u64>(),
                96
            );
            let mut expected_files = Vec::new();
            for (original, value) in &self.originals {
                let relative = original
                    .strip_prefix(
                        original
                            .parent()
                            .unwrap()
                            .parent()
                            .unwrap()
                            .parent()
                            .unwrap(),
                    )
                    .unwrap();
                assert_eq!(
                    fs::read(sealed.join("assets").join(relative)).unwrap(),
                    *value
                );
                expected_files.push(crate::FileRecord {
                    path: format!("assets/{}", relative.to_str().unwrap()),
                    size: value.len() as u64,
                    sha256: crate::digest(value),
                });
            }
            expected_files.sort_by(|a, b| a.path.cmp(&b.path));
            let actual_files = pin
                .manifest()
                .files
                .iter()
                .filter(|record| record.path.starts_with("assets/"))
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(
                actual_files, expected_files,
                "exact unique original files differ"
            );
            assert!(
                !sealed
                    .join("assets")
                    .join(&self.rejected_nonready.storage_key)
                    .exists()
            );
            assert_eq!(
                fs::read(control.join("source-binding.json")).unwrap(),
                self.binding
            );
            assert!(
                !pin_root
                    .join(format!("{}.complete", self.config.backup_id))
                    .exists()
            );
        }
        pub async fn finish(&mut self) -> SourceLocalPin {
            self.refresh().await;
            let before = self.legacy_bytes();
            let pin = finish_source_backup(&self.pool, &self.config)
                .await
                .unwrap();
            assert!(self.acl().await);
            assert_eq!(self.journal().record().phase(), GatePhase::Released);
            self.retained(&pin).await;
            let after = self.legacy_bytes();
            for entry in before {
                assert!(after.contains(&entry), "old canonical phase changed");
            }
            pin
        }
        pub async fn early(&mut self, phase: GatePhase) {
            let mut journal =
                crate::SourceGateJournal::start_in(&self.root(), self.config.backup_id).unwrap();
            if phase != GatePhase::Intent {
                journal.advance(GatePhase::Closed, None).unwrap();
            }
            if phase == GatePhase::Drained {
                journal.advance(GatePhase::Drained, None).unwrap();
            }
        }
        pub async fn abandon(&mut self) {
            self.refresh().await;
            let before = self.legacy_bytes();
            abandon_source_backup(&self.pool, &self.config)
                .await
                .unwrap();
            assert_eq!(self.legacy_bytes(), before);
            assert!(self.acl().await);
            scan(&self.root(), None).unwrap();
            let again = self.legacy_bytes();
            self.refresh().await;
            abandon_source_backup(&self.pool, &self.config)
                .await
                .unwrap();
            assert_eq!(self.legacy_bytes(), again);
            assert!(
                finish_source_backup(&self.pool, &self.config)
                    .await
                    .is_err()
            );
        }
        pub async fn external(&self) -> PgConnection {
            let value = String::from_utf8(private_file(&path("ADMIN_DSN_FILE"))).unwrap();
            PgConnection::connect(value.trim()).await.unwrap()
        }
    }
}
#[cfg(target_os = "linux")]
use live::Fixture;

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independent build-pinned PG18 root driver; real dump and all-ready fixture"]
async fn real_capture_all_ready_and_retained_pin() {
    let mut f = Fixture::new().await;
    let pin = super::prepare_source_backup(&f.pool, &f.assets, &f.config)
        .await
        .unwrap();
    f.retained(&pin).await;
    // Remove only the three freshly created fixture originals to prove pin
    // retention independently of the original store; all backup evidence stays.
    for (path, _) in &f.originals {
        assert!(path.starts_with(std::env::var("TEST_C4_LIFECYCLE_ASSETS_ROOT").unwrap()));
        std::fs::remove_file(path).unwrap();
    }
    let checked = crate::verify_sealed(&f.config.local_pin_root, f.config.backup_id).unwrap();
    assert_eq!(checked.manifest_sha256(), pin.sealed().manifest_sha256());
    f.refresh().await;
    let before = f.legacy_bytes();
    super::finish_source_backup(&f.pool, &f.config)
        .await
        .unwrap();
    assert_eq!(f.legacy_bytes(), before);
    assert!(
        super::abandon_source_backup(&f.pool, &f.config)
            .await
            .is_err()
    );
}
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independent build-pinned PG18 root driver; controlled sealed-rename fault"]
async fn finish_real_pin_after_sealed_rename() {
    let mut f = Fixture::new().await;
    f.capture_stop("sealed_after_rename", GatePhase::DumpAndIndexDurable)
        .await;
    let pin = f.finish().await;
    f.retained(&pin).await;
    f.next().await;
    f.capture_stop("capture_dump_durable", GatePhase::DumpAndIndexDurable)
        .await;
    assert!(
        super::finish_source_backup(&f.pool, &f.config)
            .await
            .is_err()
    );
    assert!(!f.acl().await);
    assert_eq!(f.journal().record().phase(), GatePhase::DumpAndIndexDurable);
    f.abandon().await;
}
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independent build-pinned PG18 root driver; full pin rehash"]
async fn finish_real_pin_from_pins_durable() {
    let mut f = Fixture::new().await;
    f.capture_stop("capture_pins_durable", GatePhase::PinsDurable)
        .await;
    let _ = f.finish().await;
    f.next().await;
    f.capture_stop("capture_pins_durable", GatePhase::PinsDurable)
        .await;
    let source = f
        .config
        .control_root
        .join(format!("{}.source/database.dump", f.config.backup_id));
    let bytes = std::fs::read(&source).unwrap();
    let mut altered = bytes.clone();
    altered[5] ^= 1;
    std::fs::write(&source, &altered).unwrap();
    let before = f.legacy_bytes();
    assert!(
        super::finish_source_backup(&f.pool, &f.config)
            .await
            .is_err()
    );
    assert_eq!(f.legacy_bytes(), before);
    assert!(!f.acl().await);
    assert_eq!(std::fs::read(source).unwrap(), altered);
    f.abandon().await;
    f.next().await;
    f.capture_stop("capture_pins_durable", GatePhase::PinsDurable)
        .await;
    let original = &f.originals[0].0;
    let relative = original
        .strip_prefix(
            original
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap(),
        )
        .unwrap();
    let sealed_asset = f
        .config
        .local_pin_root
        .join(format!("{}.sealed", f.config.backup_id))
        .join("assets")
        .join(relative);
    let mut altered = std::fs::read(&sealed_asset).unwrap();
    altered[0] ^= 1;
    std::fs::write(&sealed_asset, &altered).unwrap();
    let before = f.legacy_bytes();
    assert!(
        super::finish_source_backup(&f.pool, &f.config)
            .await
            .is_err()
    );
    assert!(!f.acl().await);
    assert_eq!(f.legacy_bytes(), before);
    assert_eq!(std::fs::read(&sealed_asset).unwrap(), altered);
    f.abandon().await;
}
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independent build-pinned PG18 root driver; actual tokio cancellation after committed grant"]
async fn finish_real_release_ready_before_and_after_grant() {
    use sqlx::Connection;
    let mut f = Fixture::new().await;
    for (point, open) in [
        ("capture_before_grant", false),
        ("capture_after_grant", true),
    ] {
        PAUSE.with(|v| v.set(Some(point)));
        ENTERED.with(|v| v.set(false));
        let pool = f.pool.clone();
        let assets = f.assets.clone();
        let config = f.config.clone();
        let task =
            tokio::spawn(
                async move { super::prepare_source_backup(&pool, &assets, &config).await },
            );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !ENTERED.with(|v| v.get()) {
            assert!(!task.is_finished());
            assert!(std::time::Instant::now() < deadline);
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        // Observe both durable journal and committed ACL from an independent
        // management connection before cancellation, while owner still lives.
        assert_eq!(f.journal().record().phase(), GatePhase::ReleaseReady);
        let legacy = f.legacy_bytes();
        let pin = crate::verify_sealed(&f.config.local_pin_root, f.config.backup_id).unwrap();
        let mut observer = f.external().await;
        let observed: bool = sqlx::query_scalar(
            "SELECT has_database_privilege('learning_runtime',current_database(),'CONNECT')",
        )
        .fetch_one(&mut observer)
        .await
        .unwrap();
        assert_eq!(observed, open);
        let owner_pid:i32=sqlx::query_scalar("SELECT pid FROM pg_stat_activity WHERE datname=current_database() AND usename='learning_admin' AND pid<>pg_backend_pid()").fetch_one(&mut observer).await.unwrap();
        observer.close().await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        PAUSE.with(|v| v.set(None));
        f.settle().await;
        let owner_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1)")
                .bind(owner_pid)
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert!(!owner_exists);
        assert_eq!(f.legacy_bytes(), legacy);
        assert_eq!(
            crate::verify_sealed(&f.config.local_pin_root, f.config.backup_id)
                .unwrap()
                .manifest_sha256(),
            pin.manifest_sha256()
        );
        assert_eq!(f.acl().await, open);
        let _ = f.finish().await;
        f.next().await;
    }
}
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independent build-pinned PG18 root driver; early and real late abandonment"]
async fn abandon_early_and_late_attempts() {
    let mut f = Fixture::new().await;
    for phase in [GatePhase::Intent, GatePhase::Closed, GatePhase::Drained] {
        f.early(phase).await;
        let before = f.legacy_bytes();
        let acl = f.acl().await;
        assert!(
            super::finish_source_backup(&f.pool, &f.config)
                .await
                .is_err()
        );
        assert_eq!(f.legacy_bytes(), before);
        assert_eq!(f.acl().await, acl);
        f.abandon().await;
        f.next().await;
    }
    f.capture_stop("capture_pins_durable", GatePhase::PinsDurable)
        .await;
    let pin_root = f.config.local_pin_root.clone();
    let backup_id = f.config.backup_id;
    f.abandon().await;
    crate::verify_sealed(&pin_root, backup_id).unwrap();
}
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independent build-pinned PG18 root driver; controlled sidecar publication seams"]
async fn abandon_crash_retry_and_terminal_ambiguity() {
    let mut f = Fixture::new().await;
    f.early(GatePhase::Intent).await;
    fault("sidecar_before_rename", 0);
    assert!(
        super::abandon_source_backup(&f.pool, &f.config)
            .await
            .is_err()
    );
    f.settle().await;
    assert!(scan(&f.root(), None).is_err());
    f.abandon().await;
    f.next().await;
    f.early(GatePhase::Intent).await;
    fault("abandon_ready_durable", 0);
    assert!(
        super::abandon_source_backup(&f.pool, &f.config)
            .await
            .is_err()
    );
    f.settle().await;
    assert!(scan(&f.root(), None).is_err());
    f.abandon().await;
    f.next().await;
    f.early(GatePhase::Intent).await;
    fault("sidecar_after_rename", 1);
    assert!(
        super::abandon_source_backup(&f.pool, &f.config)
            .await
            .is_err()
    );
    f.settle().await;
    assert!(!f.acl().await);
    assert!(
        f.root()
            .open_dir(&format!("{}.abandonment", f.config.backup_id))
            .unwrap()
            .open_file("abandoned.json")
            .is_ok()
    );
    f.refresh().await;
    let before = f.legacy_bytes();
    assert!(
        super::abandon_source_backup(&f.pool, &f.config)
            .await
            .is_err()
    );
    assert!(!f.acl().await);
    assert_eq!(f.legacy_bytes(), before);
}
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independent build-pinned PG18 root driver; same-session controlled release failures"]
async fn lifecycle_release_failure_compensates_same_session() {
    let mut f = Fixture::new().await;
    for point in [
        "lifecycle_before_grant",
        "lifecycle_after_grant",
        "phase_after_rename",
    ] {
        f.capture_stop("capture_release_ready", GatePhase::ReleaseReady)
            .await;
        fault(point, 0);
        assert!(
            super::finish_source_backup(&f.pool, &f.config)
                .await
                .is_err()
        );
        f.settle().await;
        assert!(!f.acl().await);
        if f.journal().record().phase() == GatePhase::Released {
            f.refresh().await;
            assert!(
                super::finish_source_backup(&f.pool, &f.config)
                    .await
                    .is_err()
            );
            assert!(!f.acl().await);
            break;
        }
        let _ = f.finish().await;
        f.next().await;
    }
}
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independent build-pinned PG18 root driver; busy, prepared, aliases, held control/pin roots"]
async fn lifecycle_admission_and_held_roots() {
    use sqlx::{Connection, Executor};
    let mut f = Fixture::new().await;
    f.capture_stop("capture_pins_durable", GatePhase::PinsDurable)
        .await;
    let before = f.legacy_bytes();
    let mut other = f.external().await;
    sqlx::query("SELECT pg_advisory_lock($1,$2)")
        .bind(0x4b57_4334_i32)
        .bind(0x5352_4345_i32)
        .execute(&mut other)
        .await
        .unwrap();
    assert!(
        super::finish_source_backup(&f.pool, &f.config)
            .await
            .is_err()
    );
    assert!(
        super::abandon_source_backup(&f.pool, &f.config)
            .await
            .is_err()
    );
    assert_eq!(f.legacy_bytes(), before);
    other.close().await.unwrap();
    f.settle().await;
    let mut alias = f.config.clone();
    alias.local_pin_root = alias.control_root.clone();
    assert!(super::finish_source_backup(&f.pool, &alias).await.is_err());
    assert!(super::abandon_source_backup(&f.pool, &alias).await.is_err());

    let alternate = f.config.control_root.with_file_name("control-copy");
    std::fs::create_dir(&alternate).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&alternate, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let copied =
        learning_assets::backup_fs::BackupDir::open_trusted_private_root(&alternate).unwrap();
    {
        use std::io::Write;
        let mut file = copied.create_file("source-binding.json").unwrap();
        file.write_all(&f.binding).unwrap();
        file.sync_all().unwrap();
        copied.sync().unwrap();
    }
    let mut wrong = f.config.clone();
    wrong.control_root = alternate;
    assert!(super::finish_source_backup(&f.pool, &wrong).await.is_err());
    assert!(super::abandon_source_backup(&f.pool, &wrong).await.is_err());
    wrong = f.config.clone();
    wrong.expected_database = format!("learning_backup_c4_task3_{}", Uuid::new_v4());
    assert!(super::finish_source_backup(&f.pool, &wrong).await.is_err());
    assert!(super::abandon_source_backup(&f.pool, &wrong).await.is_err());
    assert_eq!(f.legacy_bytes(), before);
    let mut prepared = f.external().await;
    let gid = format!("lifecycle-{}", Uuid::new_v4());
    prepared.execute("BEGIN").await.unwrap();
    let table = format!("c4_lifecycle_prepared_{}", Uuid::new_v4().simple());
    prepared
        .execute(format!("CREATE TABLE public.{table}(id integer PRIMARY KEY)").as_str())
        .await
        .unwrap();
    prepared
        .execute(format!("INSERT INTO public.{table}(id) VALUES(1)").as_str())
        .await
        .unwrap();
    prepared
        .execute(format!("PREPARE TRANSACTION '{gid}'").as_str())
        .await
        .unwrap();
    prepared.close().await.unwrap();
    let outstanding: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_prepared_xacts WHERE database=current_database() AND gid=$1",
    )
    .bind(&gid)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(outstanding, 1);
    assert!(
        super::finish_source_backup(&f.pool, &f.config)
            .await
            .is_err()
    );
    assert!(
        super::abandon_source_backup(&f.pool, &f.config)
            .await
            .is_err()
    );
    assert_eq!(f.legacy_bytes(), before);
    // Only this new fixture's transaction is resolved, never external work.
    sqlx::query(&format!("ROLLBACK PREPARED '{gid}'"))
        .execute(&f.pool)
        .await
        .unwrap();
    f.refresh().await;
    let control = f.config.control_root.clone();
    let pins = f.config.local_pin_root.clone();
    let control_moved = control.with_file_name("control-held");
    let pins_moved = pins.with_file_name("pins-held");
    ACTION.with(|slot| {
        *slot.borrow_mut() = Some((
            "lifecycle_before_grant",
            Box::new(move || {
                std::fs::rename(&control, &control_moved).unwrap();
                std::fs::rename(&pins, &pins_moved).unwrap();
                for root in [&control, &pins] {
                    std::fs::create_dir(root).unwrap();
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700)).unwrap();
                }
            }),
        ))
    });
    let pin = super::finish_source_backup(&f.pool, &f.config)
        .await
        .unwrap();
    f.retained_at(
        &pin,
        &f.config.control_root.with_file_name("control-held"),
        &f.config.local_pin_root.with_file_name("pins-held"),
    )
    .await;
    assert!(f.root().list().unwrap().is_empty());
    assert!(
        std::fs::read_dir(&f.config.local_pin_root)
            .unwrap()
            .next()
            .is_none()
    );
    let retained = f.config.control_root.with_file_name("control-held");
    assert_eq!(
        crate::SourceGateJournal::recover(&retained, f.config.backup_id)
            .unwrap()
            .record()
            .phase(),
        GatePhase::Released
    );
    crate::verify_sealed(
        &f.config.local_pin_root.with_file_name("pins-held"),
        f.config.backup_id,
    )
    .unwrap();
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "root plus TEST_C4_LIFECYCLE_FS_ROOT trusted fresh directory; controlled phase write/fsync/rename seams"]
fn journal_future_publications_are_atomic_and_legacy_bad_finals_stay_bad() {
    use learning_assets::backup_fs::BackupDir;
    use std::io::Write;
    assert_eq!(unsafe { libc::geteuid() }, 0);
    let path = std::path::PathBuf::from(
        std::env::var_os("TEST_C4_LIFECYCLE_FS_ROOT").expect("trusted fresh FS root required"),
    );
    let root = BackupDir::open_trusted_private_root(&path).unwrap();

    for point in [
        "initial_before_write",
        "initial_before_rename",
        "initial_after_rename",
        "initial_before_readback",
    ] {
        let id = Uuid::new_v4();
        fault(point, 0);
        assert!(crate::SourceGateJournal::start_in(&root, id).is_err());
        if ["initial_after_rename", "initial_before_readback"].contains(&point) {
            let recovered = crate::SourceGateJournal::recover_in(&root, id).unwrap();
            assert_eq!(recovered.record().phase(), GatePhase::Intent);
        } else {
            assert!(root.open_dir(&format!("{id}.control")).is_err());
            assert!(crate::SourceGateJournal::recover_in(&root, id).is_err());
        }
        assert!(root.open_dir(&format!("{id}.journal-staging")).is_ok());
    }
    for point in [
        "phase_before_write",
        "phase_before_file_sync",
        "phase_before_staging_sync",
        "phase_before_rename",
        "phase_after_rename",
        "phase_before_readback",
    ] {
        let id = Uuid::new_v4();
        let mut journal = crate::SourceGateJournal::start_in(&root, id).unwrap();
        let original = std::fs::read(path.join(format!("{id}.control/intent.json"))).unwrap();
        fault(point, 0);
        assert!(journal.advance(GatePhase::Closed, None).is_err());
        // Memory never claims a failed publication, even when final rename is visible.
        assert_eq!(journal.record().phase(), GatePhase::Intent);
        let dir = root.open_dir(&format!("{id}.control")).unwrap();
        assert!(
            !dir.list()
                .unwrap()
                .iter()
                .any(|name| name.ends_with(".tmp"))
        );
        let recovered = crate::SourceGateJournal::recover_in(&root, id).unwrap();
        let expected = if ["phase_after_rename", "phase_before_readback"].contains(&point) {
            GatePhase::Closed
        } else {
            GatePhase::Intent
        };
        assert_eq!(recovered.record().phase(), expected);
        assert_eq!(
            std::fs::read(path.join(format!("{id}.control/intent.json"))).unwrap(),
            original
        );
    }
    for value in [b"".as_slice(),b"{",br#"{"backup_id":"11111111-1111-4111-8111-111111111111","phase":"closed","dump_and_index_sha256":null,"pins_sha256":null}"#] {
        let id=Uuid::new_v4();let dir=root.create_dir(&format!("{id}.control")).unwrap();let mut file=dir.create_file("intent.json").unwrap();file.write_all(value).unwrap();file.sync_all().unwrap();dir.sync().unwrap();
        assert!(crate::SourceGateJournal::recover_in(&root,id).is_err());assert_eq!(std::fs::read(path.join(format!("{id}.control/intent.json"))).unwrap(),value);
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "root plus TEST_C4_LIFECYCLE_FS_ROOT trusted fresh directory and independent compiled binding; synthetic authority only"]
fn strict_scan_joins_journal_sidecar_and_retained_staging_without_inference() {
    use learning_assets::backup_fs::BackupDir;
    use std::{
        io::Write,
        os::unix::fs::{PermissionsExt, symlink},
    };
    assert_eq!(unsafe { libc::geteuid() }, 0);
    let path = std::path::PathBuf::from(
        std::env::var_os("TEST_C4_LIFECYCLE_FS_ROOT").expect("trusted fresh FS root"),
    );
    let parent = BackupDir::open_trusted_private_root(&path).unwrap();
    let binding = match option_env!("KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256") {
        Some(value) => value,
        None => panic!("independently compiled binding required"),
    };
    let write = |dir: &BackupDir, name: &str, value: &[u8]| {
        let mut file = dir.create_file(name).unwrap();
        file.write_all(value).unwrap();
        file.sync_all().unwrap();
        dir.sync().unwrap();
    };
    for case in [
        "valid_terminal",
        "partial_temp",
        "unknown",
        "orphan_terminal",
        "bad_ready",
        "wide_ready",
        "linked_ready",
        "symlink_ready",
        "foreign_ready",
        "released_conflict",
        "other_unresolved",
        "orphan_staging",
    ] {
        let case_name = format!("case-{case}-{}", Uuid::new_v4());
        let root = parent.create_dir(&case_name).unwrap();
        let id = Uuid::new_v4();
        if case == "orphan_staging" {
            root.create_dir(&format!("{id}.journal-staging")).unwrap();
            assert!(scan(&root, Some(id)).is_err());
            continue;
        }
        let mut journal = crate::SourceGateJournal::start_in(&root, id).unwrap();
        if case == "other_unresolved" {
            crate::SourceGateJournal::start_in(&root, Uuid::new_v4()).unwrap();
            assert!(scan(&root, Some(id)).is_err());
            continue;
        }
        let ready =
            serde_json::to_vec(&Ready::new(id, binding, journal.record()).unwrap()).unwrap();
        if case == "released_conflict" {
            for (phase, proof) in [
                (GatePhase::Closed, None),
                (GatePhase::Drained, None),
                (GatePhase::DumpAndIndexDurable, Some(A)),
                (GatePhase::PinsDurable, Some(B)),
                (GatePhase::ReleaseReady, None),
                (GatePhase::Released, None),
            ] {
                journal.advance(phase, proof).unwrap();
            }
        }
        let sidecar = root.create_dir(&format!("{id}.abandonment")).unwrap();
        match case {
            "valid_terminal" => {
                write(&sidecar, "ready.json", &ready);
                write(
                    &sidecar,
                    "abandoned.json",
                    &serde_json::to_vec(&Terminal::new(id, binding, &ready)).unwrap(),
                );
            }
            "partial_temp" => write(&sidecar, &format!(".tmp-{}", Uuid::new_v4()), b"{"),
            "unknown" => write(&sidecar, "unknown", b"{}"),
            "orphan_terminal" => write(
                &sidecar,
                "abandoned.json",
                &serde_json::to_vec(&Terminal::new(id, binding, &ready)).unwrap(),
            ),
            "bad_ready" => write(&sidecar, "ready.json", b"{"),
            "foreign_ready" => {
                let foreign = Uuid::new_v4();
                let value = serde_json::to_vec(
                    &Ready::new(foreign, binding, &SourceGateRecord::new(foreign)).unwrap(),
                )
                .unwrap();
                write(&sidecar, "ready.json", &value);
            }
            "released_conflict" => write(&sidecar, "ready.json", &ready),
            "wide_ready" => {
                write(&sidecar, "ready.json", &ready);
                std::fs::set_permissions(
                    path.join(&case_name)
                        .join(format!("{id}.abandonment/ready.json")),
                    std::fs::Permissions::from_mode(0o640),
                )
                .unwrap();
            }
            "linked_ready" => {
                write(&sidecar, "ready.json", &ready);
                std::fs::hard_link(
                    path.join(&case_name)
                        .join(format!("{id}.abandonment/ready.json")),
                    path.join(format!("hardlink-{id}")),
                )
                .unwrap();
            }
            "symlink_ready" => {
                let target = path.join(format!("external-ready-{id}"));
                std::fs::write(&target, &ready).unwrap();
                symlink(
                    &target,
                    path.join(&case_name)
                        .join(format!("{id}.abandonment/ready.json")),
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        if case == "valid_terminal" {
            scan(&root, None).unwrap();
        } else if case == "partial_temp" {
            scan(&root, Some(id)).unwrap();
            assert!(scan(&root, None).is_err());
        } else {
            assert!(
                scan(&root, Some(id)).is_err(),
                "malformed authority must block even its target"
            );
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "root plus fresh TEST_C4_LIFECYCLE_FS_ROOT and independent compile binding; controlled rename/pre-sync seam"]
fn abandonment_terminal_scan_resyncs_child_and_rechecks_before_clearing_attempt() {
    use learning_assets::backup_fs::BackupDir;
    use std::{
        io::Write,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };
    assert_eq!(unsafe { libc::geteuid() }, 0);
    let path = std::path::PathBuf::from(
        std::env::var_os("TEST_C4_LIFECYCLE_FS_ROOT").expect("fresh trusted filesystem root"),
    );
    let parent = BackupDir::open_trusted_private_root(&path).unwrap();
    let binding = match option_env!("KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256") {
        Some(value) => value,
        None => panic!("independent compile binding required"),
    };
    for case in ["visible_terminal", "child_sync_error", "readback_changed"] {
        let name = format!("fix1-{case}-{}", Uuid::new_v4());
        let root = parent.create_dir(&name).unwrap();
        let id = Uuid::new_v4();
        let journal = crate::SourceGateJournal::start_in(&root, id).unwrap();
        let sidecar = root.create_dir(&format!("{id}.abandonment")).unwrap();
        let ready =
            serde_json::to_vec(&Ready::new(id, binding, journal.record()).unwrap()).unwrap();
        let terminal = serde_json::to_vec(&Terminal::new(id, binding, &ready)).unwrap();
        let write = |leaf: &str, value: &[u8]| {
            let mut file = sidecar.create_file(leaf).unwrap();
            file.write_all(value).unwrap();
            file.sync_all().unwrap();
        };
        write("ready.json", &ready);
        sidecar.sync().unwrap();
        let temp = format!(".tmp-{}", Uuid::new_v4());
        write(&temp, &terminal);
        sidecar.sync().unwrap();
        // Exact seam: terminal rename is visible; no child parent sync yet.
        sidecar
            .rename_noreplace_without_sync(&temp, "abandoned.json")
            .unwrap();
        assert_eq!(
            std::fs::read(
                path.join(&name)
                    .join(format!("{id}.abandonment/abandoned.json"))
            )
            .unwrap(),
            terminal
        );
        let observed = Arc::new(AtomicBool::new(false));
        let seen = observed.clone();
        ACTION.with(|slot| {
            *slot.borrow_mut() = Some((
                "scan_abandonment_after_child_sync",
                Box::new(move || {
                    seen.store(true, Ordering::SeqCst);
                }),
            ))
        });
        if case == "child_sync_error" {
            fault("scan_abandonment_before_child_sync", 0);
            assert!(matches!(
                scan(&root, None),
                Err(crate::BackupError::Invalid(
                    "controlled lifecycle test fault"
                ))
            ));
            assert!(
                !observed.load(Ordering::SeqCst),
                "readback must not run after failed child sync boundary"
            );
            assert!(
                FAULT.with(|slot| slot.borrow().is_none()),
                "scanner failed before intended child-sync fault site"
            );
            scan(&root, None).unwrap();
            assert!(observed.load(Ordering::SeqCst));
        } else if case == "readback_changed" {
            let ready_path = path
                .join(&name)
                .join(format!("{id}.abandonment/ready.json"));
            let seen = observed.clone();
            ACTION.with(|slot| {
                *slot.borrow_mut() = Some((
                    "scan_abandonment_after_child_sync",
                    Box::new(move || {
                        seen.store(true, Ordering::SeqCst);
                        std::fs::write(ready_path, b"{").unwrap();
                    }),
                ))
            });
            assert!(
                scan(&root, None).is_err(),
                "post-sync changed authority must not clear unresolved attempts"
            );
            assert!(observed.load(Ordering::SeqCst));
        } else {
            scan(&root, None).unwrap();
            assert!(
                observed.load(Ordering::SeqCst),
                "terminal cleared without child sync boundary"
            );
            assert_eq!(
                std::fs::read(
                    path.join(&name)
                        .join(format!("{id}.abandonment/ready.json"))
                )
                .unwrap(),
                ready
            );
        }
    }
}
