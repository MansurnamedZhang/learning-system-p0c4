mod support;

use learning_assets::{FsAssetStore, UploadDeclaration};
use learning_core::{
    AssetRef, BlockRef, BodyV3, ContentDraft, ContentV3, CreateContent, ExactRef, Intent, JobInput,
    Principal, canonical_json, hex_digest,
};
use learning_db::{
    AssetIntegrityProcessor, AssetMedia, AssetProcessOutcome, AssetRecord, AssetStore, JobLease,
    JobStore, VersionedContentStore,
};
use serde_json::json;
use sqlx::Row;
use std::{fs, path::PathBuf, time::Duration};
use support::{TestRig, sqlstate};
use uuid::Uuid;

const PNG: &[u8] = &[137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 0, 73, 69, 78, 68];
const PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n%%EOF\n";
const NOTEBOOK: &[u8] = br#"{"cells":[{"cell_type":"code","source":["print(42)"]}],"nbformat":4}"#;

struct Files {
    root: PathBuf,
    store: FsAssetStore,
}

impl Files {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("p0c2-processor-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let store = FsAssetStore::new(root.join("assets"), root.join("staging")).unwrap();
        Self { root, store }
    }

    async fn register(
        &self,
        rig: &TestRig,
        actor: Principal,
        space: Uuid,
        bytes: &[u8],
        mime: &str,
    ) -> AssetRecord {
        let source = self.root.join(format!("source-{}", Uuid::new_v4()));
        fs::write(&source, bytes).unwrap();
        let blob = self
            .store
            .put_from_file(
                Uuid::new_v4(),
                &source,
                UploadDeclaration {
                    expected_size_bytes: bytes.len() as u64,
                    max_size_bytes: 1_000_000,
                },
            )
            .unwrap();
        AssetStore::new(rig.runtime_pool.clone(), self.store.clone())
            .register_verified(
                actor,
                space,
                Uuid::new_v4(),
                blob,
                AssetMedia {
                    media_type: mime.into(),
                    original_file_name: "source.bin".into(),
                },
            )
            .await
            .unwrap()
    }

    fn asset_path(&self, asset: &AssetRecord) -> PathBuf {
        self.root.join("assets").join(&asset.storage_key)
    }

    fn processor(&self, rig: &TestRig) -> AssetIntegrityProcessor {
        AssetIntegrityProcessor::new(
            AssetStore::new(rig.runtime_pool.clone(), self.store.clone()),
            JobStore::new(rig.runtime_pool.clone()),
        )
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn draft(asset: AssetRef, mime: &str, basis_refs: Vec<ExactRef>) -> ContentDraft {
    let body = if mime == "image/png" {
        BodyV3::Figure {
            asset,
            usage: "lecture_diagram".into(),
            caption: "A diagram".into(),
            alt: "A diagram".into(),
            decorative: false,
        }
    } else {
        BodyV3::Attachment {
            asset,
            display_name: "An original".into(),
        }
    };
    ContentDraft::V3(ContentV3 {
        intent: Intent::Note,
        language: "en".into(),
        title: "Asset source".into(),
        body,
        basis_refs,
        requires_context: vec![],
        source_run: None,
    })
}

struct Work {
    asset: AssetRecord,
    block: BlockRef,
    job: Uuid,
    lease: JobLease,
}

async fn create_work(
    rig: &TestRig,
    files: &Files,
    actor: Principal,
    space: Uuid,
    bytes: &[u8],
    mime: &str,
    basis_refs: Vec<ExactRef>,
) -> Work {
    let asset = files.register(rig, actor, space, bytes, mime).await;
    let saved = VersionedContentStore::new(rig.runtime_pool.clone())
        .create(
            actor,
            space,
            CreateContent {
                request_id: Uuid::new_v4(),
                draft: draft(asset.reference.clone(), mime, basis_refs),
                reason: "source".into(),
            },
        )
        .await
        .unwrap();
    let block = BlockRef {
        block_id: saved.block_id,
        revision_id: saved.revision_id,
    };
    let input = JobInput::asset_integrity(actor.actor_id, space, block.clone()).unwrap();
    let jobs = JobStore::new(rig.runtime_pool.clone());
    assert!(jobs.dispatch_pending(256).await.unwrap() >= 1);
    let job: Uuid = sqlx::query_scalar("SELECT id FROM public.job WHERE idempotency_key=$1")
        .bind(input.business_key())
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    let lease = jobs
        .claim(job, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    Work {
        asset,
        block,
        job,
        lease,
    }
}

async fn assert_no_published_result(rig: &TestRig, job: Uuid) {
    let row = sqlx::query("SELECT status,output_digest FROM public.job WHERE id=$1")
        .bind(job)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    assert_ne!(row.get::<String, _>("status"), "succeeded");
    assert!(row.get::<Option<String>, _>("output_digest").is_none());
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.asset_integrity_result WHERE job_id=$1")
            .bind(job)
            .fetch_one(&rig.admin_pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
}

async fn assert_failure_state(rig: &TestRig, job: Uuid, status: &str, error_class: &str) {
    let row = sqlx::query(
        "SELECT status,attempt_count,last_error_class,lease_token,lease_expires_at,next_attempt_at \
         FROM public.job WHERE id=$1",
    )
    .bind(job)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("status"), status);
    assert_eq!(row.get::<i32, _>("attempt_count"), 1);
    assert_eq!(
        row.get::<Option<String>, _>("last_error_class").as_deref(),
        Some(error_class)
    );
    assert!(row.get::<Option<Uuid>, _>("lease_token").is_none());
    assert!(
        row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("lease_expires_at")
            .is_none()
    );
    assert_eq!(
        row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("next_attempt_at")
            .is_some(),
        status == "retry_wait"
    );
    assert_no_published_result(rig, job).await;
}

#[tokio::test]
async fn versioned_result_table_is_required() {
    let rig = TestRig::from_env().await;
    let exists: bool =
        sqlx::query_scalar("SELECT to_regclass('public.asset_integrity_result') IS NOT NULL")
            .fetch_one(&rig.admin_pool)
            .await
            .unwrap();
    assert!(
        exists,
        "the processor result needs an append-only migration"
    );
}

#[tokio::test]
async fn runtime_cannot_write_result_directly_and_public_cannot_complete() {
    let rig = TestRig::from_env().await;
    let job = Uuid::new_v4();
    for statement in [
        "INSERT INTO public.asset_integrity_result(job_id) VALUES($1)",
        "UPDATE public.asset_integrity_result SET media_type='image/png' WHERE job_id=$1",
        "DELETE FROM public.asset_integrity_result WHERE job_id=$1",
    ] {
        let error = sqlx::query(statement)
            .bind(job)
            .execute(&rig.runtime_pool)
            .await
            .unwrap_err();
        assert_eq!(sqlstate(&error).as_deref(), Some("42501"));
    }
    let public_execute: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_proc p \
          CROSS JOIN LATERAL pg_catalog.aclexplode(p.proacl) a \
         WHERE p.oid='public.p0c2_complete_asset_job(uuid,uuid,uuid,uuid,uuid,uuid,text,bigint,text,text)'::regprocedure \
           AND a.grantee=0 AND a.privilege_type='EXECUTE')",
    )
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert!(!public_execute);
}

#[tokio::test]
async fn completion_function_rejects_wrong_identity_and_malformed_digest() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let work = create_work(&rig, &files, actor, space, PNG, "image/png", vec![]).await;
    let accepted: bool =
        sqlx::query_scalar("SELECT public.p0c2_complete_asset_job($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
            .bind(work.job)
            .bind(work.lease.token)
            .bind(Uuid::new_v4())
            .bind(work.block.block_id)
            .bind(work.block.revision_id)
            .bind(work.asset.reference.asset_id)
            .bind(&work.asset.sha256)
            .bind(work.asset.byte_size)
            .bind(&work.asset.media.media_type)
            .bind("a".repeat(64))
            .fetch_one(&rig.runtime_pool)
            .await
            .unwrap();
    assert!(!accepted);
    let error = sqlx::query_scalar::<_, bool>(
        "SELECT public.p0c2_complete_asset_job($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
    )
    .bind(work.job)
    .bind(work.lease.token)
    .bind(space)
    .bind(work.block.block_id)
    .bind(work.block.revision_id)
    .bind(work.asset.reference.asset_id)
    .bind(&work.asset.sha256)
    .bind(work.asset.byte_size)
    .bind(&work.asset.media.media_type)
    .bind("not-a-digest")
    .fetch_one(&rig.runtime_pool)
    .await
    .unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("22023"));
    let status: String = sqlx::query_scalar("SELECT status FROM public.job WHERE id=$1")
        .bind(work.job)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    assert_eq!(status, "running");
    assert_no_published_result(&rig, work.job).await;
    assert_eq!(
        files.processor(&rig).process(&work.lease).await.unwrap(),
        AssetProcessOutcome::Succeeded
    );
}

#[tokio::test]
async fn result_schema_is_versioned_and_readable_after_a_successful_process() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    for (bytes, mime) in [
        (PNG, "image/png"),
        (PDF, "application/pdf"),
        (NOTEBOOK, "application/x-ipynb+json"),
    ] {
        let work = create_work(&rig, &files, actor, space, bytes, mime, vec![]).await;
        assert_eq!(
            files.processor(&rig).process(&work.lease).await.unwrap(),
            AssetProcessOutcome::Succeeded
        );
        let row = sqlx::query(
            "SELECT r.processor_version,r.result_version,r.space_id,r.asset_id, \
                    r.sha256,r.byte_size,r.media_type,r.output_digest,j.output_digest AS job_digest \
               FROM public.asset_integrity_result r \
               JOIN public.job j ON j.id=r.job_id WHERE r.job_id=$1",
        )
        .bind(work.job)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
        assert_eq!(row.get::<i32, _>("processor_version"), 1);
        assert_eq!(row.get::<i32, _>("result_version"), 1);
        assert_eq!(row.get::<Uuid, _>("space_id"), space);
        assert_eq!(
            row.get::<Uuid, _>("asset_id"),
            work.asset.reference.asset_id
        );
        assert_eq!(row.get::<String, _>("sha256"), work.asset.sha256);
        assert_eq!(row.get::<i64, _>("byte_size"), bytes.len() as i64);
        assert_eq!(row.get::<String, _>("media_type"), mime);
        let digest = row.get::<String, _>("output_digest");
        assert_eq!(digest.len(), 64);
        assert_eq!(row.get::<Option<String>, _>("job_digest"), Some(digest));
        // Result-v1 digest contract: SHA-256 of canonical_json over precisely
        // these versioned fields; the digest field itself is excluded.
        let contract = json!({
            "version": 1,
            "kind": "asset_integrity",
            "processor_version": 1,
            "space_id": row.get::<Uuid, _>("space_id"),
            "block_id": work.block.block_id,
            "revision_id": work.block.revision_id,
            "asset_id": row.get::<Uuid, _>("asset_id"),
            "sha256": row.get::<String, _>("sha256"),
            "byte_size": row.get::<i64, _>("byte_size"),
            "media_type": row.get::<String, _>("media_type"),
        });
        assert_eq!(
            row.get::<String, _>("output_digest"),
            hex_digest(canonical_json(&contract).as_bytes())
        );
        assert_eq!(fs::read(files.asset_path(&work.asset)).unwrap(), bytes);
    }
}

#[tokio::test]
async fn revoked_actor_or_hidden_necessary_reference_cannot_publish() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let revoked = create_work(&rig, &files, actor, space, PNG, "image/png", vec![]).await;
    rig.revoke(actor, space).await;
    assert_eq!(
        files.processor(&rig).process(&revoked.lease).await.unwrap(),
        AssetProcessOutcome::Failed
    );
    assert_failure_state(&rig, revoked.job, "failed", "invalid_input").await;

    let (actor, space) = rig.seed_actor_space(true).await;
    let (other, dependency_space) = rig.seed_actor_space(true).await;
    let dependency = rig
        .store
        .create(
            other,
            dependency_space,
            support::command("hidden prerequisite"),
        )
        .await
        .unwrap();
    rig.grant(actor, dependency_space, false).await;
    let hidden = create_work(
        &rig,
        &files,
        actor,
        space,
        PNG,
        "image/png",
        vec![ExactRef::Block(BlockRef {
            block_id: dependency.block_id,
            revision_id: dependency.revision_id,
        })],
    )
    .await;
    rig.revoke(actor, dependency_space).await;
    assert_eq!(
        files.processor(&rig).process(&hidden.lease).await.unwrap(),
        AssetProcessOutcome::Failed
    );
    assert_failure_state(&rig, hidden.job, "failed", "invalid_input").await;
}

#[tokio::test]
async fn a_forged_cross_space_event_cannot_reuse_an_authorized_exact_block() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, real_space) = rig.seed_actor_space(true).await;
    let (_, false_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, false_space, false).await;
    let genuine = create_work(&rig, &files, actor, real_space, PNG, "image/png", vec![]).await;
    let forged =
        JobInput::asset_integrity(actor.actor_id, false_space, genuine.block.clone()).unwrap();
    sqlx::query("INSERT INTO public.job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) VALUES($1,$2,'asset_integrity_requested',1,$3,$4,1)")
        .bind(Uuid::new_v4())
        .bind(forged.business_key())
        .bind(forged.to_value())
        .bind(actor.actor_id)
        .execute(&rig.runtime_pool)
        .await
        .unwrap();
    let jobs = JobStore::new(rig.runtime_pool.clone());
    assert!(jobs.dispatch_pending(256).await.unwrap() >= 1);
    let forged_job: Uuid = sqlx::query_scalar("SELECT id FROM public.job WHERE idempotency_key=$1")
        .bind(forged.business_key())
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    let lease = jobs
        .claim(forged_job, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        files.processor(&rig).process(&lease).await.unwrap(),
        AssetProcessOutcome::Failed
    );
    assert_failure_state(&rig, forged_job, "failed", "invalid_input").await;
    assert_eq!(
        AssetStore::new(rig.runtime_pool.clone(), files.store.clone())
            .read_for_use(actor, learning_core::AssetUseRef::Block(genuine.block))
            .await
            .unwrap()
            .unwrap()
            .reference
            .space_id,
        real_space
    );
}

#[tokio::test]
async fn corrupt_or_missing_bytes_do_not_rollback_originals_or_content() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    for damage in ["changed_length", "same_length", "missing"] {
        let files = Files::new();
        let work = create_work(&rig, &files, actor, space, PDF, "application/pdf", vec![]).await;
        let path = files.asset_path(&work.asset);
        match damage {
            "changed_length" => fs::write(&path, b"corrupted").unwrap(),
            "same_length" => {
                fs::write(&path, vec![b'X'; PDF.len()]).unwrap();
                assert_eq!(fs::metadata(&path).unwrap().len(), PDF.len() as u64);
            }
            "missing" => fs::remove_file(&path).unwrap(),
            _ => unreachable!(),
        }
        assert_eq!(
            files.processor(&rig).process(&work.lease).await.unwrap(),
            AssetProcessOutcome::RetryWait
        );
        assert_failure_state(&rig, work.job, "retry_wait", "transient_storage").await;
        let rows: (i64, String, i64, i64, i64, i64, Uuid) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM public.asset WHERE id=$1),\
                    (SELECT sha256 FROM public.asset WHERE id=$1),\
                    (SELECT count(*) FROM public.upload_receipt WHERE asset_id=$1),\
                    (SELECT count(*) FROM public.block_revision WHERE id=$2),\
                    (SELECT count(*) FROM public.block_asset_use WHERE revision_id=$2),\
                    (SELECT count(*) FROM public.job_outbox WHERE business_key=$3),\
                    (SELECT head_revision_id FROM public.block WHERE id=$4)",
        )
        .bind(work.asset.reference.asset_id)
        .bind(work.block.revision_id)
        .bind(
            JobInput::asset_integrity(actor.actor_id, space, work.block.clone())
                .unwrap()
                .business_key(),
        )
        .bind(work.block.block_id)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
        assert_eq!(
            rows,
            (1, work.asset.sha256, 1, 1, 1, 1, work.block.revision_id)
        );
    }
}

#[tokio::test]
async fn repaired_bytes_retry_the_same_job_and_publish_one_result() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let work = create_work(&rig, &files, actor, space, PDF, "application/pdf", vec![]).await;
    let path = files.asset_path(&work.asset);
    fs::remove_file(&path).unwrap();
    let processor = files.processor(&rig);
    assert_eq!(
        processor.process(&work.lease).await.unwrap(),
        AssetProcessOutcome::RetryWait
    );
    assert_failure_state(&rig, work.job, "retry_wait", "transient_storage").await;
    fs::write(&path, PDF).unwrap();
    // The retry backoff is one database-clock second after attempt 1.
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let second = JobStore::new(rig.runtime_pool.clone())
        .claim(work.job, Duration::from_secs(30))
        .await
        .unwrap()
        .expect("repaired job should be due after its first backoff");
    assert_eq!(second.attempt_count, 2);
    assert_ne!(second.token, work.lease.token);
    assert_eq!(
        processor.process(&second).await.unwrap(),
        AssetProcessOutcome::Succeeded
    );
    let row = sqlx::query(
        "SELECT j.status,j.attempt_count,j.output_digest,count(r.job_id) AS result_count \
         FROM public.job j LEFT JOIN public.asset_integrity_result r ON r.job_id=j.id \
         WHERE j.id=$1 GROUP BY j.id",
    )
    .bind(work.job)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("status"), "succeeded");
    assert_eq!(row.get::<i32, _>("attempt_count"), 2);
    assert!(row.get::<Option<String>, _>("output_digest").is_some());
    assert_eq!(row.get::<i64, _>("result_count"), 1);
}

#[tokio::test]
async fn revoke_after_preparation_is_rechecked_before_publication() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let work = create_work(&rig, &files, actor, space, PNG, "image/png", vec![]).await;
    let processor = files.processor(&rig);
    let prepared = processor.prepare(&work.lease).await.unwrap();
    rig.revoke(actor, space).await;
    assert_eq!(
        processor.publish(prepared).await.unwrap(),
        AssetProcessOutcome::Failed
    );
    assert_failure_state(&rig, work.job, "failed", "invalid_input").await;
}

#[tokio::test]
async fn stale_attempt_and_repeat_completion_cannot_publish_another_result() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let work = create_work(&rig, &files, actor, space, PNG, "image/png", vec![]).await;
    let processor = files.processor(&rig);
    let stale_prepared = processor.prepare(&work.lease).await.unwrap();
    sqlx::query(
        "UPDATE public.job SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(work.job)
    .execute(&rig.admin_pool)
    .await
    .unwrap();
    let jobs = JobStore::new(rig.runtime_pool.clone());
    let current = jobs
        .claim(work.job, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(work.lease.token, current.token);
    assert_eq!(
        processor.publish(stale_prepared).await.unwrap(),
        AssetProcessOutcome::LeaseLost
    );
    assert_no_published_result(&rig, work.job).await;
    assert_eq!(
        processor.process(&current).await.unwrap(),
        AssetProcessOutcome::Succeeded
    );
    assert_eq!(
        processor.process(&current).await.unwrap(),
        AssetProcessOutcome::LeaseLost
    );
    let row = sqlx::query("SELECT count(*) AS result_count,min(output_digest) AS digest FROM public.asset_integrity_result WHERE job_id=$1")
        .bind(work.job)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    assert_eq!(row.get::<i64, _>("result_count"), 1);
    let digest = row.get::<Option<String>, _>("digest").unwrap();
    let job_digest: Option<String> =
        sqlx::query_scalar("SELECT output_digest FROM public.job WHERE id=$1")
            .bind(work.job)
            .fetch_one(&rig.admin_pool)
            .await
            .unwrap();
    assert_eq!(job_digest, Some(digest));
}
