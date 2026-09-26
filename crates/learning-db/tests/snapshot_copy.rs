use learning_assets::sanitize_reading_copy;
use learning_core::{
    BlockRef, ContentDraft, ContentRevision, CopyItem, EpistemicReview, EpistemicReviewRef,
    EpistemicState, ReadingEvidence, ReadingItemData, ReadingMode, ReadingRef, RelationScope,
    ReviewProjection, SourceProjection, TextDraft, VersionedReadingProjection,
};
use uuid::Uuid;

fn revision(id: Uuid, text: &str) -> ContentRevision {
    let draft: TextDraft = serde_json::from_value(serde_json::json!({"kind":"text","intent":"note","language":"en","title":"Title","payload":{"format":"markdown","text":text}})).unwrap();
    ContentRevision {
        block_id: id,
        revision_id: id,
        parent_revision_id: None,
        content_sha256: draft.digest(),
        draft: ContentDraft::V1(draft),
        author_id: id,
        reason: "private reason".into(),
        created_at: chrono::Utc::now(),
    }
}
fn projection() -> VersionedReadingProjection {
    let id = Uuid::from_u128(0x123456);
    VersionedReadingProjection {
        contract_version: 1,
        overlay: learning_core::OverlayRef {
            overlay_id: id,
            revision_id: id,
        },
        view: ReadingRef {
            view_id: id,
            revision_id: id,
        },
        source: SourceProjection::Available {
            base: learning_core::CompositionRef {
                composition_id: id,
                revision_id: id,
            },
        },
        items: vec![
            ReadingItemData::SectionStart {
                path: vec![id],
                title: "Chapter".into(),
            },
            ReadingItemData::Original {
                path: vec![id],
                revision: revision(id, "Original"),
            },
            ReadingItemData::Personal {
                item: learning_core::PersonalItemData {
                    placement_id: id,
                    revision: revision(id, "Personal"),
                    location: None,
                },
            },
            ReadingItemData::SectionEnd { path: vec![id] },
        ],
        unplaced: vec![],
        evidence: ReadingEvidence::default(),
    }
}

#[test]
fn copy_modes_only_show_requested_visible_text() {
    let projection = projection();
    let original = sanitize_reading_copy(&projection, ReadingMode::Original, false).unwrap();
    let fused = sanitize_reading_copy(&projection, ReadingMode::Fused, true).unwrap();
    let personal = sanitize_reading_copy(&projection, ReadingMode::Personal, true).unwrap();
    let text = |items: &[CopyItem]| serde_json::to_string(items).unwrap();
    assert!(text(&original.items).contains("Original"));
    assert!(!text(&original.items).contains("Personal"));
    assert!(text(&fused.items).contains("Original"));
    assert!(text(&fused.items).contains("Personal"));
    assert!(!text(&personal.items).contains("Original"));
    assert!(text(&personal.items).contains("Personal"));
    assert!(
        !text(
            &sanitize_reading_copy(&projection, ReadingMode::Fused, false)
                .unwrap()
                .items
        )
        .contains("Personal")
    );
}

#[test]
fn copy_omits_private_identity_and_uses_one_generic_marker() {
    let projection = projection();
    let copy = sanitize_reading_copy(&projection, ReadingMode::Fused, false).unwrap();
    let json = serde_json::to_string(&copy).unwrap();
    assert_eq!(
        copy.items
            .iter()
            .filter(|item| matches!(item, CopyItem::Omitted))
            .count(),
        1
    );
    for forbidden in [
        "00000000-0000-0000-0000-000000123456",
        "overlay_id",
        "placement_id",
        "revision_id",
        "private reason",
        "Personal",
    ] {
        assert!(!json.contains(forbidden), "leaked {forbidden}: {json}");
    }
}

#[test]
fn original_and_no_personal_modes_never_emit_personal_evidence() {
    let mut projection = projection();
    let id = Uuid::from_u128(0x123456);
    projection
        .evidence
        .epistemic_reviews
        .push(ReviewProjection::Available(EpistemicReview {
            reference: EpistemicReviewRef {
                stream_id: id,
                review_id: id,
            },
            scope: RelationScope::Space { space_id: id },
            target: BlockRef {
                block_id: id,
                revision_id: id,
            },
            previous: None,
            state: EpistemicState::Inconclusive,
            relations: vec![],
            evidence: vec![],
            conditions: "private conditions".into(),
            explanation: "Personal evidence".into(),
            reviewer_id: id,
            created_at: chrono::Utc::now(),
        }));
    let original = sanitize_reading_copy(&projection, ReadingMode::Original, true).unwrap();
    let no_personal = sanitize_reading_copy(&projection, ReadingMode::Fused, false).unwrap();
    let included = sanitize_reading_copy(&projection, ReadingMode::Fused, true).unwrap();
    assert!(original.evidence.is_empty());
    assert!(no_personal.evidence.is_empty());
    assert_eq!(included.evidence, ["Personal evidence"]);
    let json = serde_json::to_string(&included).unwrap();
    for forbidden in [
        "stream_id",
        "reviewer_id",
        "private conditions",
        "00000000-0000-0000-0000-000000123456",
    ] {
        assert!(!json.contains(forbidden));
    }
}
