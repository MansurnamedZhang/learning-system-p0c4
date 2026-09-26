//! Atomic exact publication. No ordinary writer, request receipt, or job path.
use super::{
    import::{PreparedSnapshotImport, SnapshotImportStore, check_identities, check_row},
    import_plan::{HeadPlan, insertion_order, receipt_policy},
    import_rows::{Package, id, invalid},
    import_schema, rows,
};
use crate::{authorization, storage};
use learning_assets::UploadDeclaration;
use learning_core::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotImportReceipt {
    pub manifest_sha256: String,
    pub reused: bool,
}

// Reconciliation can inspect the existing CAS; failures never unlink a digest
// potentially shared with committed assets. Keep the log generic and internal.
struct RetainedCas {
    digests: usize,
    committed: bool,
}
impl Drop for RetainedCas {
    fn drop(&mut self) {
        if !self.committed && self.digests > 0 {
            eprintln!(
                "snapshot import failed; {} CAS digest(s) retained for conservative reconciliation",
                self.digests
            );
        }
    }
}
impl SnapshotImportStore {
    pub async fn import_exact(
        &self,
        actor: Principal,
        request_id: Uuid,
        mut prepared: PreparedSnapshotImport,
    ) -> Result<SnapshotImportReceipt, ContentError> {
        let package = Package {
            rows: &prepared.rows,
        };
        package.validate(&prepared.manifest.root, actor)?;
        let heads = HeadPlan::build(&package, &prepared.manifest.root)?;
        let ordered = insertion_order(&package)?;
        let mut retained = RetainedCas {
            digests: 0,
            committed: false,
        };
        if !prepared.manifest.requires_destination_assets {
            let mut copied = BTreeSet::new();
            for asset in &mut prepared.assets {
                if !copied.insert(asset.sha256.clone()) {
                    continue;
                }
                // Count attempts too: an interrupted copy may leave an orphan.
                retained.digests += 1;
                let blob = self
                    .files
                    .put_from_open_file(
                        Uuid::new_v4(),
                        &mut asset.source,
                        UploadDeclaration {
                            expected_size_bytes: asset.byte_size,
                            max_size_bytes: SNAPSHOT_MAX_ASSET_FILE_BYTES as u64,
                        },
                    )
                    .map_err(storage)?;
                if blob.sha256() != asset.sha256 || blob.size_bytes() != asset.byte_size {
                    return Err(invalid());
                }
            }
        }
        let mut tx = self.pool.begin().await.map_err(storage)?;
        for statement in [
            "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
            "SET LOCAL lock_timeout='10s'",
            "SET LOCAL statement_timeout='15s'",
            "SET CONSTRAINTS ALL DEFERRED",
        ] {
            sqlx::query(statement)
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
        }
        for key in [
            format!(
                "learning/snapshot/import/request/{}:{}",
                actor.actor_id, request_id
            ),
            format!(
                "learning/snapshot/import/manifest/{}:{}",
                actor.actor_id, prepared.manifest_sha256
            ),
        ] {
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
                .bind(key)
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
        }
        check_identities(&mut tx, actor, &package).await?;
        let mut required = vec![];
        for row in package.rows {
            let checked = check_row(&mut tx, actor, &package, row).await?;
            if prepared.manifest.requires_destination_assets
                && row.table == SnapshotTable::Asset
                && checked.present.is_none()
            {
                return Err(ContentError::NotFound);
            }
            required.push((checked.space, checked.require_write));
        }
        authorization::lock_grants(&mut tx, actor, &required).await?;
        for row in package.rows {
            check_row(&mut tx, actor, &package, row).await?;
        }
        let prior:Option<String>=sqlx::query_scalar("SELECT manifest_sha256 FROM public.snapshot_import_batch WHERE actor_id=$1 AND request_id=$2")
            .bind(actor.actor_id).bind(request_id).fetch_optional(&mut *tx).await.map_err(storage)?;
        let seen:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.snapshot_import_batch WHERE actor_id=$1 AND manifest_sha256=$2)")
            .bind(actor.actor_id).bind(&prepared.manifest_sha256).fetch_one(&mut *tx).await.map_err(storage)?;
        let reused = receipt_policy(prior.as_deref(), &prepared.manifest_sha256, seen)?;
        for row in ordered {
            if prepared.manifest.requires_destination_assets && row.table == SnapshotTable::Asset {
                if check_row(&mut tx, actor, &package, row)
                    .await?
                    .present
                    .is_none()
                {
                    return Err(ContentError::NotFound);
                }
                continue;
            }
            let value = heads.value(row)?;
            let mut columns = import_schema::columns(row.table)
                .iter()
                .map(|(column, _, _)| *column)
                .collect::<Vec<_>>();
            for head in ["head_revision_id", "head_review_id"] {
                if value.get(head).is_some() {
                    columns.push(head);
                }
            }
            let table = rows::table_name(row.table);
            // Only compiled table/column vocabulary enters SQL identifiers.
            let sql = format!(
                "INSERT INTO public.{table} ({}) SELECT {} FROM jsonb_populate_record(NULL::public.{table},$1) AS p WHERE true ON CONFLICT DO NOTHING",
                columns.join(","),
                columns
                    .iter()
                    .map(|c| format!("p.{c}"))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            sqlx::query(&sql)
                .bind(value)
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
            // A fresh READ COMMITTED statement sees a concurrent unique-key
            // winner after INSERT waited. Authorize actual scope before compare.
            if check_row(&mut tx, actor, &package, row)
                .await?
                .present
                .is_none()
            {
                return Err(ContentError::IdentityConflict);
            }
        }
        for row in package.rows {
            if check_row(&mut tx, actor, &package, row)
                .await?
                .present
                .is_none()
            {
                return Err(invalid());
            }
        }
        verify_invariants(&mut tx, &package).await?;
        sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        // Also on replay: no old retained inode can repair a missing target
        // path for metadata-only imports. Always reopen the current CAS entry.
        for row in package
            .rows
            .iter()
            .filter(|r| r.table == SnapshotTable::Asset)
        {
            let v = &row.immutable_values;
            self.files
                .open_record(
                    v["storage_key"].as_str().ok_or_else(invalid)?,
                    v["sha256"].as_str().ok_or_else(invalid)?,
                    v["byte_size"].as_i64().ok_or_else(invalid)?,
                )
                .map_err(|_| ContentError::NotFound)?;
        }
        sqlx::query("INSERT INTO public.snapshot_import_batch(actor_id,request_id,manifest_sha256,status) VALUES($1,$2,$3,'succeeded') ON CONFLICT DO NOTHING")
            .bind(actor.actor_id).bind(request_id).bind(&prepared.manifest_sha256).execute(&mut *tx).await.map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        retained.committed = true;
        Ok(SnapshotImportReceipt {
            manifest_sha256: prepared.manifest_sha256,
            reused,
        })
    }
}

async fn exact_children(
    tx: &mut rows::Tx<'_>,
    package: &Package<'_>,
    table: SnapshotTable,
    columns: &[&str],
    values: &[Value],
) -> Result<(), ContentError> {
    let predicate = columns
        .iter()
        .enumerate()
        .map(|(n, c)| format!("to_jsonb(t.{c})=($1::jsonb->{n})"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let actual: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT to_jsonb(t) FROM public.{} t WHERE {predicate}",
        rows::table_name(table)
    ))
    .bind(json!(values))
    .fetch_all(&mut **tx)
    .await
    .map_err(storage)?;
    compare_children(package, table, columns, values, actual)
}

fn registry_scope() -> &'static [&'static str] {
    // A typed revision must have exactly its package registry set, including
    // rejection of a second registration under a different object identity.
    &["kind", "revision_id"]
}

fn compare_children(
    package: &Package<'_>,
    table: SnapshotTable,
    columns: &[&str],
    values: &[Value],
    actual: Vec<Value>,
) -> Result<(), ContentError> {
    let expected = package.matching(table, columns, values);
    if actual.len() != expected.len() {
        return Err(invalid());
    }
    for value in actual {
        let row = rows::record(table, value).map_err(storage)?;
        if !package.rows.contains(&row) {
            return Err(invalid());
        }
    }
    Ok(())
}
async fn verify_invariants(
    tx: &mut rows::Tx<'_>,
    package: &Package<'_>,
) -> Result<(), ContentError> {
    use SnapshotTable::*;
    for row in package.rows {
        let v = &row.immutable_values;
        match row.table {
            ReferenceObject => {
                let scope = registry_scope();
                let values = scope
                    .iter()
                    .map(|column| v[*column].clone())
                    .collect::<Vec<_>>();
                exact_children(tx, package, ReferenceObject, scope, &values).await?;
                sqlx::query("SELECT public.b3_check_object($1,$2,$3)")
                    .bind(v["kind"].as_str().ok_or_else(invalid)?)
                    .bind(id(v, "object_id")?)
                    .bind(id(v, "revision_id")?)
                    .execute(&mut **tx)
                    .await
                    .map_err(|_| invalid())?;
                exact_children(
                    tx,
                    package,
                    ReferenceDependency,
                    &["source_kind", "source_object_id", "source_revision_id"],
                    &[
                        v["kind"].clone(),
                        v["object_id"].clone(),
                        v["revision_id"].clone(),
                    ],
                )
                .await?;
            }
            BlockRevision => {
                sqlx::query("SELECT public.p0c_check_block_asset_use($1,$2,$3)")
                    .bind(id(v, "space_id")?)
                    .bind(id(v, "block_id")?)
                    .bind(id(v, "id")?)
                    .execute(&mut **tx)
                    .await
                    .map_err(|_| invalid())?;
                exact_children(
                    tx,
                    package,
                    BlockAssetUse,
                    &["space_id", "block_id", "revision_id"],
                    &[
                        v["space_id"].clone(),
                        v["block_id"].clone(),
                        v["id"].clone(),
                    ],
                )
                .await?;
            }
            CompositionRevision => {
                exact_children(
                    tx,
                    package,
                    CompositionOccurrence,
                    &["composition_revision_id"],
                    &[v["id"].clone()],
                )
                .await?
            }
            OverlayRevision => {
                for table in [OverlayGroup, OverlayPlacement, PlacementManualDecision] {
                    exact_children(
                        tx,
                        package,
                        table,
                        &["overlay_revision_id"],
                        &[v["id"].clone()],
                    )
                    .await?;
                }
            }
            ReadingViewRevision => {
                sqlx::query("SELECT public.b3_check_reading_evidence($1)")
                    .bind(id(v, "id")?)
                    .execute(&mut **tx)
                    .await
                    .map_err(|_| invalid())?;
                for table in [ReadingRelationSelection, ReadingEpistemicSelection] {
                    exact_children(
                        tx,
                        package,
                        table,
                        &["view_revision_id"],
                        &[v["id"].clone()],
                    )
                    .await?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_comparison_rejects_extra_object_for_included_typed_revision() {
        let (rows, _, _) = super::super::import_tests::fixture();
        let package = Package { rows: &rows };
        let expected = rows
            .iter()
            .find(|r| r.table == SnapshotTable::ReferenceObject)
            .unwrap();
        let mut extra = expected.immutable_values.clone();
        extra["object_id"] = json!(Uuid::new_v4());
        let target = [expected.immutable_values.clone(), extra];
        let columns = registry_scope();
        let values = columns
            .iter()
            .map(|c| expected.immutable_values[*c].clone())
            .collect::<Vec<_>>();
        // Use the production query's exact predicate to select target rows.
        let actual = target
            .into_iter()
            .filter(|row| columns.iter().zip(&values).all(|(c, v)| row[*c] == *v))
            .collect();
        assert!(matches!(
            compare_children(
                &package,
                SnapshotTable::ReferenceObject,
                columns,
                &values,
                actual
            ),
            Err(ContentError::Invalid(_))
        ));
    }

    #[test]
    fn registry_comparison_preserves_unrelated_revisions_and_types() {
        let (rows, _, _) = super::super::import_tests::fixture();
        let package = Package { rows: &rows };
        let expected = rows
            .iter()
            .find(|r| r.table == SnapshotTable::ReferenceObject)
            .unwrap();
        let mut newer = expected.immutable_values.clone();
        newer["revision_id"] = json!(Uuid::new_v4());
        let mut other_type = expected.immutable_values.clone();
        other_type["kind"] = json!("relation");
        let target = [expected.immutable_values.clone(), newer, other_type];
        let columns = registry_scope();
        let values = columns
            .iter()
            .map(|c| expected.immutable_values[*c].clone())
            .collect::<Vec<_>>();
        let actual = target
            .into_iter()
            .filter(|row| columns.iter().zip(&values).all(|(c, v)| row[*c] == *v))
            .collect();
        compare_children(
            &package,
            SnapshotTable::ReferenceObject,
            columns,
            &values,
            actual,
        )
        .unwrap();
        assert!(
            compare_children(
                &package,
                SnapshotTable::ReferenceObject,
                columns,
                &values,
                vec![]
            )
            .is_err()
        );
    }
}
