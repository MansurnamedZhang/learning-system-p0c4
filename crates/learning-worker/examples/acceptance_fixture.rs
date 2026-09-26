//! Isolated P0-C2 acceptance control plane. Never run this inside a Worker
//! container: `seed` and `revoke` require an administrative database login.
//! No command writes job or outbox rows directly.
use learning_assets::{FsAssetStore, UploadDeclaration};
#[cfg(unix)]
use learning_core::hex_digest;
use learning_core::{
    BlockRef, BodyV3, ContentDraft, ContentV3, CreateContent, Intent, JobInput, Principal,
};
use learning_db::{
    AssetMedia, AssetStore, JobFailureClass, JobStore, MIGRATOR, VersionedContentStore,
};
use serde_json::json;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
    time::Duration,
};
use uuid::Uuid;

// The same minimal PNG signature accepted by the repository's asset tests.
const PNG: &[u8] = &[137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 0, 73, 69, 78, 68];

#[derive(Debug)]
struct SeededJob {
    job_id: Uuid,
    actor_id: Uuid,
    space_id: Uuid,
    block_id: Uuid,
    revision_id: Uuid,
    asset_id: Uuid,
    asset_sha256: String,
    business_key: String,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("acceptance fixture error: {error}");
        process::exit(78);
    }
}

async fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        return Err(usage());
    };
    let (admin, runtime) = match command {
        "seed" | "revoke" | "fence" | "capture-token" => {
            let admin = connect("ADMIN_DATABASE_URL").await?;
            let runtime = connect("DATABASE_URL").await?;
            (Some(admin), runtime)
        }
        "snapshot" | "cancel" => (None, connect("DATABASE_URL").await?),
        _ => return Err(usage()),
    };
    match (command, args.as_slice()) {
        ("seed", [_]) => {
            let admin = admin.as_ref().ok_or("admin connection required")?;
            MIGRATOR
                .run(admin)
                .await
                .map_err(|_| "isolated schema migration failed")?;
            let files = FsAssetStore::new(path("ASSET_ROOT")?, path("STAGING_ROOT")?)
                .map_err(|_| "fixture asset roots unavailable")?;
            let seeded = seed_fixture(admin, &runtime, &files).await?;
            println!(
                "{}",
                json!({"job_id":seeded.job_id,"actor_id":seeded.actor_id,
                    "space_id":seeded.space_id,"block_id":seeded.block_id,
                    "revision_id":seeded.revision_id,"asset_id":seeded.asset_id,
                    "asset_sha256":seeded.asset_sha256,"business_key":seeded.business_key})
            );
        }
        ("snapshot", [_, job]) => {
            let job = parse_uuid(job)?;
            snapshot(&runtime, job).await?;
        }
        ("capture-token", [_, job]) => {
            let job = parse_uuid(job)?;
            let (status, attempt, token, current, output, results): (
                String,
                i32,
                Option<Uuid>,
                bool,
                Option<String>,
                i64,
            ) = sqlx::query_as(
                "SELECT j.status,j.attempt_count,j.lease_token,\
                        COALESCE(j.lease_expires_at>clock_timestamp(),false),\
                        j.output_digest,\
                        (SELECT count(*) FROM public.asset_integrity_result WHERE job_id=j.id) \
                   FROM public.job AS j WHERE j.id=$1",
            )
            .bind(job)
            .fetch_one(admin.as_ref().ok_or("admin connection required")?)
            .await
            .map_err(|_| "first-lease capture preflight failed")?;
            if status != "running" || attempt != 1 || !current || output.is_some() || results != 0 {
                return Err(
                    "first-lease capture requires an active unprocessed first attempt".into(),
                );
            }
            let token = token.ok_or("first lease has no token")?;
            let file_hash = create_token_file(&path("OLD_LEASE_TOKEN_FILE")?, token)?;
            let still_current: bool = sqlx::query_scalar(
                "SELECT status='running' AND attempt_count=1 \
                        AND lease_token=$2 AND lease_expires_at>clock_timestamp() \
                        AND output_digest IS NULL \
                   FROM public.job WHERE id=$1",
            )
            .bind(job)
            .bind(token)
            .fetch_one(admin.as_ref().ok_or("admin connection required")?)
            .await
            .map_err(|_| "first-lease capture postflight failed")?;
            if !still_current {
                return Err("first lease changed during token capture".into());
            }
            println!(
                "{}",
                json!({"captured":true,"attempt_count":1,
                "lease_current":true,"token_file_sha256":file_hash})
            );
        }
        ("fence", [_, job]) => {
            let job = parse_uuid(job)?;
            let (old_token, file_hash) = read_token_file(&path("OLD_LEASE_TOKEN_FILE")?)?;
            let (status, attempt, current_token, lease_current): (String, i32, Option<Uuid>, bool) =
                sqlx::query_as(
                    "SELECT status,attempt_count,lease_token,\
                        COALESCE(lease_expires_at>clock_timestamp(),false) \
                   FROM public.job WHERE id=$1",
                )
                .bind(job)
                .fetch_one(admin.as_ref().ok_or("admin connection required")?)
                .await
                .map_err(|_| "old-token preflight failed")?;
            if status != "running"
                || attempt != 2
                || !lease_current
                || current_token.is_none()
                || current_token == Some(old_token)
            {
                return Err("old-token preflight requires a current distinct second lease".into());
            }
            assert_old_token_rejected(&runtime, job, old_token).await?;
            let still_current: bool = sqlx::query_scalar(
                "SELECT status='running' AND attempt_count=2 \
                        AND lease_token=$2 AND lease_expires_at>clock_timestamp() \
                   FROM public.job WHERE id=$1",
            )
            .bind(job)
            .bind(current_token)
            .fetch_one(admin.as_ref().ok_or("admin connection required")?)
            .await
            .map_err(|_| "second-lease postflight failed")?;
            if !still_current {
                return Err("second lease changed during old-token checks".into());
            }
            let (saved_token, final_file_hash) = read_token_file(&path("OLD_LEASE_TOKEN_FILE")?)?;
            if saved_token != old_token || final_file_hash != file_hash {
                return Err("old-token file changed during fencing checks".into());
            }
            println!(
                "{}",
                json!({"old_token_renew":false,"checkpoint":false,"succeed":false,
                    "fail":false,"token_file_sha256":file_hash})
            );
        }
        ("cancel", [_, job]) => {
            let cancelled = JobStore::new(runtime)
                .cancel(parse_uuid(job)?)
                .await
                .map_err(|_| "cancel failed")?;
            if !cancelled {
                return Err("job was not cancellable".into());
            }
            println!("{}", json!({"cancelled":true}));
        }
        ("revoke", [_, actor, space]) => {
            let actor = parse_uuid(actor)?;
            let space = parse_uuid(space)?;
            let deleted =
                sqlx::query("DELETE FROM public.space_grant WHERE actor_id=$1 AND space_id=$2")
                    .bind(actor)
                    .bind(space)
                    .execute(admin.as_ref().ok_or("admin connection required")?)
                    .await
                    .map_err(|_| "grant revocation failed")?
                    .rows_affected();
            if deleted != 1 {
                return Err("expected exactly one grant to revoke".into());
            }
            println!("{}", json!({"revoked":true}));
        }
        _ => return Err(usage()),
    }
    Ok(())
}

fn usage() -> String {
    "usage: acceptance_fixture seed | snapshot JOB_ID | capture-token JOB_ID | fence JOB_ID | cancel JOB_ID | revoke ACTOR_ID SPACE_ID".into()
}

fn parse_uuid(value: &str) -> Result<Uuid, String> {
    value.parse().map_err(|_| "invalid UUID".into())
}

fn path(name: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(env::var_os(name).ok_or_else(|| format!("{name} required"))?);
    if !path.is_absolute() {
        return Err(format!("{name} must be absolute"));
    }
    Ok(path)
}

#[cfg(unix)]
fn create_token_file(path: &Path, token: Uuid) -> Result<String, String> {
    use std::{
        fs::OpenOptions,
        io::Write,
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| "old-token file must be new and private")?;
    file.write_all(token.to_string().as_bytes())
        .map_err(|_| "old-token file write failed")?;
    file.sync_all().map_err(|_| "old-token file sync failed")?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| "old-token file permission setup failed")?;
    drop(file);
    let (saved, digest) = read_token_file(path)?;
    if saved != token {
        return Err("old-token file verification failed".into());
    }
    Ok(digest)
}

#[cfg(not(unix))]
fn create_token_file(_: &Path, _: Uuid) -> Result<String, String> {
    Err("token capture requires an isolated Unix host".into())
}

#[cfg(unix)]
fn read_token_file(path: &Path) -> Result<(Uuid, String), String> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = fs::symlink_metadata(path).map_err(|_| "old-token file unavailable")?;
    if !metadata.file_type().is_file() || metadata.permissions().mode() & 0o777 != 0o600 {
        return Err("old-token file must be a regular 0600 file".into());
    }
    let bytes = fs::read(path).map_err(|_| "old-token file read failed")?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "old-token file is not UTF-8")?;
    let token = parse_uuid(text)?;
    Ok((token, hex_digest(&bytes)))
}

#[cfg(not(unix))]
fn read_token_file(_: &Path) -> Result<(Uuid, String), String> {
    Err("token fencing requires an isolated Unix host".into())
}

async fn connect(name: &str) -> Result<PgPool, String> {
    let url = env::var(name).map_err(|_| format!("{name} required"))?;
    PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .map_err(|_| format!("{name} connection failed"))
}

async fn seed_fixture(
    admin: &PgPool,
    runtime: &PgPool,
    files: &FsAssetStore,
) -> Result<SeededJob, String> {
    let actor = Principal {
        actor_id: Uuid::new_v4(),
    };
    let space_id = Uuid::new_v4();
    // These are the only management writes in the fixture setup. Asset,
    // content, outbox and job state all flow through their public APIs.
    sqlx::query("INSERT INTO public.app_user VALUES ($1)")
        .bind(actor.actor_id)
        .execute(admin)
        .await
        .map_err(|_| "actor setup failed")?;
    sqlx::query("INSERT INTO public.space VALUES ($1,$2)")
        .bind(space_id)
        .bind(actor.actor_id)
        .execute(admin)
        .await
        .map_err(|_| "space setup failed")?;
    sqlx::query("INSERT INTO public.space_grant VALUES ($1,$2,true)")
        .bind(actor.actor_id)
        .bind(space_id)
        .execute(admin)
        .await
        .map_err(|_| "grant setup failed")?;

    let source = env::temp_dir().join(format!("knowweave-acceptance-{}.png", Uuid::new_v4()));
    fs::write(&source, PNG).map_err(|_| "fixture source write failed")?;
    let blob = files
        .put_from_file(
            Uuid::new_v4(),
            &source,
            UploadDeclaration {
                expected_size_bytes: PNG.len() as u64,
                max_size_bytes: 1024,
            },
        )
        .map_err(|_| "fixture asset upload failed")?;
    fs::remove_file(source).map_err(|_| "fixture source cleanup failed")?;
    let asset = AssetStore::new(runtime.clone(), files.clone())
        .register_verified(
            actor,
            space_id,
            Uuid::new_v4(),
            blob,
            AssetMedia {
                media_type: "image/png".into(),
                original_file_name: "acceptance.png".into(),
            },
        )
        .await
        .map_err(|_| "fixture asset registration failed")?;
    let content = VersionedContentStore::new(runtime.clone())
        .create(
            actor,
            space_id,
            CreateContent {
                request_id: Uuid::new_v4(),
                draft: ContentDraft::V3(ContentV3 {
                    intent: Intent::Note,
                    language: "en".into(),
                    title: "Worker container acceptance".into(),
                    body: BodyV3::Figure {
                        asset: asset.reference.clone(),
                        usage: "lecture_diagram".into(),
                        caption: "Acceptance fixture".into(),
                        alt: "Asset integrity check".into(),
                        decorative: false,
                    },
                    basis_refs: vec![],
                    requires_context: vec![],
                    source_run: None,
                }),
                reason: "worker container acceptance".into(),
            },
        )
        .await
        .map_err(|_| "fixture content creation failed")?;
    let input = JobInput::asset_integrity(
        actor.actor_id,
        space_id,
        BlockRef {
            block_id: content.block_id,
            revision_id: content.revision_id,
        },
    )
    .map_err(|_| "fixture job input invalid")?;
    let jobs = JobStore::new(runtime.clone());
    if jobs
        .dispatch_pending(256)
        .await
        .map_err(|_| "fixture dispatch failed")?
        == 0
    {
        return Err("fixture event was not dispatched".into());
    }
    // Read-only lookup; job and outbox are never inserted by fixture SQL.
    let job_id: Uuid = sqlx::query_scalar("SELECT id FROM public.job WHERE idempotency_key=$1")
        .bind(input.business_key())
        .fetch_one(runtime)
        .await
        .map_err(|_| "dispatched job lookup failed")?;
    let job = jobs
        .get(job_id)
        .await
        .map_err(|_| "dispatched job invalid")?
        .ok_or("dispatched job missing")?;
    if job.input != input || job.attempt_count != 0 {
        return Err("dispatched job identity changed".into());
    }
    Ok(SeededJob {
        job_id,
        actor_id: actor.actor_id,
        space_id,
        block_id: content.block_id,
        revision_id: content.revision_id,
        asset_id: asset.reference.asset_id,
        asset_sha256: asset.sha256,
        business_key: input.business_key(),
    })
}

async fn snapshot(runtime: &PgPool, job: Uuid) -> Result<(), String> {
    let row: (String, i32, Option<String>, bool, i64) = sqlx::query_as(
        "SELECT j.status,j.attempt_count,j.output_digest,\
                COALESCE(j.lease_expires_at<=clock_timestamp(),false),\
                (SELECT count(*) FROM public.asset_integrity_result WHERE job_id=j.id) \
           FROM public.job AS j WHERE j.id=$1",
    )
    .bind(job)
    .fetch_one(runtime)
    .await
    .map_err(|_| "job snapshot failed")?;
    println!(
        "{}",
        json!({"status":row.0,"attempt_count":row.1,
        "output_digest":row.2,"lease_expired_by_db_clock":row.3,"result_count":row.4})
    );
    Ok(())
}

async fn assert_old_token_rejected(runtime: &PgPool, job: Uuid, token: Uuid) -> Result<(), String> {
    let jobs = JobStore::new(runtime.clone());
    let renew = jobs
        .renew(job, token, Duration::from_secs(30))
        .await
        .map_err(|_| "old-token renew check failed")?;
    let checkpoint = jobs
        .checkpoint(job, token, json!({"acceptance": "stale"}))
        .await
        .map_err(|_| "old-token checkpoint check failed")?;
    let succeed = jobs
        .succeed(job, token, &"a".repeat(64))
        .await
        .map_err(|_| "old-token succeed check failed")?;
    let fail = jobs
        .fail(job, token, JobFailureClass::TransientStorage)
        .await
        .map_err(|_| "old-token fail check failed")?;
    if renew || checkpoint || succeed || fail {
        return Err("stale token altered a job".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use learning_core::JobStatus;

    #[cfg(unix)]
    #[test]
    fn captured_token_is_private_and_cannot_be_overwritten() {
        use std::os::unix::fs::PermissionsExt;
        let file = env::temp_dir().join(format!("knowweave-old-token-{}", Uuid::new_v4()));
        let token = Uuid::new_v4();
        let hash = create_token_file(&file, token).unwrap();
        assert_eq!(read_token_file(&file).unwrap(), (token, hash));
        assert!(create_token_file(&file, Uuid::new_v4()).is_err());
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_token_file(&file).is_err());
        fs::remove_file(file).unwrap();
    }

    #[tokio::test]
    async fn seed_creates_exact_business_job_without_direct_queue_writes() {
        let admin = connect("TEST_ADMIN_DATABASE_URL").await.unwrap();
        MIGRATOR.run(&admin).await.unwrap();
        let runtime = connect("TEST_DATABASE_URL").await.unwrap();
        let root = env::temp_dir().join(format!("knowweave-acceptance-test-{}", Uuid::new_v4()));
        let files = FsAssetStore::new(root.join("assets"), root.join("staging")).unwrap();
        let seeded = seed_fixture(&admin, &runtime, &files).await.unwrap();
        let job = JobStore::new(runtime.clone())
            .get(seeded.job_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job.status, JobStatus::Queued);
        assert_eq!(job.attempt_count, 0);
        assert_eq!(job.input.actor_id(), seeded.actor_id);
        assert_eq!(job.input.space_id(), seeded.space_id);
        assert_eq!(
            job.input
                .asset_block()
                .expect("asset job required")
                .block_id,
            seeded.block_id
        );
        assert_eq!(
            job.input
                .asset_block()
                .expect("asset job required")
                .revision_id,
            seeded.revision_id
        );
        let row: (i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM public.job_outbox WHERE id=$1),\
                    (SELECT count(*) FROM public.block_asset_use WHERE space_id=$2 AND block_id=$3 AND revision_id=$4 AND asset_id=$5),\
                    (SELECT count(*) FROM public.asset_integrity_result WHERE job_id=$6)",
        )
        .bind(job.outbox_id)
        .bind(seeded.space_id)
        .bind(seeded.block_id)
        .bind(seeded.revision_id)
        .bind(seeded.asset_id)
        .bind(seeded.job_id)
        .fetch_one(&runtime)
        .await
        .unwrap();
        assert_eq!(row, (1, 1, 0));
        fs::remove_dir_all(root).unwrap();
    }
}
