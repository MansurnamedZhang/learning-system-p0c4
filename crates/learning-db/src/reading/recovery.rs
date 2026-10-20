//! Read-only application semantics on the management executor's held target.
//! This grants no target, restore, activation or ordinary-runtime connection
//! authority. SQL role/catalog/CONNECT checks remain a separate recovery gate.
use super::{ReadingStore, read::project_in_transaction};
use crate::storage;
use learning_core::{ContentError, Principal, ReadingMode, ReadingRef, canonical_json, hex_digest};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use std::time::{Duration, Instant};
use uuid::Uuid;

const MAX_VIEWS: usize = 100_000;
const MAX_PROJECTION_BYTES: u64 = 64 * 1024 * 1024;

pub struct RecoveryReadingReport {
    evidence: Value,
    canonical_sha256: String,
}
impl RecoveryReadingReport {
    pub fn evidence(&self) -> &Value {
        &self.evidence
    }
    pub fn canonical_sha256(&self) -> &str {
        &self.canonical_sha256
    }
}

impl ReadingStore {
    /// Management-only observation on the caller's *already authenticated held*
    /// transaction. Uses the exact runtime projection core; never commits,
    /// changes grants, starts a pool, writes a request or activates a worker.
    /// A report alone cannot authorize any recovery or production operation.
    pub async fn validate_recovery_views(
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<RecoveryReadingReport, ContentError> {
        tokio::time::timeout(
            Duration::from_secs(900),
            Self::validate_recovery_views_inner(tx),
        )
        .await
        .map_err(|_| ContentError::Storage)?
    }

    async fn validate_recovery_views_inner(
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<RecoveryReadingReport, ContentError> {
        let end = Instant::now() + Duration::from_secs(900);
        sqlx::query("SET LOCAL statement_timeout='15s'")
            .execute(&mut **tx)
            .await
            .map_err(storage)?;
        let users: (String, String) =
            sqlx::query_as("SELECT current_user::text, session_user::text")
                .fetch_one(&mut **tx)
                .await
                .map_err(storage)?;
        if users != ("learning_admin".into(), "learning_admin".into()) {
            return Err(ContentError::Invalid(
                "recovery_reading_management_role".into(),
            ));
        }
        let rows: Vec<(Uuid, Uuid, Uuid, bool, bool)> = sqlx::query_as(
            "SELECT v.view_id,v.id,o.owner_id,EXISTS(SELECT 1 FROM public.space_grant g WHERE g.space_id=o.space_id AND g.actor_id=o.owner_id),v.id=h.head_revision_id FROM public.reading_view_revision v JOIN public.reading_view h ON h.id=v.view_id JOIN public.overlay o ON o.id=v.overlay_id ORDER BY v.view_id,v.id LIMIT 100001"
        ).fetch_all(&mut **tx).await.map_err(storage)?;
        if rows.len() > MAX_VIEWS {
            return Err(ContentError::Invalid(
                "recovery_reading_view_capacity".into(),
            ));
        }
        let denied_actor = loop {
            let id = Uuid::new_v4();
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.app_user WHERE id=$1)")
                    .bind(id)
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(storage)?;
            if !exists {
                break Principal { actor_id: id };
            }
            if Instant::now() >= end {
                return Err(ContentError::Storage);
            }
        };
        // Keep only framed full-output digests, bounded by three modes per
        // admitted view; the complete output byte budget is still charged.
        let mut observations = b"knowweave-recovery-reading-projections-v1\0".to_vec();
        let mut bytes = 0_u64;
        let mut owner_present = 0_u64;
        let mut owner_denied = 0_u64;
        let mut nonowner_denied = 0_u64;
        let mut granted_nonowner_denied = 0_u64;
        let mut views_without_granted_nonowner = 0_u64;
        let mut current = 0_u64;
        let mut historical = 0_u64;
        for (view_id, revision_id, owner, granted, head) in &rows {
            if *head {
                current += 1;
            } else {
                historical += 1;
            }
            let granted_peer: Option<Uuid> = sqlx::query_scalar(
                "SELECT g.actor_id FROM public.space_grant g JOIN public.app_user u ON u.id=g.actor_id JOIN public.overlay o ON o.space_id=g.space_id JOIN public.reading_view_revision v ON v.overlay_id=o.id WHERE v.view_id=$1 AND v.id=$2 AND g.actor_id<>$3 ORDER BY g.actor_id LIMIT 1"
            ).bind(view_id).bind(revision_id).bind(owner)
                .fetch_optional(&mut **tx).await.map_err(storage)?;
            if granted_peer.is_none() {
                views_without_granted_nonowner += 1;
            }
            for mode in [
                ReadingMode::Original,
                ReadingMode::Personal,
                ReadingMode::Fused,
            ] {
                if Instant::now() >= end {
                    return Err(ContentError::Storage);
                }
                let reference = ReadingRef {
                    view_id: *view_id,
                    revision_id: *revision_id,
                };
                let result = project_in_transaction(
                    tx,
                    Principal { actor_id: *owner },
                    reference.clone(),
                    mode,
                )
                .await?;
                if result.is_some() != *granted
                    || result.as_ref().is_some_and(|r| r.view != reference)
                {
                    return Err(ContentError::Invalid(
                        "recovery_reading_owner_projection".into(),
                    ));
                }
                if result.is_some() {
                    owner_present += 1;
                } else {
                    owner_denied += 1;
                }
                let denial =
                    project_in_transaction(tx, denied_actor, reference.clone(), mode).await?;
                if denial.is_some() {
                    return Err(ContentError::Invalid(
                        "recovery_reading_nonowner_visible".into(),
                    ));
                }
                nonowner_denied += 1;
                if let Some(peer) = granted_peer {
                    if project_in_transaction(
                        tx,
                        Principal { actor_id: peer },
                        reference.clone(),
                        mode,
                    )
                    .await?
                    .is_some()
                    {
                        return Err(ContentError::Invalid(
                            "recovery_reading_granted_nonowner_visible".into(),
                        ));
                    }
                    granted_nonowner_denied += 1;
                }
                let raw =
                    canonical_json(&json!({"view":reference,"mode":mode,"projection":result}));
                bytes = bytes
                    .checked_add(raw.len() as u64)
                    .ok_or(ContentError::Storage)?;
                if bytes > MAX_PROJECTION_BYTES {
                    return Err(ContentError::Invalid(
                        "recovery_reading_projection_capacity".into(),
                    ));
                }
                observations.extend_from_slice(&(raw.len() as u64).to_be_bytes());
                observations.extend_from_slice(hex_digest(raw.as_bytes()).as_bytes());
            }
        }
        let projection_sha256 = hex_digest(&observations);
        let evidence = json!({"format_version":1,"capability":"recovery_application_read_projections_v1",
            "execution_context":"EXACT_HELD_MANAGEMENT_TRANSACTION_NOT_ORDINARY_RUNTIME_CONNECTION",
            "sql_runtime_privileges":"SEPARATE_ROLE_CATALOG_AND_CONNECT_GATE",
            "view_revisions":rows.len(),"current_revisions":current,"historical_revisions":historical,
            "owner_present":owner_present,"owner_without_grant_denied":owner_denied,
            "nonexistent_actor_denied":nonowner_denied,"existing_granted_nonowner_denied":granted_nonowner_denied,
            "views_without_existing_granted_nonowner":views_without_granted_nonowner,
            "modes":["original","personal","fused"],
            "projection_bytes":bytes,"projection_sha256":projection_sha256,
            "business_writes":0,"preview_generator":"ABSENT_NOT_REBUILT","search_generator":"ABSENT_NOT_REBUILT"});
        let canonical_sha256 = hex_digest(canonical_json(&evidence).as_bytes());
        Ok(RecoveryReadingReport {
            evidence,
            canonical_sha256,
        })
    }
}
