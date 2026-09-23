use super::{consumers, scope};
use crate::{lineage, storage};
use learning_core::*;
use sqlx::{Postgres, Transaction};
use std::collections::BTreeSet;
use uuid::Uuid;

const BATCH: i64 = 128;
const INTERNAL_SCAN_LIMIT: usize = 100_000;

// Exact input revision leads the predicate. The bounded keyset is ordered by
// the operation and member position, not by any hidden candidate count.
const REVERSE_INPUT: &str = "SELECT i.operation_id,i.position \
    FROM public.lineage_input i \
    JOIN public.lineage_operation o ON o.id=i.operation_id \
    JOIN public.space_grant g ON g.space_id=o.space_id AND g.actor_id=$1 \
    WHERE i.block_id=$2 AND i.revision_id=$3 \
      AND ($4::uuid IS NULL OR (i.operation_id,i.position)>($4,$5)) \
    ORDER BY i.operation_id,i.position LIMIT $6";

fn lineage_type(operation: LineageOperation) -> SystemLineageType {
    match operation {
        LineageOperation::Derive => SystemLineageType::DerivedFrom,
        LineageOperation::Split => SystemLineageType::SplitFrom,
        LineageOperation::Merge => SystemLineageType::MergedFrom,
    }
}

fn step(from: ImpactNode, to: ImpactNode, relation_type: SystemLineageType) -> ImpactStep {
    ImpactStep {
        from,
        to,
        family: ImpactFamily::Lineage,
        dependency_role: None,
        dependency_position: None,
        direction: Some(TraversalDirection::SavedReverse),
        relation_type: None,
        lineage_type: Some(relation_type),
        provenance: ImpactProvenance::Stored,
        location: None,
        reason: ImpactReason::DerivedFrom,
    }
}

fn push_visible(
    result: &mut Vec<ImpactStep>,
    budget: &mut VisibleWorkBudget,
    edge: ImpactStep,
) -> Result<(), consumers::DirectError> {
    edge.validate()?;
    budget
        .charge_node(edge.to.clone())
        .map_err(|_| consumers::DirectError::Budget)?;
    budget
        .charge_edge()
        .map_err(|_| consumers::DirectError::Budget)?;
    result.push(edge);
    Ok(())
}

async fn from_input(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    input: &BlockRef,
    fixed: &scope::FixedScope,
    budget: &mut VisibleWorkBudget,
) -> Result<Vec<ImpactStep>, consumers::DirectError> {
    if !fixed
        .members
        .contains_key(&ImpactStart::Block(input.clone()))
    {
        return Ok(vec![]);
    }
    let mut after: Option<(Uuid, i32)> = None;
    let mut scanned = 0usize;
    let mut seen_operations = BTreeSet::new();
    let mut result = vec![];
    loop {
        let rows: Vec<(Uuid, i32)> = sqlx::query_as(REVERSE_INPUT)
            .bind(actor.actor_id)
            .bind(input.block_id)
            .bind(input.revision_id)
            .bind(after.map(|v| v.0))
            .bind(after.map(|v| v.1))
            .bind(BATCH)
            .fetch_all(&mut **tx)
            .await
            .map_err(storage)?;
        let count = rows.len();
        for (operation_id, position) in rows {
            scanned += 1;
            if scanned > INTERNAL_SCAN_LIMIT {
                return Err(consumers::DirectError::Storage(ContentError::Storage));
            }
            after = Some((operation_id, position));
            if !seen_operations.insert(operation_id) {
                continue;
            }
            // B3 checks the operation grant and *every* input/output necessary
            // closure in the same caller-owned read-only RR transaction.
            let saved = match lineage::load_for_impact(tx, actor, operation_id).await {
                Ok(saved) => saved,
                Err(ContentError::NotFound) => continue,
                Err(error) => {
                    return Err(consumers::DirectError::Storage(consumers::internal_error(
                        error,
                    )));
                }
            };
            if !saved.inputs.contains(input)
                || !saved.outputs.iter().any(|output| {
                    fixed
                        .members
                        .contains_key(&ImpactStart::Block(output.clone()))
                })
            {
                continue;
            }
            push_visible(
                &mut result,
                budget,
                step(
                    ImpactNode::Block(input.clone()),
                    ImpactNode::Lineage { operation_id },
                    lineage_type(saved.operation),
                ),
            )?;
        }
        if count < BATCH as usize {
            break;
        }
    }
    Ok(result)
}

async fn from_operation(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    operation_id: Uuid,
    fixed: &scope::FixedScope,
    budget: &mut VisibleWorkBudget,
) -> Result<Vec<ImpactStep>, consumers::DirectError> {
    let saved = match lineage::load_for_impact(tx, actor, operation_id).await {
        Ok(saved) => saved,
        Err(ContentError::NotFound) => return Ok(vec![]),
        Err(error) => {
            return Err(consumers::DirectError::Storage(consumers::internal_error(
                error,
            )));
        }
    };
    if !saved.inputs.iter().any(|input| {
        fixed
            .members
            .contains_key(&ImpactStart::Block(input.clone()))
    }) {
        return Ok(vec![]);
    }
    let mut result = vec![];
    for output in saved.outputs {
        if !fixed
            .members
            .contains_key(&ImpactStart::Block(output.clone()))
        {
            continue;
        }
        push_visible(
            &mut result,
            budget,
            step(
                ImpactNode::Lineage { operation_id },
                ImpactNode::Block(output),
                lineage_type(saved.operation),
            ),
        )?;
    }
    Ok(result)
}

pub(super) async fn neighbors(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    current: &ImpactNode,
    fixed: &scope::FixedScope,
    budget: &mut VisibleWorkBudget,
) -> Result<Vec<ImpactStep>, consumers::DirectError> {
    match current {
        ImpactNode::Block(input) => from_input(tx, actor, input, fixed, budget).await,
        ImpactNode::Lineage { operation_id } => {
            from_operation(tx, actor, *operation_id, fixed, budget).await
        }
        _ => Ok(vec![]),
    }
}
