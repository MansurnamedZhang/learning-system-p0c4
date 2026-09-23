mod support;

use learning_assets::{FsAssetStore, UploadDeclaration, VerifiedBlob};
use learning_core::ContentError;
use learning_db::{AssetMedia, AssetStore, ResourceInput};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
use support::TestRig;
use uuid::Uuid;

const PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n%%EOF\n";
const PNG: &[u8] = &[137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 73, 69, 78, 68];
const NOTEBOOK: &[u8] = br#"{"cells":[{"cell_type":"code","source":["print(42)"]}],"nbformat":4}"#;

struct Files {
    root: PathBuf,
    store: FsAssetStore,
}

impl Files {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("learning-db-assets-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let store = FsAssetStore::new(root.join("assets"), root.join("staging")).unwrap();
        Self { root, store }
    }

    fn verified(&self, name: &str, bytes: &[u8]) -> VerifiedBlob {
        let source = self.root.join(name);
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
        self.store.verify(&blob).unwrap();
        blob
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn media(media_type: &str, name: &str) -> AssetMedia {
    AssetMedia {
        media_type: media_type.into(),
        original_file_name: name.into(),
    }
}

#[tokio::test]
async fn verified_registration_replays_one_receipt_and_conflicts_on_different_bytes() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let actor = rig.seed_actor_space(true).await;
    let store = AssetStore::new(rig.runtime_pool.clone(), files.store.clone());
    let blob = files.verified("first.pdf", PDF);
    let request = Uuid::new_v4();
    let first = store
        .register_verified(
            actor.0,
            actor.1,
            request,
            blob.clone(),
            media("application/pdf", "handout.pdf"),
        )
        .await
        .unwrap();
    assert_eq!(first.reference.space_id, actor.1);
    assert_eq!(
        first.sha256,
        "904636248025ad20fb9c6bd8b700179a2a42edb5df3636e926c7e09055ee3f75"
    );
    assert_eq!(first.byte_size, PDF.len() as i64);
    assert_eq!(first.storage_key, blob.storage_key());
    let replay = store
        .register_verified(
            actor.0,
            actor.1,
            request,
            blob,
            media("application/pdf", "handout.pdf"),
        )
        .await
        .unwrap();
    assert_eq!(replay, first);
    let changed = files.verified("different.pdf", b"different original bytes");
    assert!(matches!(
        store
            .register_verified(
                actor.0,
                actor.1,
                request,
                changed,
                media("application/pdf", "handout.pdf")
            )
            .await,
        Err(ContentError::IdempotencyConflict)
    ));
    let counts: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM asset WHERE space_id=$1),(SELECT count(*) FROM upload_receipt WHERE actor_id=$2 AND request_id=$3)")
        .bind(actor.1)
        .bind(actor.0.actor_id)
        .bind(request)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    assert_eq!(counts, (1, 1));
}

#[tokio::test]
async fn media_claim_must_match_original_bytes_without_weakening_request_conflicts() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let store = AssetStore::new(rig.runtime_pool.clone(), files.store.clone());

    for (name, bytes, claimed_type) in [
        ("png-as-pdf", PNG, "application/pdf"),
        ("bad-pdf", b"plain text" as &[u8], "application/pdf"),
        ("bad-png", b"not a PNG" as &[u8], "image/png"),
        (
            "bad-notebook",
            br#"{"nbformat":4,"cells":{}}"# as &[u8],
            "application/x-ipynb+json",
        ),
        ("unknown-type", PDF, "image/jpeg"),
    ] {
        let blob = files.verified(name, bytes);
        let request_id = Uuid::new_v4();
        assert!(matches!(
            store
                .register_verified(actor, space, request_id, blob, media(claimed_type, name))
                .await,
            Err(ContentError::Invalid(_))
        ));
        let counts: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM asset WHERE space_id=$1),(SELECT count(*) FROM upload_receipt WHERE actor_id=$2 AND request_id=$3)",
        )
        .bind(space)
        .bind(actor.actor_id)
        .bind(request_id)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
        assert_eq!(counts, (0, 0), "rejected media {name}");
    }

    // Opaque attachments may be registered only under the generic type.
    let opaque = files.verified("opaque", b"arbitrary attachment bytes");
    let generic = store
        .register_verified(
            actor,
            space,
            Uuid::new_v4(),
            opaque,
            media("application/octet-stream", "opaque"),
        )
        .await
        .unwrap();
    assert_eq!(generic.media.media_type, "application/octet-stream");

    let request_id = Uuid::new_v4();
    let valid_png = files.verified("valid-png", PNG);
    let first = store
        .register_verified(
            actor,
            space,
            request_id,
            valid_png.clone(),
            media("image/png", "valid.png"),
        )
        .await
        .unwrap();
    // A used request key has precedence over validating a different media
    // claim: this is still the old idempotency conflict, never a new receipt.
    assert!(matches!(
        store
            .register_verified(
                actor,
                space,
                request_id,
                valid_png,
                media("application/pdf", "changed.pdf"),
            )
            .await,
        Err(ContentError::IdempotencyConflict)
    ));
    let saved: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM upload_receipt WHERE actor_id=$1 AND request_id=$2 AND asset_id=$3",
    )
    .bind(actor.actor_id)
    .bind(request_id)
    .bind(first.reference.asset_id)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(saved, 1);
}

#[tokio::test]
async fn physical_dedup_does_not_merge_logical_assets_or_bypass_grants() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (owner, private_space) = rig.seed_actor_space(true).await;
    let (outsider, public_space) = rig.seed_actor_space(true).await;
    let store = AssetStore::new(rig.runtime_pool.clone(), files.store.clone());
    let blob = files.verified("shared.png", PNG);
    let private = store
        .register_verified(
            owner,
            private_space,
            Uuid::new_v4(),
            blob.clone(),
            media("image/png", "private.png"),
        )
        .await
        .unwrap();
    let public = store
        .register_verified(
            outsider,
            public_space,
            Uuid::new_v4(),
            blob.clone(),
            media("image/png", "public.png"),
        )
        .await
        .unwrap();
    assert_ne!(private.reference, public.reference);
    assert_eq!(private.storage_key, public.storage_key);
    assert!(matches!(
        store
            .register_verified(
                outsider,
                private_space,
                Uuid::new_v4(),
                blob,
                media("image/png", "intruder.png")
            )
            .await,
        Err(ContentError::NotFound)
    ));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM asset WHERE space_id=$1")
        .bind(private_space)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn source_segments_pin_exact_pdf_image_and_notebook_versions() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let store = AssetStore::new(rig.runtime_pool.clone(), files.store.clone());
    for (name, mime, bytes, selector) in [
        (
            "paper.pdf",
            "application/pdf",
            PDF,
            json!({"page": 7, "region": [0.1, 0.2, 0.3, 0.4]}),
        ),
        (
            "figure.png",
            "image/png",
            PNG,
            json!({"region": [10, 20, 30, 40]}),
        ),
        (
            "cells.ipynb",
            "application/x-ipynb+json",
            NOTEBOOK,
            json!({"cell_id": "analysis-2"}),
        ),
    ] {
        let blob = files.verified(name, bytes);
        let asset = store
            .register_verified(actor, space, Uuid::new_v4(), blob, media(mime, name))
            .await
            .unwrap();
        let version = store
            .link_resource_version(
                actor,
                ResourceInput {
                    space_id: space,
                    resource_id: None,
                    display_name: name.into(),
                },
                asset.reference.clone(),
            )
            .await
            .unwrap();
        let segment = store
            .add_source_segment(actor, version.clone(), selector.clone())
            .await
            .unwrap();
        let saved: (Uuid, Uuid, Uuid, Value) = sqlx::query_as("SELECT space_id,resource_id,resource_version_id,selector FROM source_segment WHERE id=$1")
            .bind(segment.segment_id)
            .fetch_one(&rig.admin_pool)
            .await
            .unwrap();
        assert_eq!(
            saved,
            (space, version.resource_id, version.version_id, selector)
        );
        let original: (Uuid, String) = sqlx::query_as("SELECT asset_id,a.sha256 FROM resource_version rv JOIN asset a ON (a.space_id,a.id)=(rv.space_id,rv.asset_id) WHERE rv.space_id=$1 AND rv.resource_id=$2 AND rv.id=$3")
            .bind(space)
            .bind(version.resource_id)
            .bind(version.version_id)
            .fetch_one(&rig.admin_pool)
            .await
            .unwrap();
        assert_eq!(original.0, asset.reference.asset_id);
        assert_eq!(original.1, asset.sha256);
    }
}

#[tokio::test]
async fn revoked_grant_hides_receipt_replay_and_resource_mutation() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let store = AssetStore::new(rig.runtime_pool.clone(), files.store.clone());
    let blob = files.verified("source.pdf", PDF);
    let request = Uuid::new_v4();
    let asset = store
        .register_verified(
            actor,
            space,
            request,
            blob.clone(),
            media("application/pdf", "source.pdf"),
        )
        .await
        .unwrap();
    let version = store
        .link_resource_version(
            actor,
            ResourceInput {
                space_id: space,
                resource_id: None,
                display_name: "Source".into(),
            },
            asset.reference,
        )
        .await
        .unwrap();
    rig.revoke(actor, space).await;
    assert!(matches!(
        store
            .register_verified(
                actor,
                space,
                request,
                blob,
                media("application/pdf", "source.pdf")
            )
            .await,
        Err(ContentError::NotFound)
    ));
    assert!(matches!(
        store
            .add_source_segment(actor, version, json!({"page": 1}))
            .await,
        Err(ContentError::NotFound)
    ));
}

#[tokio::test]
async fn changed_finalized_bytes_cannot_be_registered_as_ready() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let store = AssetStore::new(rig.runtime_pool.clone(), files.store.clone());
    let blob = files.verified("source.pdf", PDF);
    fs::write(
        files.root.join("assets").join(blob.storage_key()),
        b"corrupt",
    )
    .unwrap();
    let request = Uuid::new_v4();
    assert!(
        store
            .register_verified(
                actor,
                space,
                request,
                blob,
                media("application/pdf", "source.pdf")
            )
            .await
            .is_err()
    );
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM asset WHERE space_id=$1),(SELECT count(*) FROM request_key WHERE actor_id=$2 AND request_id=$3)",
    )
    .bind(space)
    .bind(actor.actor_id)
    .bind(request)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(counts, (0, 0));
}
