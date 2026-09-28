//! Fixed immutable schema from migrations 0001–0013. No package identifiers become SQL.
use learning_core::SnapshotTable;
pub(super) struct Link {
    pub columns: &'static [&'static str],
    pub target: &'static str,
    pub keys: &'static [&'static str],
}
pub(super) fn columns(table: SnapshotTable) -> &'static [(&'static str, &'static str, bool)] {
    use SnapshotTable::*;
    match table {
        Asset => &[
            ("space_id", "uuid", false),
            ("id", "uuid", false),
            ("sha256", "text", false),
            ("byte_size", "bigint", false),
            ("storage_key", "text", false),
            ("media_type", "text", false),
            ("original_file_name", "text", false),
            ("status", "text", false),
            ("created_at", "timestamptz", false),
        ],
        Resource => &[
            ("space_id", "uuid", false),
            ("id", "uuid", false),
            ("display_name", "text", false),
            ("created_at", "timestamptz", false),
        ],
        ResourceVersion => &[
            ("space_id", "uuid", false),
            ("resource_id", "uuid", false),
            ("id", "uuid", false),
            ("asset_id", "uuid", false),
            ("version_no", "integer", false),
            ("created_at", "timestamptz", false),
        ],
        SourceSegment => &[
            ("space_id", "uuid", false),
            ("resource_id", "uuid", false),
            ("resource_version_id", "uuid", false),
            ("id", "uuid", false),
            ("selector", "jsonb", false),
            ("created_at", "timestamptz", false),
        ],
        Block => &[
            ("id", "uuid", false),
            ("space_id", "uuid", false),
            ("created_at", "timestamptz", false),
        ],
        BlockRevision => &[
            ("id", "uuid", false),
            ("space_id", "uuid", false),
            ("block_id", "uuid", false),
            ("parent_revision_id", "uuid", true),
            ("content", "jsonb", false),
            ("content_sha256", "text", false),
            ("author_id", "uuid", false),
            ("reason", "text", false),
            ("created_at", "timestamptz", false),
            ("contract_version", "integer", false),
        ],
        BlockAssetUse => &[
            ("space_id", "uuid", false),
            ("block_id", "uuid", false),
            ("revision_id", "uuid", false),
            ("asset_id", "uuid", false),
        ],
        Composition => &[
            ("id", "uuid", false),
            ("space_id", "uuid", false),
            ("kind", "text", false),
            ("created_at", "timestamptz", false),
        ],
        CompositionRevision => &[
            ("id", "uuid", false),
            ("space_id", "uuid", false),
            ("composition_id", "uuid", false),
            ("parent_revision_id", "uuid", true),
            ("kind", "text", false),
            ("title", "text", false),
            ("content_sha256", "text", false),
            ("author_id", "uuid", false),
            ("reason", "text", false),
            ("created_at", "timestamptz", false),
        ],
        CompositionOccurrence => &[
            ("composition_revision_id", "uuid", false),
            ("space_id", "uuid", false),
            ("composition_id", "uuid", false),
            ("occurrence_id", "uuid", false),
            ("position", "integer", false),
            ("block_space_id", "uuid", true),
            ("block_id", "uuid", true),
            ("block_revision_id", "uuid", true),
            ("child_space_id", "uuid", true),
            ("child_composition_id", "uuid", true),
            ("child_revision_id", "uuid", true),
        ],
        Overlay => &[
            ("id", "uuid", false),
            ("space_id", "uuid", false),
            ("owner_id", "uuid", false),
            ("root_composition_id", "uuid", false),
        ],
        OverlayRevision => &[
            ("id", "uuid", false),
            ("overlay_id", "uuid", false),
            ("space_id", "uuid", false),
            ("root_composition_id", "uuid", false),
            ("source_space_id", "uuid", false),
            ("base_revision_id", "uuid", false),
            ("parent_revision_id", "uuid", true),
            ("title", "text", false),
            ("content_sha256", "text", false),
            ("author_id", "uuid", false),
            ("reason", "text", false),
            ("created_at", "timestamptz", false),
        ],
        OverlayGroupIdentity => &[("overlay_id", "uuid", false), ("group_id", "uuid", false)],
        OverlayPlacementIdentity => &[
            ("overlay_id", "uuid", false),
            ("placement_id", "uuid", false),
            ("block_id", "uuid", false),
        ],
        OverlayGroup => &[
            ("overlay_id", "uuid", false),
            ("overlay_revision_id", "uuid", false),
            ("group_id", "uuid", false),
            ("placed", "boolean", false),
            ("source_space_id", "uuid", false),
            ("root_composition_id", "uuid", false),
            ("base_revision_id", "uuid", false),
            ("placed_base_revision_id", "uuid", true),
            ("parent_path", "uuid[]", false),
            ("left_id", "uuid", true),
            ("right_id", "uuid", true),
            ("affinity", "text", false),
        ],
        OverlayPlacement => &[
            ("overlay_id", "uuid", false),
            ("overlay_revision_id", "uuid", false),
            ("placement_id", "uuid", false),
            ("group_id", "uuid", false),
            ("position", "integer", false),
            ("block_space_id", "uuid", false),
            ("block_id", "uuid", false),
            ("block_revision_id", "uuid", false),
        ],
        PlacementManualDecision => &[
            ("overlay_id", "uuid", false),
            ("overlay_revision_id", "uuid", false),
            ("source_group_id", "uuid", false),
            ("result_group_id", "uuid", false),
        ],
        ReferenceObject => &[
            ("kind", "text", false),
            ("object_id", "uuid", false),
            ("revision_id", "uuid", false),
            ("space_id", "uuid", false),
        ],
        ReferenceDependency => &[
            ("source_kind", "text", false),
            ("source_object_id", "uuid", false),
            ("source_revision_id", "uuid", false),
            ("position", "integer", false),
            ("role", "text", false),
            ("target_kind", "text", false),
            ("target_object_id", "uuid", false),
            ("target_revision_id", "uuid", false),
        ],
        Relation => &[
            ("id", "uuid", false),
            ("space_id", "uuid", false),
            ("overlay_id", "uuid", true),
            ("type", "text", false),
            ("origin", "text", false),
            ("from_block_id", "uuid", false),
            ("to_block_id", "uuid", false),
            ("created_at", "timestamptz", false),
        ],
        RelationRevision => &[
            ("id", "uuid", false),
            ("space_id", "uuid", false),
            ("relation_id", "uuid", false),
            ("parent_revision_id", "uuid", true),
            ("from_space_id", "uuid", false),
            ("from_block_id", "uuid", false),
            ("from_revision_id", "uuid", false),
            ("to_space_id", "uuid", false),
            ("to_block_id", "uuid", false),
            ("to_revision_id", "uuid", false),
            ("rationale", "text", false),
            ("conditions", "text", false),
            ("content_sha256", "text", false),
            ("author_id", "uuid", false),
            ("created_at", "timestamptz", false),
        ],
        RelationReviewHead => &[
            ("relation_id", "uuid", false),
            ("relation_revision_id", "uuid", false),
        ],
        RelationReview => &[
            ("id", "uuid", false),
            ("space_id", "uuid", false),
            ("relation_id", "uuid", false),
            ("relation_revision_id", "uuid", false),
            ("previous_review_id", "uuid", true),
            ("state", "text", false),
            ("explanation", "text", false),
            ("reviewer_id", "uuid", false),
            ("created_at", "timestamptz", false),
        ],
        EpistemicStream => &[
            ("id", "uuid", false),
            ("space_id", "uuid", false),
            ("overlay_id", "uuid", true),
            ("target_space_id", "uuid", false),
            ("target_block_id", "uuid", false),
            ("target_revision_id", "uuid", false),
            ("actor_id", "uuid", false),
        ],
        EpistemicReview => &[
            ("id", "uuid", false),
            ("space_id", "uuid", false),
            ("stream_id", "uuid", false),
            ("previous_review_id", "uuid", true),
            ("state", "text", false),
            ("relations", "jsonb", false),
            ("evidence", "jsonb", false),
            ("conditions", "text", false),
            ("explanation", "text", false),
            ("reviewer_id", "uuid", false),
            ("created_at", "timestamptz", false),
        ],
        ReadingView => &[("id", "uuid", false), ("overlay_id", "uuid", false)],
        ReadingViewRevision => &[
            ("id", "uuid", false),
            ("view_id", "uuid", false),
            ("overlay_id", "uuid", false),
            ("overlay_revision_id", "uuid", false),
            ("parent_revision_id", "uuid", true),
            ("author_id", "uuid", false),
            ("created_at", "timestamptz", false),
            ("contract_version", "integer", false),
            ("evidence", "jsonb", true),
        ],
        ReadingRelationSelection => &[
            ("view_id", "uuid", false),
            ("view_revision_id", "uuid", false),
            ("position", "integer", false),
            ("relation_id", "uuid", false),
            ("relation_revision_id", "uuid", false),
            ("review_id", "uuid", true),
        ],
        ReadingEpistemicSelection => &[
            ("view_id", "uuid", false),
            ("view_revision_id", "uuid", false),
            ("position", "integer", false),
            ("stream_id", "uuid", false),
            ("review_id", "uuid", false),
        ],
    }
}
pub(super) fn links(table: SnapshotTable) -> &'static [Link] {
    use SnapshotTable::*;
    match table {
        Asset => &[Link {
            columns: &["space_id"],
            target: "space",
            keys: &["id"],
        }],
        Resource => &[Link {
            columns: &["space_id"],
            target: "space",
            keys: &["id"],
        }],
        ResourceVersion => &[
            Link {
                columns: &["space_id", "resource_id"],
                target: "resource",
                keys: &["space_id", "id"],
            },
            Link {
                columns: &["space_id", "asset_id"],
                target: "asset",
                keys: &["space_id", "id"],
            },
        ],
        SourceSegment => &[Link {
            columns: &["space_id", "resource_id", "resource_version_id"],
            target: "resource_version",
            keys: &["space_id", "resource_id", "id"],
        }],
        Block => &[Link {
            columns: &["space_id"],
            target: "space",
            keys: &["id"],
        }],
        BlockRevision => &[
            Link {
                columns: &["author_id"],
                target: "app_user",
                keys: &["id"],
            },
            Link {
                columns: &["space_id", "block_id"],
                target: "block",
                keys: &["space_id", "id"],
            },
            Link {
                columns: &["space_id", "block_id", "parent_revision_id"],
                target: "block_revision",
                keys: &["space_id", "block_id", "id"],
            },
        ],
        BlockAssetUse => &[
            Link {
                columns: &["space_id", "block_id", "revision_id"],
                target: "block_revision",
                keys: &["space_id", "block_id", "id"],
            },
            Link {
                columns: &["space_id", "asset_id"],
                target: "asset",
                keys: &["space_id", "id"],
            },
        ],
        Composition => &[Link {
            columns: &["space_id"],
            target: "space",
            keys: &["id"],
        }],
        CompositionRevision => &[
            Link {
                columns: &["author_id"],
                target: "app_user",
                keys: &["id"],
            },
            Link {
                columns: &["space_id", "composition_id"],
                target: "composition",
                keys: &["space_id", "id"],
            },
            Link {
                columns: &["space_id", "composition_id", "parent_revision_id"],
                target: "composition_revision",
                keys: &["space_id", "composition_id", "id"],
            },
        ],
        CompositionOccurrence => &[
            Link {
                columns: &["space_id", "composition_id", "composition_revision_id"],
                target: "composition_revision",
                keys: &["space_id", "composition_id", "id"],
            },
            Link {
                columns: &["block_space_id", "block_id", "block_revision_id"],
                target: "block_revision",
                keys: &["space_id", "block_id", "id"],
            },
            Link {
                columns: &[
                    "child_space_id",
                    "child_composition_id",
                    "child_revision_id",
                ],
                target: "composition_revision",
                keys: &["space_id", "composition_id", "id"],
            },
        ],
        Overlay => &[
            Link {
                columns: &["space_id"],
                target: "space",
                keys: &["id"],
            },
            Link {
                columns: &["owner_id"],
                target: "app_user",
                keys: &["id"],
            },
            Link {
                columns: &["root_composition_id"],
                target: "composition",
                keys: &["id"],
            },
        ],
        OverlayRevision => &[
            Link {
                columns: &["author_id"],
                target: "app_user",
                keys: &["id"],
            },
            Link {
                columns: &["space_id", "overlay_id"],
                target: "overlay",
                keys: &["space_id", "id"],
            },
            Link {
                columns: &["overlay_id", "root_composition_id"],
                target: "overlay",
                keys: &["id", "root_composition_id"],
            },
            Link {
                columns: &["source_space_id", "root_composition_id", "base_revision_id"],
                target: "composition_revision",
                keys: &["space_id", "composition_id", "id"],
            },
            Link {
                columns: &["overlay_id", "parent_revision_id"],
                target: "overlay_revision",
                keys: &["overlay_id", "id"],
            },
        ],
        OverlayGroupIdentity => &[Link {
            columns: &["overlay_id"],
            target: "overlay",
            keys: &["id"],
        }],
        OverlayPlacementIdentity => &[
            Link {
                columns: &["overlay_id"],
                target: "overlay",
                keys: &["id"],
            },
            Link {
                columns: &["block_id"],
                target: "block",
                keys: &["id"],
            },
        ],
        OverlayGroup => &[
            Link {
                columns: &["overlay_id", "overlay_revision_id"],
                target: "overlay_revision",
                keys: &["overlay_id", "id"],
            },
            Link {
                columns: &["overlay_id", "group_id"],
                target: "overlay_group_identity",
                keys: &["overlay_id", "group_id"],
            },
            Link {
                columns: &["overlay_id", "root_composition_id"],
                target: "overlay",
                keys: &["id", "root_composition_id"],
            },
            Link {
                columns: &["source_space_id", "root_composition_id", "base_revision_id"],
                target: "composition_revision",
                keys: &["space_id", "composition_id", "id"],
            },
            Link {
                columns: &[
                    "overlay_id",
                    "overlay_revision_id",
                    "root_composition_id",
                    "placed_base_revision_id",
                ],
                target: "overlay_revision",
                keys: &[
                    "overlay_id",
                    "id",
                    "root_composition_id",
                    "base_revision_id",
                ],
            },
        ],
        OverlayPlacement => &[
            Link {
                columns: &["overlay_id", "overlay_revision_id", "group_id"],
                target: "overlay_group",
                keys: &["overlay_id", "overlay_revision_id", "group_id"],
            },
            Link {
                columns: &["overlay_id", "placement_id", "block_id"],
                target: "overlay_placement_identity",
                keys: &["overlay_id", "placement_id", "block_id"],
            },
            Link {
                columns: &["block_space_id", "block_id", "block_revision_id"],
                target: "block_revision",
                keys: &["space_id", "block_id", "id"],
            },
        ],
        PlacementManualDecision => &[
            Link {
                columns: &["overlay_id", "overlay_revision_id"],
                target: "overlay_revision",
                keys: &["overlay_id", "id"],
            },
            Link {
                columns: &["overlay_id", "source_group_id"],
                target: "overlay_group_identity",
                keys: &["overlay_id", "group_id"],
            },
            Link {
                columns: &["overlay_id", "result_group_id"],
                target: "overlay_group_identity",
                keys: &["overlay_id", "group_id"],
            },
        ],
        ReferenceObject => &[Link {
            columns: &["space_id"],
            target: "space",
            keys: &["id"],
        }],
        ReferenceDependency => &[
            Link {
                columns: &["source_kind", "source_object_id", "source_revision_id"],
                target: "reference_object",
                keys: &["kind", "object_id", "revision_id"],
            },
            Link {
                columns: &["target_kind", "target_object_id", "target_revision_id"],
                target: "reference_object",
                keys: &["kind", "object_id", "revision_id"],
            },
        ],
        Relation => &[
            Link {
                columns: &["space_id"],
                target: "space",
                keys: &["id"],
            },
            Link {
                columns: &["from_block_id"],
                target: "block",
                keys: &["id"],
            },
            Link {
                columns: &["to_block_id"],
                target: "block",
                keys: &["id"],
            },
            Link {
                columns: &["space_id", "overlay_id"],
                target: "overlay",
                keys: &["space_id", "id"],
            },
        ],
        RelationRevision => &[
            Link {
                columns: &["author_id"],
                target: "app_user",
                keys: &["id"],
            },
            Link {
                columns: &["space_id", "relation_id", "from_block_id", "to_block_id"],
                target: "relation",
                keys: &["space_id", "id", "from_block_id", "to_block_id"],
            },
            Link {
                columns: &["space_id", "relation_id", "parent_revision_id"],
                target: "relation_revision",
                keys: &["space_id", "relation_id", "id"],
            },
            Link {
                columns: &["from_space_id", "from_block_id", "from_revision_id"],
                target: "block_revision",
                keys: &["space_id", "block_id", "id"],
            },
            Link {
                columns: &["to_space_id", "to_block_id", "to_revision_id"],
                target: "block_revision",
                keys: &["space_id", "block_id", "id"],
            },
        ],
        RelationReviewHead => &[Link {
            columns: &["relation_id", "relation_revision_id"],
            target: "relation_revision",
            keys: &["relation_id", "id"],
        }],
        RelationReview => &[
            Link {
                columns: &["reviewer_id"],
                target: "app_user",
                keys: &["id"],
            },
            Link {
                columns: &["space_id", "relation_id", "relation_revision_id"],
                target: "relation_revision",
                keys: &["space_id", "relation_id", "id"],
            },
            Link {
                columns: &["relation_id", "relation_revision_id"],
                target: "relation_review_head",
                keys: &["relation_id", "relation_revision_id"],
            },
            Link {
                columns: &["relation_id", "relation_revision_id", "previous_review_id"],
                target: "relation_review",
                keys: &["relation_id", "relation_revision_id", "id"],
            },
        ],
        EpistemicStream => &[
            Link {
                columns: &["space_id"],
                target: "space",
                keys: &["id"],
            },
            Link {
                columns: &["actor_id"],
                target: "app_user",
                keys: &["id"],
            },
            Link {
                columns: &["space_id", "overlay_id"],
                target: "overlay",
                keys: &["space_id", "id"],
            },
            Link {
                columns: &["target_space_id", "target_block_id", "target_revision_id"],
                target: "block_revision",
                keys: &["space_id", "block_id", "id"],
            },
        ],
        EpistemicReview => &[
            Link {
                columns: &["reviewer_id"],
                target: "app_user",
                keys: &["id"],
            },
            Link {
                columns: &["space_id", "stream_id", "reviewer_id"],
                target: "epistemic_stream",
                keys: &["space_id", "id", "actor_id"],
            },
            Link {
                columns: &["stream_id", "previous_review_id"],
                target: "epistemic_review",
                keys: &["stream_id", "id"],
            },
        ],
        ReadingView => &[Link {
            columns: &["overlay_id"],
            target: "overlay",
            keys: &["id"],
        }],
        ReadingViewRevision => &[
            Link {
                columns: &["author_id"],
                target: "app_user",
                keys: &["id"],
            },
            Link {
                columns: &["view_id", "overlay_id"],
                target: "reading_view",
                keys: &["id", "overlay_id"],
            },
            Link {
                columns: &["overlay_id", "overlay_revision_id"],
                target: "overlay_revision",
                keys: &["overlay_id", "id"],
            },
            Link {
                columns: &["view_id", "parent_revision_id"],
                target: "reading_view_revision",
                keys: &["view_id", "id"],
            },
        ],
        ReadingRelationSelection => &[
            Link {
                columns: &["view_id", "view_revision_id"],
                target: "reading_view_revision",
                keys: &["view_id", "id"],
            },
            Link {
                columns: &["relation_id", "relation_revision_id"],
                target: "relation_revision",
                keys: &["relation_id", "id"],
            },
            Link {
                columns: &["relation_id", "relation_revision_id", "review_id"],
                target: "relation_review",
                keys: &["relation_id", "relation_revision_id", "id"],
            },
        ],
        ReadingEpistemicSelection => &[
            Link {
                columns: &["view_id", "view_revision_id"],
                target: "reading_view_revision",
                keys: &["view_id", "id"],
            },
            Link {
                columns: &["stream_id", "review_id"],
                target: "epistemic_review",
                keys: &["stream_id", "id"],
            },
        ],
    }
}

/// Immutable SQL CHECK predicates from the frozen schema; evaluated by SELECT only.
pub(super) fn checks(table: SnapshotTable) -> &'static str {
    use SnapshotTable::*;
    match table {
        Asset => {
            r#"(sha256 ~ '^[0-9a-f]{64}$') AND (byte_size>=0) AND (char_length(media_type) BETWEEN 3 AND 200 AND media_type ~ '^[A-Za-z0-9.+_-]+/[A-Za-z0-9.+_-]+$') AND (char_length(original_file_name) BETWEEN 1 AND 255 AND btrim(original_file_name)<>'') AND (status='ready') AND (storage_key='sha256/'||substring(sha256 from 1 for 2)||'/'||sha256)"#
        }
        Resource => r#"(char_length(display_name) BETWEEN 1 AND 255 AND btrim(display_name)<>'')"#,
        ResourceVersion => r#"(version_no>0)"#,
        SourceSegment => {
            r#"(jsonb_typeof(selector)='object' AND octet_length(selector::text)<=16000)"#
        }
        Block => r#"true"#,
        BlockRevision => {
            r#"(content_sha256 ~ '^[0-9a-f]{64}$') AND (char_length(reason) BETWEEN 1 AND 1000) AND (parent_revision_id IS NULL OR parent_revision_id <> id)"#
        }
        BlockAssetUse => r#"true"#,
        Composition => r#"(kind IN ('document','section'))"#,
        CompositionRevision => {
            r#"(kind IN ('document','section')) AND (char_length(title)<=300) AND (content_sha256 ~ '^[0-9a-f]{64}$') AND (char_length(reason) BETWEEN 1 AND 1000) AND (parent_revision_id IS NULL OR parent_revision_id<>id)"#
        }
        CompositionOccurrence => {
            r#"(position BETWEEN 0 AND 511) AND ((block_space_id IS NOT NULL AND block_id IS NOT NULL AND block_revision_id IS NOT NULL AND child_space_id IS NULL AND child_composition_id IS NULL AND child_revision_id IS NULL) OR (block_space_id IS NULL AND block_id IS NULL AND block_revision_id IS NULL AND child_space_id IS NOT NULL AND child_composition_id IS NOT NULL AND child_revision_id IS NOT NULL))"#
        }
        Overlay => r#"true"#,
        OverlayRevision => {
            r#"(char_length(title)<=300) AND (content_sha256 ~ '^[0-9a-f]{64}$') AND (char_length(reason) BETWEEN 1 AND 1000) AND (parent_revision_id IS NULL OR parent_revision_id<>id)"#
        }
        OverlayGroupIdentity => r#"true"#,
        OverlayPlacementIdentity => r#"true"#,
        OverlayGroup => {
            r#"(cardinality(parent_path)<=15 AND array_position(parent_path,NULL) IS NULL) AND (affinity IN ('after_left','before_right')) AND ((placed AND placed_base_revision_id IS NOT NULL AND placed_base_revision_id=base_revision_id) OR (NOT placed AND placed_base_revision_id IS NULL)) AND (left_id IS NULL OR right_id IS NULL OR left_id<>right_id)"#
        }
        OverlayPlacement => r#"(position BETWEEN 0 AND 2047)"#,
        PlacementManualDecision => r#"true"#,
        ReferenceObject => r#"(kind IN ('block','relation','relation_review','epistemic_review'))"#,
        ReferenceDependency => {
            r#"(position>=0) AND (role IN ('basis','target','requires_context','source_run','selected_relation','selected_review'))"#
        }
        Relation => {
            r#"(type IN ('annotates','questions','answers','inspired_by','supports','opposes','tests','related_to')) AND (origin='user_asserted') AND (from_block_id<>to_block_id) AND (type<>'related_to' OR from_block_id<to_block_id)"#
        }
        RelationRevision => {
            r#"(octet_length(conditions)<=10000) AND (content_sha256 ~ '^[0-9a-f]{64}$') AND (parent_revision_id IS NULL OR parent_revision_id<>id)"#
        }
        RelationReviewHead => r#"true"#,
        RelationReview => {
            r#"(state IN ('unreviewed','reviewed','needs_recheck','withdrawn')) AND (octet_length(explanation)<=10000) AND (previous_review_id IS NULL OR previous_review_id<>id)"#
        }
        EpistemicStream => r#"true"#,
        EpistemicReview => {
            r#"(state IN ('untested','testing','inconclusive','supported_within_scope','refuted_within_scope','superseded')) AND (octet_length(conditions)<=10000) AND (octet_length(explanation)<=10000) AND (previous_review_id IS NULL OR previous_review_id<>id)"#
        }
        ReadingView => r#"true"#,
        ReadingViewRevision => r#"(parent_revision_id IS NULL OR parent_revision_id<>id)"#,
        ReadingRelationSelection => r#"(position BETWEEN 0 AND 255)"#,
        ReadingEpistemicSelection => r#"(position BETWEEN 0 AND 255)"#,
    }
}

/// Additional immutable uniqueness constraints, excluding supersets of the PK.
/// Nullable tuples here use NULLS NOT DISTINCT in the frozen schema.
pub(super) fn unique_key(table: SnapshotTable) -> &'static [&'static str] {
    use SnapshotTable::*;
    match table {
        ResourceVersion => &["space_id", "resource_id", "version_no"],
        CompositionOccurrence => &["composition_revision_id", "position"],
        OverlayPlacement => &["overlay_revision_id", "group_id", "position"],
        ReadingView => &["overlay_id"],
        Relation => &[
            "space_id",
            "overlay_id",
            "type",
            "from_block_id",
            "to_block_id",
        ],
        EpistemicStream => &[
            "space_id",
            "overlay_id",
            "target_block_id",
            "target_revision_id",
            "actor_id",
        ],
        ReadingRelationSelection => &["view_revision_id", "relation_id", "relation_revision_id"],
        ReadingEpistemicSelection => &["view_revision_id", "stream_id", "review_id"],
        OverlayGroup => &[
            "overlay_revision_id",
            "root_composition_id",
            "base_revision_id",
            "parent_path",
            "left_id",
            "right_id",
            "affinity",
        ],
        _ => &[],
    }
}
