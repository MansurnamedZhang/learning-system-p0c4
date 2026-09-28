use super::{LineageStore, read};
use crate::{
    authorization,
    block_write::{PendingContent, insert_content},
    references, request, storage,
};
use learning_core::*;
use serde_json::json;
use uuid::Uuid;

impl LineageStore {
    pub async fn apply(
        &self,
        actor: Principal,
        output_space: Uuid,
        command: LineageCommand,
    ) -> Result<LineageSaved, ContentError> {
        command.validate()?;
        let digest = hex_digest(canonical_json(&json!({"domain":"lineage-request-v1","actor_id":actor.actor_id,"space_id":output_space,"command":command})).as_bytes());
        let mut tx = request::begin(&self.pool, actor, command.request_id).await?;
        let mut roots: Vec<_> = command
            .inputs
            .iter()
            .cloned()
            .map(ExactRef::Block)
            .collect();
        roots.extend(
            command
                .outputs
                .iter()
                .flat_map(ContentDraft::dependencies)
                .map(|d| d.target),
        );
        let discovered = references::load(&mut tx, actor, &roots).await?;
        let mut spaces = discovered.spaces();
        spaces.push((output_space, true));
        authorization::lock_grants(&mut tx, actor, &spaces).await?;
        let checked = references::load(&mut tx, actor, &roots).await?;
        if checked.spaces() != discovered.spaces() {
            return Err(ContentError::Invalid(
                "reference_authorization_changed".into(),
            ));
        }
        if request::check(&mut tx, actor, command.request_id, &digest, "lineage_apply").await? {
            let id = sqlx::query_scalar("SELECT operation_id FROM public.lineage_receipt WHERE actor_id=$1 AND request_id=$2").bind(actor.actor_id).bind(command.request_id).fetch_one(&mut *tx).await.map_err(storage)?;
            let saved = read::load(&mut tx, actor, id).await?;
            tx.commit().await.map_err(storage)?;
            return Ok(saved);
        }
        let operation_id = Uuid::new_v4();
        let kind = serde_json::to_value(command.operation).map_err(storage)?;
        sqlx::query("INSERT INTO public.lineage_operation(id,space_id,operation,author_id,reason,input_count,output_count) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(operation_id).bind(output_space).bind(kind.as_str().ok_or(ContentError::Storage)?).bind(actor.actor_id).bind(&command.reason).bind(command.inputs.len() as i32).bind(command.outputs.len() as i32).execute(&mut *tx).await.map_err(storage)?;
        let outputs: Vec<_> = command
            .outputs
            .iter()
            .map(|_| BlockRef {
                block_id: Uuid::new_v4(),
                revision_id: Uuid::new_v4(),
            })
            .collect();
        // New identities are inserted in sorted lock order; command order remains
        // on lineage_output.position and does not depend on UUID ordering.
        let mut order: Vec<_> = (0..outputs.len()).collect();
        order.sort_by_key(|&i| outputs[i].block_id);
        for i in order {
            let output = &outputs[i];
            sqlx::query("INSERT INTO public.block(id,space_id,head_revision_id) VALUES($1,$2,$3)")
                .bind(output.block_id)
                .bind(output_space)
                .bind(output.revision_id)
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
            insert_content(
                &mut tx,
                PendingContent {
                    block_id: output.block_id,
                    space_id: output_space,
                    revision_id: output.revision_id,
                    parent: None,
                    actor,
                    draft: &command.outputs[i],
                    reason: &command.reason,
                },
            )
            .await?;
        }
        for (i, input) in command.inputs.iter().enumerate() {
            let space = checked
                .objects
                .get(&ExactRef::Block(input.clone()))
                .ok_or(ContentError::Storage)?
                .0;
            sqlx::query("INSERT INTO public.lineage_input(operation_id,position,space_id,block_id,revision_id) VALUES($1,$2,$3,$4,$5)").bind(operation_id).bind(i as i32).bind(space).bind(input.block_id).bind(input.revision_id).execute(&mut *tx).await.map_err(storage)?;
        }
        for (i, output) in outputs.iter().enumerate() {
            sqlx::query("INSERT INTO public.lineage_output(operation_id,position,space_id,block_id,revision_id) VALUES($1,$2,$3,$4,$5)").bind(operation_id).bind(i as i32).bind(output_space).bind(output.block_id).bind(output.revision_id).execute(&mut *tx).await.map_err(storage)?;
        }
        // Includes new bodies as well as all shared input/output dependency
        // closures. Over-budget operations roll back all new rows.
        let saved = read::load(&mut tx, actor, operation_id).await?;
        request::register(&mut tx, actor, command.request_id, &digest, "lineage_apply").await?;
        sqlx::query("INSERT INTO public.lineage_receipt(actor_id,request_id,operation_id,request_sha256) VALUES($1,$2,$3,$4)").bind(actor.actor_id).bind(command.request_id).bind(operation_id).bind(digest).execute(&mut *tx).await.map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(saved)
    }
}
