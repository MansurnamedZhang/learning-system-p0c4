//! Fresh current-invocation recovery bodies; private inside the held importer.
use super::*;
use serde_json::json;
use uuid::Uuid;

tokio::task_local! { static RECOVERY: (); }

pub(crate) async fn leases() -> Result<(), BackupError> {
    RECOVERY
        .scope((), real_full_import_rehearsal(FullFault::Success))
        .await
}

pub(super) async fn seed(pool: &PgPool) -> Result<(), BackupError> {
    if RECOVERY.try_with(|_| ()).is_err() {
        return Ok(());
    }
    let (actor, space): (Uuid, Uuid) =
        sqlx::query_as("SELECT owner_id,id FROM public.space ORDER BY id LIMIT 1")
            .fetch_one(pool)
            .await?;
    for snapshot in [false, true] {
        for expired in [false, true] {
            let id = Uuid::new_v4();
            let block = Uuid::new_v4();
            let revision = Uuid::new_v4();
            let (event, payload, key) = if snapshot {
                let request = json!({"reading":{"view_id":block,"revision_id":revision},"mode":"original","include_personal":false,"include_originals":false,"resource_versions":[],"source_segments":[]});
                let canonical: String = sqlx::query_scalar("SELECT public.p0c3_canonical_json($1)")
                    .bind(&request)
                    .fetch_one(pool)
                    .await?;
                (
                    "snapshot_export_requested",
                    json!({"version":1,"kind":"snapshot_export","actor_id":actor,"space_id":space,"request":request}),
                    format!(
                        "snapshot-export:v1:{actor}:{space}:{}",
                        hex::encode(Sha256::digest(canonical.as_bytes()))
                    ),
                )
            } else {
                (
                    "asset_integrity_requested",
                    json!({"version":1,"kind":"asset_integrity","actor_id":actor,"space_id":space,"block_id":block,"revision_id":revision}),
                    format!("asset-integrity:v1:{space}:{block}:{revision}"),
                )
            };
            sqlx::query("INSERT INTO public.job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) VALUES($1,$2,$3,1,$4,$5,1)").bind(id).bind(&key).bind(event).bind(payload).bind(actor).execute(pool).await?;
            sqlx::query("INSERT INTO public.job(id,outbox_id,idempotency_key,status) VALUES($1,$1,$2,'queued')").bind(id).bind(key).execute(pool).await?;
            sqlx::query("UPDATE public.job_outbox SET dispatched_at=clock_timestamp() WHERE id=$1")
                .bind(id)
                .execute(pool)
                .await?;
            let claim = if snapshot {
                "SELECT token FROM public.p0c3_claim_snapshot_job($1,300000)"
            } else {
                "SELECT token FROM public.p0c2_claim_job($1,300000)"
            };
            let _: Uuid = sqlx::query_scalar(claim).bind(id).fetch_one(pool).await?;
            sqlx::query("UPDATE public.job SET checkpoint=$2,lease_expires_at=CASE WHEN $3 THEN clock_timestamp()-interval '1 hour' ELSE clock_timestamp()+interval '1 hour' END WHERE id=$1").bind(id).bind(json!({"recovery_fixture":true,"expired":expired})).bind(expired).execute(pool).await?;
        }
    }
    Ok(())
}

pub(super) async fn after_import(
    target: &mut ImportedTarget,
    _package: &VerifiedPackage,
) -> Result<(), BackupError> {
    if RECOVERY.try_with(|_| ()).is_err() {
        return Ok(());
    }
    // Native RED: source leases survive today's import. Task6 must normalize
    // this SAME held target before this assertion can become true.
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM public.job WHERE lease_token IS NOT NULL OR lease_expires_at IS NOT NULL").fetch_one(target.connection()).await?;
    if remaining != 0 {
        return Err(BackupError::Invalid(
            "RECOVERY_SOURCE_LEASES_SURVIVED_IMPORT",
        ));
    }
    Ok(())
}
