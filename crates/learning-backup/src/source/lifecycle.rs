//! Explicit source-only recovery. Abandonment never advances the legacy journal.
use crate::{BackupError, GatePhase, SourceGateRecord, digest, valid_digest};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Ready {
    format_version: u32,
    capability: String,
    backup_id: Uuid,
    action: String,
    source_binding_sha256: String,
    journal_phase: GatePhase,
    journal_record_sha256: String,
    retained_artifacts: String,
    state: String,
}
impl Ready {
    pub(super) fn new(
        id: Uuid,
        binding: &str,
        record: &SourceGateRecord,
    ) -> Result<Self, BackupError> {
        record.validate_for(id)?;
        if record.phase() == GatePhase::Released || !valid_digest(binding) {
            return Err(BackupError::Invalid(
                "abandonment conflicts with successful source",
            ));
        }
        Ok(Self {
            format_version: 1,
            capability: "source_abandonment_v1".into(),
            backup_id: id,
            action: "abandon".into(),
            source_binding_sha256: binding.into(),
            journal_phase: record.phase(),
            journal_record_sha256: digest(&serde_json::to_vec(record)?),
            retained_artifacts: "keep_all".into(),
            state: "release_ready".into(),
        })
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Terminal {
    format_version: u32,
    capability: String,
    backup_id: Uuid,
    action: String,
    source_binding_sha256: String,
    ready_sha256: String,
    state: String,
}
impl Terminal {
    pub(super) fn new(id: Uuid, binding: &str, ready: &[u8]) -> Self {
        Self {
            format_version: 1,
            capability: "source_abandonment_v1".into(),
            backup_id: id,
            action: "abandon".into(),
            source_binding_sha256: binding.into(),
            ready_sha256: digest(ready),
            state: "abandoned".into(),
        }
    }
}
fn bounded(bytes: &[u8]) -> Result<(), BackupError> {
    if bytes.is_empty() || bytes.len() > 4096 {
        return Err(BackupError::Invalid("abandonment record length"));
    }
    Ok(())
}
pub(super) fn parse_ready(
    bytes: &[u8],
    id: Uuid,
    binding: &str,
    record: &SourceGateRecord,
) -> Result<(), BackupError> {
    bounded(bytes)?;
    let actual: Ready = serde_json::from_slice(bytes)?;
    let expected = Ready::new(id, binding, record)?;
    if serde_json::to_vec(&actual)? != bytes || serde_json::to_vec(&expected)? != bytes {
        return Err(BackupError::Invalid("abandonment ready authority"));
    }
    Ok(())
}
pub(super) fn parse_terminal(
    bytes: &[u8],
    id: Uuid,
    binding: &str,
    ready: &[u8],
) -> Result<(), BackupError> {
    bounded(bytes)?;
    if id.is_nil() || !valid_digest(binding) {
        return Err(BackupError::Invalid("abandonment identity"));
    }
    let actual: Terminal = serde_json::from_slice(bytes)?;
    if serde_json::to_vec(&actual)? != bytes
        || serde_json::to_vec(&Terminal::new(id, binding, ready))? != bytes
    {
        return Err(BackupError::Invalid("abandonment terminal authority"));
    }
    Ok(())
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Decision {
    Pending,
    Ready,
    Abandoned,
}
pub(super) fn decision(
    ready: Option<&[u8]>,
    terminal: Option<&[u8]>,
    id: Uuid,
    binding: &str,
    record: &SourceGateRecord,
) -> Result<Decision, BackupError> {
    match (ready, terminal) {
        (None, None) => Ok(Decision::Pending),
        (None, Some(_)) => Err(BackupError::Invalid("orphan abandonment terminal")),
        (Some(r), t) => {
            parse_ready(r, id, binding, record)?;
            if let Some(t) = t {
                parse_terminal(t, id, binding, r)?;
                Ok(Decision::Abandoned)
            } else {
                Ok(Decision::Ready)
            }
        }
    }
}
pub(super) fn finish_eligible(phase: GatePhase, abandonment: bool) -> bool {
    !abandonment
        && matches!(
            phase,
            GatePhase::DumpAndIndexDurable
                | GatePhase::PinsDurable
                | GatePhase::ReleaseReady
                | GatePhase::Released
        )
}
pub(super) fn canonical_v4(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| {
        !id.is_nil()
            && id.to_string() == value
            && id.get_version_num() == 4
            && id.get_variant() == uuid::Variant::RFC4122
    })
}
pub(super) fn temp_name(name: &str) -> bool {
    name.strip_prefix(".tmp-").is_some_and(canonical_v4)
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::source::{
        SourceAdmission, SourceBackupConfig, SourceLocalPin, binding, collect_source_identity,
        compensate_release, inspect_gate, verify_isolation_attestation,
        wait_for_runtime_connect_probe,
    };
    use crate::{BackupManifestV1, SourceGateJournal};
    use learning_assets::backup_fs::{BackupDir, BackupEntryKind};
    use sqlx::PgPool;
    use std::{
        collections::BTreeSet,
        io::{Read, Write},
        os::unix::fs::MetadataExt,
        time::{Duration, Instant},
    };

    fn binding_sha() -> Result<&'static str, BackupError> {
        let value = option_env!("KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256").ok_or(
            BackupError::Invalid("source lifecycle lacks compile binding"),
        )?;
        if !valid_digest(value) {
            return Err(BackupError::Invalid("source lifecycle binding digest"));
        }
        Ok(value)
    }
    fn canonical_id(value: &str) -> Result<Uuid, BackupError> {
        let id =
            Uuid::parse_str(value).map_err(|_| BackupError::Invalid("unknown source authority"))?;
        if id.is_nil() || id.to_string() != value {
            return Err(BackupError::Invalid("noncanonical source authority"));
        }
        Ok(id)
    }
    fn bytes(dir: &BackupDir, name: &str, partial: bool) -> Result<Vec<u8>, BackupError> {
        let file = dir.open_file(name)?;
        let m = file.metadata()?;
        if m.uid() != 0
            || m.mode() & 0o7777 != 0o600
            || m.nlink() != 1
            || m.len() > 4096
            || (!partial && m.len() == 0)
        {
            return Err(BackupError::Invalid("private source authority file"));
        }
        let mut value = Vec::new();
        file.take(4097).read_to_end(&mut value)?;
        if value.len() > 4096 {
            return Err(BackupError::Invalid("source authority size"));
        }
        Ok(value)
    }
    fn sidecar(
        root: &BackupDir,
        id: Uuid,
        record: &SourceGateRecord,
    ) -> Result<Option<(BackupDir, Decision)>, BackupError> {
        let name = format!("{id}.abandonment");
        let dir = match root.open_dir(&name) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let state = read_sidecar(&dir, id, record)?;
        if state == Decision::Abandoned {
            #[cfg(test)]
            crate::source::lifecycle_tests::hook("scan_abandonment_before_child_sync")?;
            dir.sync()?;
            #[cfg(test)]
            crate::source::lifecycle_tests::hook("scan_abandonment_after_child_sync")?;
            if read_sidecar(&dir, id, record)? != Decision::Abandoned {
                return Err(BackupError::Invalid(
                    "abandonment terminal durability readback",
                ));
            }
        }
        Ok(Some((dir, state)))
    }
    fn read_sidecar(
        dir: &BackupDir,
        id: Uuid,
        record: &SourceGateRecord,
    ) -> Result<Decision, BackupError> {
        dir.require_private_directory()?;
        let mut ready = None;
        let mut terminal = None;
        for name in dir.list()? {
            match name.as_str() {
                "ready.json" => ready = Some(bytes(dir, &name, false)?),
                "abandoned.json" => terminal = Some(bytes(dir, &name, false)?),
                _ if temp_name(&name) => {
                    bytes(dir, &name, true)?;
                }
                _ => return Err(BackupError::Invalid("unknown abandonment entry")),
            }
        }
        let state = decision(
            ready.as_deref(),
            terminal.as_deref(),
            id,
            binding_sha()?,
            record,
        )?;
        Ok(state)
    }
    fn staging(root: &BackupDir, id: Uuid) -> Result<(), BackupError> {
        let dir = root.open_dir(&format!("{id}.journal-staging"))?;
        dir.require_private_directory()?;
        for name in dir.list()? {
            if let Some(token) = name.strip_prefix("initial-") {
                if !canonical_v4(token) {
                    return Err(BackupError::Invalid("journal staging initial name"));
                }
                let initial = dir.open_dir(&name)?;
                initial.require_private_directory()?;
                for name in initial.list()? {
                    if name != "intent.json" {
                        return Err(BackupError::Invalid("journal initial staging entry"));
                    }
                    bytes(&initial, &name, true)?;
                }
            } else {
                let stem = name
                    .strip_suffix(".tmp")
                    .ok_or(BackupError::Invalid("journal staging name"))?;
                let cut = stem
                    .len()
                    .checked_sub(37)
                    .ok_or(BackupError::Invalid("journal staging name"))?;
                if !stem.is_char_boundary(cut)
                    || !stem.is_char_boundary(cut + 1)
                    || stem.as_bytes()[cut] != b'-'
                {
                    return Err(BackupError::Invalid("journal staging name"));
                }
                let (prefix, tail) = stem.split_at(cut);
                let token = &tail[1..];
                if !canonical_v4(token)
                    || ![
                        "intent",
                        "closed",
                        "drained",
                        "dump-and-index-durable",
                        "pins-durable",
                        "release-ready",
                        "released",
                    ]
                    .contains(&prefix)
                {
                    return Err(BackupError::Invalid("journal staging phase"));
                }
                bytes(&dir, &name, true)?;
            }
        }
        dir.sync()?;
        Ok(())
    }
    pub(crate) fn scan(root: &BackupDir, target: Option<Uuid>) -> Result<(), BackupError> {
        let entries = root.list()?;
        let mut journals = BTreeSet::new();
        let mut related = BTreeSet::new();
        for name in &entries {
            if name == "source-binding.json" {
                continue;
            }
            let mut known = false;
            for suffix in [
                ".control",
                ".source",
                ".journal-staging",
                ".abandonment",
                ".release-recovery",
            ] {
                if let Some(value) = name.strip_suffix(suffix) {
                    let id = canonical_id(value)?;
                    if root.kind(name)? != BackupEntryKind::Directory {
                        return Err(BackupError::Invalid("source authority is not directory"));
                    }
                    if suffix == ".control" {
                        journals.insert(id);
                    } else {
                        related.insert(id);
                    }
                    if suffix == ".journal-staging" {
                        staging(root, id)?;
                    }
                    known = true;
                    break;
                }
            }
            if !known {
                return Err(BackupError::Invalid("unknown source control entry"));
            }
        }
        if related.iter().any(|id| !journals.contains(id)) {
            return Err(BackupError::Invalid("orphan source authority"));
        }
        for id in journals {
            let directory = root.open_dir(&format!("{id}.control"))?;
            directory.require_private_directory()?;
            for name in directory.list()? {
                bytes(&directory, &name, false)?;
            }
            let journal = SourceGateJournal::recover_in(root, id)?;
            let state = sidecar(root, id, journal.record())?;
            let terminal = if let Some((_, decision)) = state {
                decision == Decision::Abandoned
            } else {
                journal.record().phase() == GatePhase::Released
            };
            if !terminal && target != Some(id) {
                return Err(BackupError::Invalid(
                    "unfinished source maintenance journal",
                ));
            }
        }
        Ok(())
    }
    struct Context {
        admission: SourceAdmission,
        control: BackupDir,
        pin_root: BackupDir,
        journal: SourceGateJournal,
        source: Option<BackupDir>,
        sealed: Option<BackupDir>,
        sidecar: Option<(BackupDir, Decision)>,
    }
    fn optional_dir(root: &BackupDir, name: &str) -> Result<Option<BackupDir>, BackupError> {
        match root.open_dir(name) {
            Ok(d) => Ok(Some(d)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn role_safe(f: &crate::GateInspection) -> bool {
        f.admin_is_database_owner
            && !f.public_can_connect
            && !f.runtime_can_inherit_admin
            && !f.runtime_is_privileged
            && f.other_login_writers == 0
    }
    fn released_safe(f: &crate::GateInspection) -> bool {
        role_safe(f) && f.runtime_can_connect && f.other_sessions == 0
    }
    async fn admit(pool: &PgPool, config: &SourceBackupConfig) -> Result<Context, BackupError> {
        if config.backup_id.is_nil()
            || config.drain_timeout.is_zero()
            || config.drain_timeout > Duration::from_secs(60)
            || config.control_root == config.local_pin_root
        {
            return Err(BackupError::Invalid("source lifecycle configuration"));
        }
        let mut admission = SourceAdmission::try_acquire(pool, &config.expected_database).await?;
        let control = binding::admit_control_root(&mut admission, &config.control_root).await?;
        let journal = SourceGateJournal::recover_in(&control, config.backup_id)?;
        scan(&control, Some(config.backup_id))?;
        let sidecar = sidecar(&control, config.backup_id, journal.record())?;
        let facts = inspect_gate(admission.connection()).await?;
        if !role_safe(&facts) {
            return Err(BackupError::Invalid("source lifecycle role preflight"));
        }
        let pin_root = BackupDir::open_trusted_private_root(&config.local_pin_root)?;
        if pin_root.identity()? == control.identity()? {
            return Err(BackupError::Invalid("source control and pin root alias"));
        }
        let source = optional_dir(&control, &format!("{}.source", config.backup_id))?;
        let sealed = optional_dir(&pin_root, &format!("{}.sealed", config.backup_id))?;
        verify_isolation_attestation(config, true)?;
        Ok(Context {
            admission,
            control,
            pin_root,
            journal,
            source,
            sealed,
            sidecar,
        })
    }
    async fn close_and_drain(
        ctx: &mut Context,
        config: &SourceBackupConfig,
    ) -> Result<(), BackupError> {
        let database = ctx.admission.database().to_owned();
        sqlx::query(&format!(
            "REVOKE CONNECT ON DATABASE \"{database}\" FROM PUBLIC, learning_runtime"
        ))
        .execute(ctx.admission.connection())
        .await?;
        wait_for_runtime_connect_probe(config).await?;
        let deadline = Instant::now() + config.drain_timeout;
        loop {
            let facts = inspect_gate(ctx.admission.connection()).await?;
            if facts.validate().is_ok() {
                return Ok(());
            }
            if !role_safe(&facts) || facts.runtime_can_connect || Instant::now() >= deadline {
                return Err(BackupError::Invalid(
                    "source lifecycle sessions did not drain",
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    async fn proof(
        ctx: &mut Context,
        config: &SourceBackupConfig,
    ) -> Result<SourceLocalPin, BackupError> {
        let source = ctx
            .source
            .as_ref()
            .ok_or(BackupError::Invalid("source capture missing"))?;
        let sealed = ctx
            .sealed
            .as_ref()
            .ok_or(BackupError::Invalid("published sealed pin missing"))?;
        let found = source.list()?.into_iter().collect::<BTreeSet<_>>();
        if found
            != BTreeSet::from([
                "manifest.json".into(),
                "asset-index.json".into(),
                "database.dump".into(),
                "roles.json".into(),
            ])
        {
            return Err(BackupError::Invalid("source exact four files"));
        }
        let manifest_bytes = crate::sealed::read_small(source, "manifest.json", 64 * 1024 * 1024)?;
        let manifest: BackupManifestV1 = serde_json::from_slice(&manifest_bytes)?;
        if manifest.backup_id != config.backup_id || manifest.canonical_bytes()? != manifest_bytes {
            return Err(BackupError::Invalid("source canonical manifest identity"));
        }
        let index = crate::sealed::read_small(
            source,
            "asset-index.json",
            crate::MAX_ASSET_INDEX_BYTES as u64,
        )?;
        manifest.validate_with_index(&index)?;
        if manifest.source != collect_source_identity(ctx.admission.connection()).await? {
            return Err(BackupError::Invalid(
                "source compiled or current database identity",
            ));
        }
        for path in ["database.dump", "roles.json", "asset-index.json"] {
            let record = manifest
                .files
                .iter()
                .find(|r| r.path == path)
                .ok_or(BackupError::Invalid("source required file record"))?;
            crate::sealed::check_file(source, record)?;
        }
        let mut magic = [0u8; 5];
        source.open_file("database.dump")?.read_exact(&mut magic)?;
        if &magic != b"PGDMP" {
            return Err(BackupError::Invalid("source PGDMP magic"));
        }
        let sha = digest(&manifest_bytes);
        if ctx.journal.record().source_manifest_sha256() != Some(sha.as_str())
            || ctx
                .journal
                .record()
                .pinned_manifest_sha256()
                .is_some_and(|pin| pin != sha)
        {
            return Err(BackupError::Invalid("source journal actual digest"));
        }
        ctx.pin_root.sync()?;
        let pin = crate::sealed::verify_directory(sealed, config.backup_id)?;
        if pin.manifest_sha256() != sha
            || crate::sealed::read_small(sealed, "manifest.json", 64 * 1024 * 1024)?
                != manifest_bytes
            || crate::sealed::read_small(
                sealed,
                "asset-index.json",
                crate::MAX_ASSET_INDEX_BYTES as u64,
            )? != index
        {
            return Err(BackupError::Invalid("source and sealed bytes disagree"));
        }
        Ok(SourceLocalPin {
            sealed: pin,
            manifest,
        })
    }
    fn atomic(dir: &BackupDir, name: &str, value: &[u8]) -> Result<(), BackupError> {
        bounded(value)?;
        let temp = format!(".tmp-{}", Uuid::new_v4());
        let mut file = dir.create_file(&temp)?;
        #[cfg(test)]
        crate::source::lifecycle_tests::hook("sidecar_before_write")?;
        file.write_all(value)?;
        #[cfg(test)]
        crate::source::lifecycle_tests::hook("sidecar_before_file_sync")?;
        file.sync_all()?;
        dir.sync()?;
        #[cfg(test)]
        crate::source::lifecycle_tests::hook("sidecar_before_rename")?;
        dir.rename_noreplace_without_sync(&temp, name)?;
        #[cfg(test)]
        crate::source::lifecycle_tests::hook("sidecar_after_rename")?;
        dir.sync()?;
        #[cfg(test)]
        crate::source::lifecycle_tests::hook("sidecar_before_readback")?;
        if bytes(dir, name, false)? != value {
            return Err(BackupError::Invalid("abandonment publication readback"));
        }
        Ok(())
    }
    async fn before_release(
        ctx: &mut Context,
        config: &SourceBackupConfig,
    ) -> Result<(), BackupError> {
        inspect_gate(ctx.admission.connection()).await?.validate()?;
        verify_isolation_attestation(config, false)
    }
    async fn grant(ctx: &mut Context) -> Result<(), BackupError> {
        #[cfg(test)]
        crate::source::lifecycle_tests::hook("lifecycle_before_grant")?;
        let db = ctx.admission.database().to_owned();
        sqlx::query(&format!(
            "GRANT CONNECT ON DATABASE \"{db}\" TO learning_runtime"
        ))
        .execute(ctx.admission.connection())
        .await?;
        #[cfg(test)]
        crate::source::lifecycle_tests::hook("lifecycle_after_grant")?;
        let facts = inspect_gate(ctx.admission.connection()).await?;
        if !released_safe(&facts) {
            return Err(BackupError::Invalid("source release safety inspection"));
        }
        Ok(())
    }
    async fn compensated_error(ctx: &mut Context, error: BackupError) -> BackupError {
        let database = ctx.admission.database().to_owned();
        match compensate_release(&mut ctx.admission, &database).await {
            Ok(()) => error,
            Err(ambiguity) => ambiguity,
        }
    }
    pub(crate) async fn finish(
        pool: &PgPool,
        config: &SourceBackupConfig,
    ) -> Result<SourceLocalPin, BackupError> {
        let mut ctx = admit(pool, config).await?;
        if !finish_eligible(ctx.journal.record().phase(), ctx.sidecar.is_some()) {
            return Err(BackupError::Invalid("source attempt cannot finish"));
        }
        if ctx.journal.record().phase() == GatePhase::Released {
            let pin = proof(&mut ctx, config).await?;
            if !released_safe(&inspect_gate(ctx.admission.connection()).await?) {
                return Err(BackupError::Invalid(
                    "terminal source ACL ambiguous; manual recovery required",
                ));
            }
            ctx.admission.close().await?;
            return Ok(pin);
        }
        close_and_drain(&mut ctx, config).await?;
        let pin = proof(&mut ctx, config).await?;
        if ctx.journal.record().phase() == GatePhase::DumpAndIndexDurable {
            ctx.journal
                .advance(GatePhase::PinsDurable, Some(pin.sealed().manifest_sha256()))?;
        }
        before_release(&mut ctx, config).await?;
        if ctx.journal.record().phase() == GatePhase::PinsDurable {
            ctx.journal.advance(GatePhase::ReleaseReady, None)?;
        }
        before_release(&mut ctx, config).await?;
        let result = async {
            grant(&mut ctx).await?;
            ctx.journal.advance(GatePhase::Released, None)
        }
        .await;
        if let Err(error) = result {
            return Err(compensated_error(&mut ctx, error).await);
        }
        ctx.admission.close().await?;
        Ok(pin)
    }
    pub(crate) async fn abandon(
        pool: &PgPool,
        config: &SourceBackupConfig,
    ) -> Result<(), BackupError> {
        let mut ctx = admit(pool, config).await?;
        if ctx.journal.record().phase() == GatePhase::Released {
            return Err(BackupError::Invalid(
                "successful source cannot be abandoned",
            ));
        }
        if ctx
            .sidecar
            .as_ref()
            .is_some_and(|(_, s)| *s == Decision::Abandoned)
        {
            ctx.sidecar
                .as_ref()
                .expect("checked terminal sidecar")
                .0
                .sync()?;
            ctx.control.sync()?;
            if !released_safe(&inspect_gate(ctx.admission.connection()).await?) {
                return Err(BackupError::Invalid(
                    "terminal abandonment ACL ambiguous; manual recovery required",
                ));
            }
            ctx.admission.close().await?;
            return Ok(());
        }
        close_and_drain(&mut ctx, config).await?;
        let dir = if let Some((dir, _)) = ctx.sidecar.take() {
            dir
        } else {
            ctx.control
                .create_dir(&format!("{}.abandonment", config.backup_id))?
        };
        dir.require_private_directory()?;
        let ready = serde_json::to_vec(&Ready::new(
            config.backup_id,
            binding_sha()?,
            ctx.journal.record(),
        )?)?;
        match dir.open_file("ready.json") {
            Ok(_) => {
                if bytes(&dir, "ready.json", false)? != ready {
                    return Err(BackupError::Invalid("abandonment decision changed"));
                }
                dir.sync()?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                atomic(&dir, "ready.json", &ready)?
            }
            Err(e) => return Err(e.into()),
        }
        #[cfg(test)]
        crate::source::lifecycle_tests::hook("abandon_ready_durable")?;
        before_release(&mut ctx, config).await?;
        let terminal =
            serde_json::to_vec(&Terminal::new(config.backup_id, binding_sha()?, &ready))?;
        let result = async {
            grant(&mut ctx).await?;
            atomic(&dir, "abandoned.json", &terminal)
        }
        .await;
        if let Err(error) = result {
            return Err(compensated_error(&mut ctx, error).await);
        }
        ctx.admission.close().await?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub(super) use linux::{abandon, finish, scan};
