use super::ReadingStore;
use crate::{
    authorization,
    overlay::{model, persist},
    references, request, storage,
};
use learning_core::*;
use sqlx::Row;
use uuid::Uuid;

pub(crate) async fn choices(
    tx: &mut model::Tx<'_>,
    view: &ReadingRef,
) -> Result<(u32, ReadingSelections), ContentError> {
    let row = sqlx::query(
        "SELECT contract_version,evidence FROM reading_view_revision WHERE view_id=$1 AND id=$2",
    )
    .bind(view.view_id)
    .bind(view.revision_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage)?
    .ok_or(ContentError::NotFound)?;
    Ok((
        row.get::<i32, _>("contract_version") as u32,
        row.get::<Option<sqlx::types::Json<ReadingSelections>>, _>("evidence")
            .map(|j| j.0)
            .unwrap_or_default(),
    ))
}
fn scope_allowed(scope: &RelationScope, overlay: Uuid) -> bool {
    match scope {
        RelationScope::Space { .. } => true,
        RelationScope::PersonalOverlay { overlay_id, .. } => *overlay_id == overlay,
    }
}
pub(crate) async fn authorize(
    tx: &mut model::Tx<'_>,
    actor: Principal,
    overlay: Uuid,
    choices: &ReadingSelections,
    session: &mut references::Session,
) -> Result<(), ContentError> {
    for s in &choices.selections {
        let rel = session
            .object(tx, actor, &ExactRef::Relation(s.relation.clone()))
            .await?
            .ok_or(ContentError::NotFound)?
            .relation()?;
        if !scope_allowed(&rel.scope, overlay) {
            return Err(ContentError::NotFound);
        }
        if let Some(r) = &s.review
            && !session
                .authorize(tx, actor, &ExactRef::RelationReview(r.clone()))
                .await?
        {
            return Err(ContentError::NotFound);
        }
    }
    for r in &choices.epistemic_reviews {
        let review = session
            .object(tx, actor, &ExactRef::EpistemicReview(r.clone()))
            .await?
            .ok_or(ContentError::NotFound)?
            .epistemic_review()?;
        if !scope_allowed(&review.scope, overlay) {
            return Err(ContentError::NotFound);
        }
    }
    Ok(())
}
pub(crate) async fn project(
    tx: &mut model::Tx<'_>,
    actor: Principal,
    choices: &ReadingSelections,
    session: &mut references::Session,
) -> Result<ReadingEvidence, ContentError> {
    let mut out = ReadingEvidence::default();
    for s in &choices.selections {
        let Some(rel) = session
            .object(tx, actor, &ExactRef::Relation(s.relation.clone()))
            .await?
        else {
            continue;
        };
        let review = if let Some(r) = &s.review {
            let root = ExactRef::RelationReview(r.clone());
            if !references::directly_visible(tx, actor, &root).await? {
                None
            } else {
                Some(match session.object(tx, actor, &root).await? {
                    Some(o) => ReviewProjection::Available(o.relation_review()?),
                    None => ReviewProjection::Incomplete,
                })
            }
        } else {
            None
        };
        out.selections.push(SelectedRelationProjection {
            relation: rel.relation()?,
            review,
        });
    }
    for r in &choices.epistemic_reviews {
        let root = ExactRef::EpistemicReview(r.clone());
        if !references::directly_visible(tx, actor, &root).await? {
            continue;
        }
        out.epistemic_reviews
            .push(match session.object(tx, actor, &root).await? {
                Some(o) => ReviewProjection::Available(o.epistemic_review()?),
                None => ReviewProjection::Incomplete,
            });
    }
    Ok(out)
}
pub(crate) struct NewView<'a> {
    pub id: Uuid,
    pub overlay_revision: Uuid,
    pub parent: Option<Uuid>,
    pub version: u32,
    pub choices: &'a ReadingSelections,
}
pub(crate) async fn insert_view(
    tx: &mut model::Tx<'_>,
    actor: Principal,
    layer: &model::Layer,
    new: NewView<'_>,
) -> Result<(), ContentError> {
    let NewView {
        id,
        overlay_revision,
        parent,
        version,
        choices,
    } = new;
    sqlx::query("INSERT INTO reading_view_revision(id,view_id,overlay_id,overlay_revision_id,parent_revision_id,author_id,contract_version,evidence) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(id).bind(layer.view.view_id).bind(layer.id).bind(overlay_revision).bind(parent).bind(actor.actor_id).bind(version as i32).bind((version==2).then_some(sqlx::types::Json(choices))).execute(&mut **tx).await.map_err(storage)?;
    for (position, s) in choices.selections.iter().enumerate() {
        sqlx::query("INSERT INTO reading_relation_selection(view_id,view_revision_id,position,relation_id,relation_revision_id,review_id) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(layer.view.view_id).bind(id).bind(position as i32).bind(s.relation.relation_id).bind(s.relation.revision_id).bind(s.review.as_ref().map(|r|r.review_id)).execute(&mut **tx).await.map_err(storage)?;
    }
    for (position, r) in choices.epistemic_reviews.iter().enumerate() {
        sqlx::query("INSERT INTO reading_epistemic_selection(view_id,view_revision_id,position,stream_id,review_id) VALUES($1,$2,$3,$4,$5)")
            .bind(layer.view.view_id).bind(id).bind(position as i32).bind(r.stream_id).bind(r.review_id).execute(&mut **tx).await.map_err(storage)?;
    }
    Ok(())
}
impl ReadingStore {
    pub async fn select_relations(
        &self,
        actor: Principal,
        id: Uuid,
        command: SelectRelations,
    ) -> Result<ReadingSaved, ContentError> {
        command.validate()?;
        let digest = command.digest(actor, id);
        let mut tx = request::begin(&self.pool, actor, command.request_id).await?;
        let layer =
            model::load(&mut tx, actor, id, Some(command.expected_overlay_revision)).await?;
        let requested = command.choices();
        let mut session = references::Session::default();
        authorize(&mut tx, actor, id, &requested, &mut session).await?;
        let mut spaces = session.spaces();
        spaces.push((layer.space, true));
        authorization::lock_grants(&mut tx, actor, &spaces).await?;
        let mut locked = references::Session::default();
        authorize(&mut tx, actor, id, &requested, &mut locked).await?;
        if locked.spaces().iter().any(|s| !spaces.contains(s)) {
            return Err(ContentError::NotFound);
        }
        if request::check(
            &mut tx,
            actor,
            command.request_id,
            &digest,
            "reading_select",
        )
        .await?
        {
            let saved = persist::receipt(&mut tx, actor, command.request_id)
                .await?
                .ok_or(ContentError::Storage)?;
            let fixed = model::load_view(&mut tx, actor, saved.view.clone()).await?;
            let (_, selected) = choices(&mut tx, &fixed.view).await?;
            authorize(&mut tx, actor, id, &selected, &mut locked).await?;
            tx.commit().await.map_err(storage)?;
            return Ok(saved);
        }
        // Both identities participate in CAS, even though only the view changes.
        model::lock_heads(
            &mut tx,
            actor,
            &layer,
            command.expected_overlay_revision,
            command.expected_reading_view_revision,
        )
        .await?;
        let prior = ReadingRef {
            view_id: layer.view.view_id,
            revision_id: command.expected_reading_view_revision,
        };
        let (_, previous) = choices(&mut tx, &prior).await?;
        if previous == requested {
            return Err(ContentError::Invalid("no_change".into()));
        }
        let revision_id = Uuid::new_v4();
        insert_view(
            &mut tx,
            actor,
            &layer,
            NewView {
                id: revision_id,
                overlay_revision: layer.revision,
                parent: Some(prior.revision_id),
                version: 2,
                choices: &requested,
            },
        )
        .await?;
        sqlx::query("UPDATE reading_view SET head_revision_id=$1 WHERE id=$2")
            .bind(revision_id)
            .bind(prior.view_id)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        let saved = ReadingSaved {
            overlay: OverlayRef {
                overlay_id: id,
                revision_id: layer.revision,
            },
            view: ReadingRef {
                view_id: prior.view_id,
                revision_id,
            },
            changed_blocks: vec![],
        };
        persist::record(
            &mut tx,
            actor,
            command.request_id,
            &digest,
            "reading_select",
            &saved,
        )
        .await?;
        tx.commit().await.map_err(storage)?;
        Ok(saved)
    }
}
