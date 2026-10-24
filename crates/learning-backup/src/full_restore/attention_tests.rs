//! Task7 RED baseline: a real isolated import must contain the Attention graph.
//! The unimplemented fixture is observed as missing data, never a new authority.
use super::*;
use crate::full_restore::attention_fixture::HypothesisHistory;
use serde_json::json;
use std::io::Write;

#[derive(Clone, Copy)]
enum Case {
    FullChain,
    MissingAsset,
    CorruptDump,
    InterruptedTransfer,
    UnknownEffect,
    RootReplacement,
}
impl Case {
    fn name(self) -> &'static str {
        match self {
            Self::FullChain => "attention-full-chain",
            Self::MissingAsset => "attention-missing-asset",
            Self::CorruptDump => "attention-corrupt-dump",
            Self::InterruptedTransfer => "attention-interrupted-transfer",
            Self::UnknownEffect => "attention-unknown-effect",
            Self::RootReplacement => "attention-root-replacement",
        }
    }
}

tokio::task_local! { static ATTENTION: Case; }

pub(super) fn active() -> bool {
    ATTENTION.try_with(|_| ()).is_ok()
}

pub(super) async fn after_import(
    target: &mut ImportedTarget,
    expected: &crate::full_restore::data::ExpectedRecoveryRows,
) -> Result<(), BackupError> {
    if !active() {
        return Ok(());
    }
    // Exercise the original all-table PK-ordered row verifier after actual
    // capture/import, before any lease normalization or semantic acceptance.
    let scratch = target
        .recovery_control()?
        .create_dir("attention-pre-normalization")?;
    let rows =
        crate::full_restore::data::verify_target_rows(target.connection(), expected, &scratch)
            .await?;
    if rows.table_count() != 64 {
        return Err(BackupError::Invalid("attention complete table roster"));
    }
    let (historical, current): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE content::text LIKE '%H@1%'), \
         count(*) FILTER (WHERE content::text LIKE '%H@2%') \
         FROM public.block_revision",
    )
    .fetch_one(target.connection())
    .await?;
    let history = HypothesisHistory {
        historical: u64::try_from(historical).map_err(|_| BackupError::Overflow)?,
        current: u64::try_from(current).map_err(|_| BackupError::Overflow)?,
    };
    let proof = json!({
        "format_version":1,
        "classification":"ATTENTION_RED_BASELINE_ACTUAL_INVENTORY_NOT_ACCEPTANCE",
        "case":ATTENTION.with(|case| case.name()),
        "all_table_count":rows.table_count(),
        "pre_normalization_rows":rows.evidence(),
        "hypothesis_history":history,
        "missing_behavior":"COMPLETE_ATTENTION_SOURCE_FIXTURE_AND_SEMANTIC_COMPARISON",
        "complete_backup":false,
        "usable":false,
        "task7_complete":false,
        "stop_required":true
    });
    let bytes = serde_json::to_vec(&proof)?;
    if bytes.len() > 1024 * 1024 {
        return Err(BackupError::Capacity("attention RED observation"));
    }
    let root = BackupDir::open_trusted_private_root(std::path::Path::new(
        "/var/lib/knowweave-source/proofs",
    ))?;
    let mut file = root.create_file("full-attention-red-observation.json")?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    root.sync()?;
    // This is the behavior RED: the current baseline really has no H@1/H@2
    // Attention graph. Missing environment/PG/permission failures occur earlier
    // and cannot satisfy the expected RED observation or be credited as RED.
    history.require_complete()
}

async fn run(case: Case) {
    ATTENTION
        .scope(case, real_full_import_rehearsal(FullFault::Success))
        .await
        .unwrap();
}

macro_rules! native_case {
    ($name:ident, $case:ident) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "fresh Root Attention profile; initial RED baseline is not GREEN implementation"]
        async fn $name() {
            run(Case::$case).await;
        }
    };
}
native_case!(
    attention_full_chain_preserves_authoritative_inventory_and_originals,
    FullChain
);
native_case!(attention_missing_asset, MissingAsset);
native_case!(attention_corrupt_dump, CorruptDump);
native_case!(attention_interrupted_transfer, InterruptedTransfer);
native_case!(attention_unknown_effect, UnknownEffect);
native_case!(attention_root_replacement, RootReplacement);
