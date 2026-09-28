use super::*;
use learning_core::*;
use learning_db::{CompositionStore, ReleaseStore};
pub fn doc(targets: Vec<NodeTarget>) -> SaveComposition {
    SaveComposition {
        request_id: Uuid::new_v4(),
        composition_id: None,
        base_revision_id: None,
        kind: CompositionKind::Document,
        title: "Attention".into(),
        nodes: targets
            .into_iter()
            .map(|target| NodeDraft {
                occurrence_id: None,
                target,
            })
            .collect(),
        reason: "initial".into(),
    }
}
pub fn block(r: &Revision) -> NodeTarget {
    NodeTarget::Block(BlockRef {
        block_id: r.block_id,
        revision_id: r.revision_id,
    })
}
pub fn child(r: &CompositionRevision) -> NodeTarget {
    NodeTarget::Composition(r.reference.clone())
}
pub fn edit(r: &CompositionRevision) -> SaveComposition {
    SaveComposition {
        request_id: Uuid::new_v4(),
        composition_id: Some(r.reference.composition_id),
        base_revision_id: Some(r.reference.revision_id),
        kind: r.kind,
        title: r.title.clone(),
        nodes: r
            .nodes
            .iter()
            .map(|n| NodeDraft {
                occurrence_id: Some(n.occurrence_id),
                target: n.target.clone(),
            })
            .collect(),
        reason: "edit".into(),
    }
}
pub fn root(r: &CompositionRevision, release: Option<Uuid>) -> PublishRoot {
    PublishRoot {
        composition_id: r.reference.composition_id,
        revision_id: r.reference.revision_id,
        expected_head_revision_id: r.reference.revision_id,
        expected_publication_token: hex_digest(canonical_json(&serde_json::json!({"domain":"publication-basis-v1","composition_id":r.reference.composition_id,"last_release_id":release})).as_bytes()),
    }
}
pub fn publish(roots: Vec<PublishRoot>) -> PublishCommand {
    PublishCommand {
        request_id: Uuid::new_v4(),
        roots,
        reason: "publish".into(),
    }
}
impl TestRig {
    pub async fn structure_counts(&self, space: Uuid) -> (i64, i64, i64) {
        sqlx::query_as("SELECT (SELECT count(*) FROM composition WHERE space_id=$1),(SELECT count(*) FROM composition_occurrence WHERE space_id=$1),(SELECT count(*) FROM release_root WHERE space_id=$1)").bind(space).fetch_one(&self.admin_pool).await.unwrap()
    }
    pub fn compositions(&self) -> CompositionStore {
        CompositionStore::new(self.runtime_pool.clone())
    }
    pub fn releases(&self) -> ReleaseStore {
        ReleaseStore::new(self.runtime_pool.clone())
    }
    pub async fn grant(&self, actor: Principal, space: Uuid, write: bool) {
        sqlx::query("INSERT INTO space_grant(actor_id,space_id,can_write) VALUES($1,$2,$3) ON CONFLICT(actor_id,space_id) DO UPDATE SET can_write=$3").bind(actor.actor_id).bind(space).bind(write).execute(&self.admin_pool).await.unwrap();
    }
    pub async fn assembly_counts(&self, actor: Principal) -> (i64, i64, i64, i64, i64) {
        sqlx::query_as("SELECT (SELECT count(*) FROM composition_revision WHERE author_id=$1),(SELECT count(*) FROM release WHERE author_id=$1),(SELECT count(*) FROM outbox_event e JOIN release r ON r.id=e.aggregate_id WHERE r.author_id=$1),(SELECT count(*) FROM request_key WHERE actor_id=$1),(SELECT count(*) FROM composition_receipt WHERE actor_id=$1)+(SELECT count(*) FROM release_receipt WHERE actor_id=$1)").bind(actor.actor_id).fetch_one(&self.admin_pool).await.unwrap()
    }
}
