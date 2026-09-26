//! Fixed SQL table/column vocabulary for exact immutable records.
use crate::storage;
use chrono::{DateTime, Utc};
use learning_core::*;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub(super) type Tx<'a> = Transaction<'a, Postgres>;

pub(super) fn table_name(table: SnapshotTable) -> &'static str {
    use SnapshotTable::*;
    match table {
        Asset => "asset",
        Resource => "resource",
        ResourceVersion => "resource_version",
        SourceSegment => "source_segment",
        Block => "block",
        BlockRevision => "block_revision",
        BlockAssetUse => "block_asset_use",
        Composition => "composition",
        CompositionRevision => "composition_revision",
        CompositionOccurrence => "composition_occurrence",
        Overlay => "overlay",
        OverlayRevision => "overlay_revision",
        OverlayGroupIdentity => "overlay_group_identity",
        OverlayPlacementIdentity => "overlay_placement_identity",
        OverlayGroup => "overlay_group",
        OverlayPlacement => "overlay_placement",
        PlacementManualDecision => "placement_manual_decision",
        ReferenceObject => "reference_object",
        ReferenceDependency => "reference_dependency",
        Relation => "relation",
        RelationRevision => "relation_revision",
        RelationReviewHead => "relation_review_head",
        RelationReview => "relation_review",
        EpistemicStream => "epistemic_stream",
        EpistemicReview => "epistemic_review",
        ReadingView => "reading_view",
        ReadingViewRevision => "reading_view_revision",
        ReadingRelationSelection => "reading_relation_selection",
        ReadingEpistemicSelection => "reading_epistemic_selection",
    }
}

pub(super) fn primary_key(table: SnapshotTable) -> &'static [&'static str] {
    use SnapshotTable::*;
    match table {
        Asset | Resource | ResourceVersion | SourceSegment => &["space_id", "id"],
        Block | BlockRevision | Composition | CompositionRevision | Overlay | OverlayRevision
        | Relation | RelationRevision | RelationReview | EpistemicStream | EpistemicReview
        | ReadingView | ReadingViewRevision => &["id"],
        BlockAssetUse => &["space_id", "block_id", "revision_id"],
        CompositionOccurrence => &["composition_revision_id", "occurrence_id"],
        OverlayGroupIdentity => &["overlay_id", "group_id"],
        OverlayPlacementIdentity => &["overlay_id", "placement_id"],
        OverlayGroup => &["overlay_revision_id", "group_id"],
        OverlayPlacement => &["overlay_revision_id", "placement_id"],
        PlacementManualDecision => &["overlay_revision_id", "source_group_id"],
        ReferenceObject => &["kind", "object_id", "revision_id"],
        ReferenceDependency => &[
            "source_kind",
            "source_object_id",
            "source_revision_id",
            "position",
        ],
        RelationReviewHead => &["relation_id", "relation_revision_id"],
        ReadingRelationSelection | ReadingEpistemicSelection => &["view_revision_id", "position"],
    }
}

pub(super) fn uuid(value: &Value, key: &str) -> Result<Uuid, ContentError> {
    optional_uuid(value, key)?.ok_or(ContentError::Storage)
}
pub(super) fn optional_uuid(value: &Value, key: &str) -> Result<Option<Uuid>, ContentError> {
    match value.get(key) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => Uuid::parse_str(raw).map(Some).map_err(storage),
        _ => Err(ContentError::Storage),
    }
}
pub(super) fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, ContentError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or(ContentError::Storage)
}

/// Callers pass only compile-time column names; no package input becomes SQL.
pub(super) async fn many(
    tx: &mut Tx<'_>,
    table: SnapshotTable,
    columns: &[&str],
    ids: &[Uuid],
) -> Result<Vec<Value>, ContentError> {
    if columns.is_empty() || columns.len() != ids.len() {
        return Err(ContentError::Storage);
    }
    let filter = columns
        .iter()
        .enumerate()
        .map(|(i, c)| format!("t.{c}=${}", i + 1))
        .collect::<Vec<_>>()
        .join(" AND ");
    let sql = format!(
        "SELECT to_jsonb(t) FROM public.{} t WHERE {filter} ORDER BY {} LIMIT {}",
        table_name(table),
        primary_key(table)
            .iter()
            .map(|c| format!("t.{c}"))
            .collect::<Vec<_>>()
            .join(","),
        SNAPSHOT_MAX_FILES + 1
    );
    let mut query = sqlx::query_scalar::<_, Value>(&sql);
    for id in ids {
        query = query.bind(id);
    }
    query.fetch_all(&mut **tx).await.map_err(storage)
}
pub(super) async fn one(
    tx: &mut Tx<'_>,
    table: SnapshotTable,
    columns: &[&str],
    ids: &[Uuid],
) -> Result<Value, ContentError> {
    let mut rows = many(tx, table, columns, ids).await?;
    if rows.len() != 1 {
        return Err(ContentError::NotFound);
    }
    Ok(rows.remove(0))
}

pub(super) fn record(table: SnapshotTable, mut values: Value) -> Result<SnapshotRow, ContentError> {
    let object = values.as_object_mut().ok_or(ContentError::Storage)?;
    // Mutable publication/head state is never read as a dependency, nor exported.
    for key in [
        "head_revision_id",
        "head_review_id",
        "published_revision_id",
        "last_release_id",
    ] {
        object.remove(key);
    }
    if let Some(time) = object.get_mut("created_at") {
        let parsed = DateTime::parse_from_rfc3339(time.as_str().ok_or(ContentError::Storage)?)
            .map_err(storage)?;
        *time = Value::String(canonical_snapshot_timestamp(parsed.with_timezone(&Utc)));
    }
    if table == SnapshotTable::BlockRevision {
        let version = values["contract_version"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or(ContentError::Storage)?;
        let draft = ContentDraft::decode(version, values["content"].clone())?;
        if values["content_sha256"].as_str() != Some(draft.digest().as_str()) {
            return Err(ContentError::Invalid(
                "snapshot_business_digest_mismatch".into(),
            ));
        }
    }
    let identity = primary_key(table)
        .iter()
        .map(|key| {
            if *key == "kind" || *key == "source_kind" {
                serde_json::from_value(values[*key].clone())
                    .map(SnapshotIdentityPart::Kind)
                    .map_err(storage)
            } else if *key == "position" {
                let position = values[*key]
                    .as_u64()
                    .and_then(|p| u32::try_from(p).ok())
                    .ok_or(ContentError::Storage)?;
                Ok(SnapshotIdentityPart::Position(position))
            } else {
                uuid(&values, key).map(SnapshotIdentityPart::Uuid)
            }
        })
        .collect::<Result<Vec<_>, ContentError>>()?;
    snapshot_object_path(table, &identity)?;
    Ok(SnapshotRow {
        table,
        identity,
        sha256: canonical_record_hash(&values),
        immutable_values: values,
    })
}

pub(super) async fn references(
    tx: &mut Tx<'_>,
    table: SnapshotTable,
    exact: &ExactRef,
) -> Result<Vec<Value>, ContentError> {
    let (kind, object, revision) = crate::references::key(exact);
    let (kind_col, object_col, revision_col) = if table == SnapshotTable::ReferenceObject {
        ("kind", "object_id", "revision_id")
    } else if table == SnapshotTable::ReferenceDependency {
        ("source_kind", "source_object_id", "source_revision_id")
    } else {
        return Err(ContentError::Storage);
    };
    sqlx::query_scalar(&format!("SELECT to_jsonb(t) FROM public.{} t WHERE {kind_col}=$1 AND {object_col}=$2 AND {revision_col}=$3 ORDER BY {} LIMIT {}",
        table_name(table), primary_key(table).join(","), SNAPSHOT_MAX_EDGES+1))
        .bind(kind).bind(object).bind(revision).fetch_all(&mut **tx).await.map_err(storage)
}

pub(super) async fn dependency_target(
    tx: &mut Tx<'_>,
    value: &Value,
) -> Result<ExactRef, ContentError> {
    let object = uuid(value, "target_object_id")?;
    let revision = uuid(value, "target_revision_id")?;
    Ok(match string(value, "target_kind")? {
        "block" => ExactRef::Block(BlockRef {
            block_id: object,
            revision_id: revision,
        }),
        "relation" => ExactRef::Relation(RelationRef {
            relation_id: object,
            revision_id: revision,
        }),
        "epistemic_review" => ExactRef::EpistemicReview(EpistemicReviewRef {
            stream_id: object,
            review_id: revision,
        }),
        "relation_review" => {
            let row = one(
                tx,
                SnapshotTable::RelationReview,
                &["relation_revision_id", "id"],
                &[object, revision],
            )
            .await?;
            ExactRef::RelationReview(RelationReviewRef {
                relation: RelationRef {
                    relation_id: uuid(&row, "relation_id")?,
                    revision_id: object,
                },
                review_id: revision,
            })
        }
        _ => return Err(ContentError::Storage),
    })
}

#[cfg(test)]
mod import_record_tests {
    use super::*;
    #[test]
    fn import_record_rejects_body_tampering_even_with_recomputed_full_row_hash() {
        let content = serde_json::json!({"kind":"text","intent":"note","language":"en","title":"Title","payload":{"format":"markdown","text":"original"}});
        let draft = ContentDraft::decode(1, content.clone()).unwrap();
        let mut value = serde_json::json!({"id":Uuid::new_v4(),"contract_version":1,"content":content,"content_sha256":draft.digest()});
        assert!(record(SnapshotTable::BlockRevision, value.clone()).is_ok());
        value["content"]["payload"]["text"] = serde_json::json!("tampered");
        assert!(
            record(SnapshotTable::BlockRevision, value).is_err(),
            "full-record hashing cannot replace business digest validation"
        );
    }
}
