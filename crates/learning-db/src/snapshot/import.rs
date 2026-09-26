//! Read-only preflight. Task 6 must repeat authorization/conflict checks inside
//! its publication transaction; this value is never a durable grant.
use super::{
    import_rows::{Package, id, invalid},
    import_schema, rows,
};
use crate::{request, storage};
use learning_assets::{FsAssetStore, SnapshotDirectory, verify_snapshot};
use learning_core::*;
use serde_json::Value;
use sqlx::PgPool;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
};
use uuid::Uuid;

pub struct SnapshotImportStore {
    pub(super) pool: PgPool,
    pub(super) files: FsAssetStore,
}

/// No public constructor or mutable fields: only this preflight can mint it.
pub struct PreparedSnapshotImport {
    pub(super) manifest: ExactSnapshotManifest,
    pub(super) manifest_sha256: String,
    pub(super) rows: Vec<SnapshotRow>,
    pub(super) assets: Vec<VerifiedImportAsset>,
}
pub(super) struct VerifiedImportAsset {
    reference: AssetRef,
    pub(super) sha256: String,
    pub(super) byte_size: u64,
    pub(super) source: File,
}
impl PreparedSnapshotImport {
    pub fn manifest(&self) -> &ExactSnapshotManifest {
        &self.manifest
    }
    pub fn rows(&self) -> &[SnapshotRow] {
        &self.rows
    }
    /// Read-only metadata; retained original handles stay private for Task 6.
    pub fn assets(&self) -> impl Iterator<Item = (&AssetRef, &str, u64)> {
        self.assets.iter().map(|a| {
            let _retained = &a.source;
            (&a.reference, a.sha256.as_str(), a.byte_size)
        })
    }
}
impl SnapshotImportStore {
    pub fn new(pool: PgPool, files: FsAssetStore) -> Self {
        Self { pool, files }
    }
    pub async fn validate_exact(
        &self,
        actor: Principal,
        stage: &SnapshotDirectory,
    ) -> Result<PreparedSnapshotImport, ContentError> {
        let verified = verify_snapshot(stage).map_err(|_| invalid())?;
        let SnapshotManifest::ExactImportV1(manifest) = verified.manifest else {
            return Err(invalid());
        };
        let package = Package {
            rows: &verified.rows,
        };
        package.validate(&manifest.root, actor)?;
        // Rechecked against the pinned manifest hash; files are sealed Linux
        // copies, not paths which a later importer may accidentally reopen.
        let mut files: BTreeMap<_, _> = stage
            .open_verified_files()
            .map_err(|_| invalid())?
            .into_iter()
            .collect();
        let mut tx = request::begin_read(&self.pool).await?;
        check_identities(&mut tx, actor, &package).await?;
        let mut assets = vec![];
        let mut originals = OriginalBudget::default();
        for row in &verified.rows {
            let values = &row.immutable_values;
            let checked = check_row(&mut tx, actor, &package, row).await?;
            let present = checked.present;
            let space = checked.space;
            if row.table == SnapshotTable::Asset {
                let sha256 = values["sha256"].as_str().ok_or_else(invalid)?.to_owned();
                let byte_size = values["byte_size"].as_u64().ok_or_else(invalid)?;
                originals.account(manifest.requires_destination_assets, &sha256, byte_size)?;
                let source = if manifest.requires_destination_assets {
                    let target = present.as_ref().ok_or(ContentError::NotFound)?;
                    self.files
                        .open_record(
                            target["storage_key"].as_str().ok_or_else(invalid)?,
                            &sha256,
                            byte_size as i64,
                        )
                        .map_err(|_| ContentError::NotFound)?
                } else {
                    let path = format!("assets/sha256/{}/{}", &sha256[..2], sha256);
                    if !manifest
                        .files
                        .iter()
                        .any(|f| f.path == path && f.sha256 == sha256 && f.size == byte_size)
                    {
                        return Err(invalid());
                    }
                    let file = files.get(&path).ok_or(ContentError::NotFound)?;
                    // Sealed handles can be cloned for multiple logical asset
                    // identities sharing bytes, without a named-path reopen.
                    file.try_clone().map_err(storage)?
                };
                assets.push(VerifiedImportAsset {
                    reference: AssetRef {
                        space_id: space,
                        asset_id: id(values, "id")?,
                    },
                    sha256,
                    byte_size,
                    source,
                });
            }
        }
        files.clear();
        tx.commit().await.map_err(storage)?;
        Ok(PreparedSnapshotImport {
            manifest,
            manifest_sha256: stage.manifest_sha256().to_owned(),
            rows: verified.rows,
            assets,
        })
    }
}

pub(super) async fn check_identities(
    tx: &mut rows::Tx<'_>,
    actor: Principal,
    package: &Package<'_>,
) -> Result<(), ContentError> {
    let actor_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.app_user WHERE id=$1)")
            .bind(actor.actor_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(storage)?;
    if !actor_exists {
        return Err(ContentError::NotFound);
    }
    let mut users = BTreeSet::from([actor.actor_id]);
    for row in package.rows {
        for link in import_schema::links(row.table)
            .iter()
            .filter(|l| l.target == "app_user")
        {
            users.insert(id(&row.immutable_values, link.columns[0])?);
        }
    }
    let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM public.app_user WHERE id=ANY($1)")
        .bind(users.iter().copied().collect::<Vec<_>>())
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?;
    if existing != users.len() as i64 {
        return Err(ContentError::NotFound);
    }
    Ok(())
}
pub(super) struct CheckedRow {
    pub present: Option<Value>,
    pub space: Uuid,
    pub require_write: bool,
}
pub(super) async fn check_row(
    tx: &mut rows::Tx<'_>,
    actor: Principal,
    package: &Package<'_>,
    row: &SnapshotRow,
) -> Result<CheckedRow, ContentError> {
    let values = &row.immutable_values;
    let schema_ok:bool=sqlx::query_scalar(&format!("WITH t AS (SELECT (jsonb_populate_record(NULL::public.{},$1)).*) SELECT ({}) IS NOT FALSE FROM t", rows::table_name(row.table),import_schema::checks(row.table)))
                .bind(values).fetch_one(&mut **tx).await.map_err(|_|invalid())?;
    if !schema_ok {
        return Err(invalid());
    }

    let present = lookup(tx, row).await?;
    let natural = import_schema::unique_key(row.table);
    if !natural.is_empty() && (row.table != SnapshotTable::OverlayGroup || values["placed"] == true)
    {
        let filter = natural
            .iter()
            .map(|k| format!("coalesce(to_jsonb(t.{k}),'null'::jsonb)=($1::jsonb->'{k}')"))
            .collect::<Vec<_>>()
            .join(" AND ");
        let active = if row.table == SnapshotTable::OverlayGroup {
            " AND t.placed"
        } else {
            ""
        };
        let collision: Option<Value> = sqlx::query_scalar(&format!(
            "SELECT to_jsonb(t) FROM public.{} t WHERE {filter}{active}",
            rows::table_name(row.table)
        ))
        .bind(values)
        .fetch_optional(&mut **tx)
        .await
        .map_err(storage)?;
        if let Some(other) = collision {
            authorize_existing(tx, actor, row.table, &other).await?;
            if rows::record(row.table, other).map_err(storage)? != *row {
                return Err(ContentError::IdentityConflict);
            }
        }
    }

    // Check the actual target object's scope BEFORE reporting any
    // identity collision; package-supplied scope cannot authorize it.
    if let Some(target) = &present {
        authorize_existing(tx, actor, row.table, target).await?;
    }
    let (space, mut owner) = package_scope(package, row.table, values)?;
    // Authorized reuse of another reviewer's already-existing space
    // stream does not mint a judgment on their behalf.
    if present.is_some()
        && matches!(
            row.table,
            SnapshotTable::EpistemicStream | SnapshotTable::EpistemicReview
        )
    {
        let stream = if row.table == SnapshotTable::EpistemicStream {
            values
        } else {
            package.one(
                SnapshotTable::EpistemicStream,
                &["id"],
                &[values["stream_id"].clone()],
            )?
        };
        if stream["overlay_id"].is_null() {
            owner = None;
        }
    }
    let require_write = present.is_none() || row.table == SnapshotTable::Overlay;
    grant(tx, actor, space, owner, require_write).await?;
    if let Some(target) = &present
        && rows::record(row.table, target.clone()).map_err(storage)? != *row
    {
        return Err(ContentError::IdentityConflict);
    }
    Ok(CheckedRow {
        present,
        space,
        require_write,
    })
}

#[derive(Default)]
struct OriginalBudget {
    bytes: u64,
    digests: BTreeSet<String>,
}
impl OriginalBudget {
    fn account(
        &mut self,
        destination_only: bool,
        digest: &str,
        size: u64,
    ) -> Result<(), ContentError> {
        // Only originals carried by this package consume the included-byte
        // budget. Destination-only assets are still individually verified.
        if destination_only {
            return Ok(());
        }
        if self.digests.insert(digest.to_owned()) {
            self.bytes = self.bytes.checked_add(size).ok_or_else(invalid)?;
        }
        if self.bytes > SNAPSHOT_MAX_ASSET_BYTES as u64 {
            return Err(invalid());
        }
        Ok(())
    }
}

async fn lookup(tx: &mut rows::Tx<'_>, row: &SnapshotRow) -> Result<Option<Value>, ContentError> {
    let keys = rows::primary_key(row.table);
    let condition = keys
        .iter()
        .map(|key| format!("to_jsonb(t.{key})=($1::jsonb->'{key}')"))
        .collect::<Vec<_>>()
        .join(" AND ");
    // Both table and column names come exclusively from the schema whitelist.
    sqlx::query_scalar(&format!(
        "SELECT to_jsonb(t) FROM public.{} t WHERE {condition}",
        rows::table_name(row.table)
    ))
    .bind(&row.immutable_values)
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage)
}
async fn grant(
    tx: &mut rows::Tx<'_>,
    actor: Principal,
    space: Uuid,
    owner: Option<Uuid>,
    write: bool,
) -> Result<(), ContentError> {
    if owner.is_some_and(|o| o != actor.actor_id) {
        return Err(ContentError::NotFound);
    }
    let allowed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.space_grant g JOIN public.space s ON s.id=g.space_id WHERE g.actor_id=$1 AND g.space_id=$2 AND (NOT $3 OR g.can_write))").bind(actor.actor_id).bind(space).bind(write).fetch_one(&mut **tx).await.map_err(storage)?;
    if allowed {
        Ok(())
    } else {
        Err(ContentError::NotFound)
    }
}
fn parent_scope(table: SnapshotTable) -> Option<(SnapshotTable, &'static str, &'static str)> {
    use SnapshotTable::*;
    match table {
        OverlayRevision
        | OverlayGroupIdentity
        | OverlayPlacementIdentity
        | OverlayGroup
        | OverlayPlacement
        | PlacementManualDecision
        | ReadingView
        | ReadingViewRevision => Some((Overlay, "overlay_id", "id")),
        RelationRevision | RelationReview | RelationReviewHead => {
            Some((Relation, "relation_id", "id"))
        }
        EpistemicReview => Some((EpistemicStream, "stream_id", "id")),
        ReadingRelationSelection | ReadingEpistemicSelection => {
            Some((ReadingViewRevision, "view_revision_id", "id"))
        }
        _ => None,
    }
}
fn package_scope(
    package: &Package<'_>,
    table: SnapshotTable,
    v: &Value,
) -> Result<(Uuid, Option<Uuid>), ContentError> {
    if let Some((parent, source, key)) = parent_scope(table) {
        return package_scope(
            package,
            parent,
            package.one(parent, &[key], &[v[source].clone()])?,
        );
    }
    if table == SnapshotTable::ReferenceDependency {
        let p = package.one(
            SnapshotTable::ReferenceObject,
            &["kind", "object_id", "revision_id"],
            &[
                v["source_kind"].clone(),
                v["source_object_id"].clone(),
                v["source_revision_id"].clone(),
            ],
        )?;
        return package_scope(package, SnapshotTable::ReferenceObject, p);
    }
    let owner = if table == SnapshotTable::Overlay {
        Some(id(v, "owner_id")?)
    } else if table == SnapshotTable::EpistemicStream {
        Some(id(v, "actor_id")?)
    } else {
        None
    };
    Ok((id(v, "space_id")?, owner))
}
async fn authorize_existing(
    tx: &mut rows::Tx<'_>,
    actor: Principal,
    mut table: SnapshotTable,
    target: &Value,
) -> Result<(), ContentError> {
    let mut v = target.clone();
    loop {
        if let Some((parent, source, key)) = parent_scope(table) {
            v = rows::one(tx, parent, &[key], &[id(&v, source)?]).await?;
            table = parent;
            continue;
        }
        if table == SnapshotTable::ReferenceDependency {
            v=sqlx::query_scalar("SELECT to_jsonb(r) FROM public.reference_object r WHERE kind=$1 AND object_id=$2 AND revision_id=$3").bind(v["source_kind"].as_str().ok_or_else(invalid)?).bind(id(&v,"source_object_id")?).bind(id(&v,"source_revision_id")?).fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(ContentError::NotFound)?;
            table = SnapshotTable::ReferenceObject;
            continue;
        }
        if table == SnapshotTable::ReferenceObject {
            let (next, key) = match v["kind"].as_str() {
                Some("block") => (SnapshotTable::BlockRevision, "block_id"),
                Some("relation") => (SnapshotTable::RelationRevision, "relation_id"),
                Some("relation_review") => (SnapshotTable::RelationReview, "relation_revision_id"),
                Some("epistemic_review") => (SnapshotTable::EpistemicReview, "stream_id"),
                _ => return Err(invalid()),
            };
            v = rows::one(
                tx,
                next,
                &[key, "id"],
                &[id(&v, "object_id")?, id(&v, "revision_id")?],
            )
            .await?;
            table = next;
            continue;
        }
        let space = id(&v, "space_id")?;
        let owner = if table == SnapshotTable::Overlay {
            Some(id(&v, "owner_id")?)
        } else if table == SnapshotTable::EpistemicStream && !v["overlay_id"].is_null() {
            Some(id(&v, "actor_id")?)
        } else {
            None
        };
        grant(tx, actor, space, owner, false).await?;
        if matches!(
            table,
            SnapshotTable::Relation | SnapshotTable::EpistemicStream
        ) && !v["overlay_id"].is_null()
        {
            let o = rows::one(
                tx,
                SnapshotTable::Overlay,
                &["id"],
                &[id(&v, "overlay_id")?],
            )
            .await?;
            grant(
                tx,
                actor,
                id(&o, "space_id")?,
                Some(id(&o, "owner_id")?),
                false,
            )
            .await?;
        }
        return Ok(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlay_revision_authorization_follows_actual_overlay_owner() {
        assert_eq!(
            parent_scope(SnapshotTable::OverlayRevision),
            Some((SnapshotTable::Overlay, "overlay_id", "id"))
        );
    }
    #[test]
    fn metadata_only_originals_do_not_consume_included_byte_budget() {
        let mut budget = OriginalBudget::default();
        for n in 0..5 {
            budget
                .account(
                    true,
                    &format!("{n:064x}"),
                    SNAPSHOT_MAX_ASSET_FILE_BYTES as u64,
                )
                .unwrap();
        }
        assert_eq!(budget.bytes, 0);
    }
    #[test]
    fn included_originals_enforce_unique_byte_budget() {
        let mut budget = OriginalBudget::default();
        for n in 0..4 {
            let digest = format!("{n:064x}");
            budget
                .account(false, &digest, SNAPSHOT_MAX_ASSET_FILE_BYTES as u64)
                .unwrap();
            budget
                .account(false, &digest, SNAPSHOT_MAX_ASSET_FILE_BYTES as u64)
                .unwrap();
        }
        assert_eq!(budget.bytes, SNAPSHOT_MAX_ASSET_BYTES as u64);
        assert!(
            budget
                .account(
                    false,
                    &format!("{:064x}", 5),
                    SNAPSHOT_MAX_ASSET_FILE_BYTES as u64
                )
                .is_err()
        );
    }
}
