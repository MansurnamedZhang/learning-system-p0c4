//! Strict inner full-C gate version. The outer captured proof stays exact12 V1.
use crate::{BackupError, GatePhase, SourceGateRecord, valid_digest};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceGateRecordV2 {
    format_version: u32,
    profile: String,
    base: SourceGateRecord,
    broker_oid: u32,
    schema_contract_sha256: String,
    initial_connect: bool,
    capture_connect_closed: bool,
    capture_sessions_drained: bool,
    release_connect_restored: bool,
}

fn closed(phase: GatePhase) -> bool {
    phase != GatePhase::Intent
}
fn drained(phase: GatePhase) -> bool {
    !matches!(phase, GatePhase::Intent | GatePhase::Closed)
}
impl SourceGateRecordV2 {
    fn from_observed_base(base: SourceGateRecord, broker_oid: u32, contract: &str) -> Self {
        let phase = base.phase();
        Self {
            format_version: 2,
            profile: "snapshot_c0_d1_v1".into(),
            base,
            broker_oid,
            schema_contract_sha256: contract.into(),
            initial_connect: true,
            capture_connect_closed: closed(phase),
            capture_sessions_drained: drained(phase),
            release_connect_restored: phase == GatePhase::Released,
        }
    }
    fn validate(&self, id: Uuid, contract: &str) -> Result<(), BackupError> {
        self.base.validate_for(id)?;
        let phase = self.base.phase();
        if self.format_version != 2
            || self.profile != "snapshot_c0_d1_v1"
            || self.broker_oid == 0
            || !valid_digest(contract)
            || self.schema_contract_sha256 != contract
            || !self.initial_connect
            || self.capture_connect_closed != closed(phase)
            || self.capture_sessions_drained != drained(phase)
            || self.release_connect_restored != (phase == GatePhase::Released)
            || self
                .base
                .pinned_manifest_sha256()
                .is_some_and(|pins| self.base.source_manifest_sha256() != Some(pins))
        {
            return Err(BackupError::Invalid(
                "full-C gate profile or observed phase",
            ));
        }
        Ok(())
    }
}

pub(crate) fn encode_gate_record(
    base: &SourceGateRecord,
    broker_oid: u32,
) -> Result<Vec<u8>, BackupError> {
    let digest = crate::full_restore::SchemaContract::c0_d1_contract_sha256()?;
    let wire = SourceGateRecordV2::from_observed_base(base.clone(), broker_oid, digest);
    let bytes = serde_json::to_vec(&wire)?;
    if bytes.len() > 4096 || broker_oid == 0 {
        return Err(BackupError::Invalid("full-C gate record boundary"));
    }
    Ok(bytes)
}

fn decode(bytes: &[u8], id: Uuid, digest: &str) -> Result<SourceGateRecordV2, BackupError> {
    if bytes.is_empty() || bytes.len() > 4096 {
        return Err(BackupError::Invalid("full-C gate record size"));
    }
    let record: SourceGateRecordV2 = serde_json::from_slice(bytes)?;
    record.validate(id, digest)?;
    if serde_json::to_vec(&record)? != bytes {
        return Err(BackupError::Invalid("full-C gate record canonical bytes"));
    }
    Ok(record)
}

pub(crate) fn decode_gate_record(
    bytes: &[u8],
    id: Uuid,
) -> Result<(SourceGateRecord, u32), BackupError> {
    let digest = crate::full_restore::SchemaContract::c0_d1_contract_sha256()?;
    let wire = decode(bytes, id, digest)?;
    Ok((wire.base, wire.broker_oid))
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SourceProfile {
    BaseV1,
    C0D1 { broker_oid: u32 },
}

#[cfg(target_os = "linux")]
impl SourceProfile {
    pub(super) fn from_broker_oid(oid: Option<u32>) -> Self {
        match oid {
            None => Self::BaseV1,
            Some(broker_oid) => Self::C0D1 { broker_oid },
        }
    }
    pub(super) async fn admit(connection: &mut sqlx::PgConnection) -> Result<Self, BackupError> {
        let present: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_' AND p.proname IN ('p0c4_snapshot_claim_locked_observation','p0c4_snapshot_publication_observe_v1','p0c4_snapshot_locked_observation'))",
        ).fetch_one(&mut *connection).await?;
        if !present {
            return Ok(Self::BaseV1);
        }
        let broker_oid: i64 = sqlx::query_scalar("SELECT oid::pg_catalog.int8 FROM pg_catalog.pg_roles WHERE rolname='learning_snapshot_claim_broker'")
            .fetch_one(&mut *connection).await?;
        let profile = Self::C0D1 {
            broker_oid: u32::try_from(broker_oid)
                .map_err(|_| BackupError::Invalid("full-C broker identity"))?,
        };
        profile.recheck(connection, true).await?;
        Ok(profile)
    }

    pub(super) fn broker_oid(self) -> Option<u32> {
        match self {
            Self::BaseV1 => None,
            Self::C0D1 { broker_oid } => Some(broker_oid),
        }
    }

    pub(super) async fn recheck(
        self,
        connection: &mut sqlx::PgConnection,
        connect: bool,
    ) -> Result<(), BackupError> {
        let Self::C0D1 { broker_oid } = self else {
            return Ok(());
        };
        let current_oid: i64 = sqlx::query_scalar("SELECT oid::pg_catalog.int8 FROM pg_catalog.pg_roles WHERE rolname='learning_snapshot_claim_broker'")
            .fetch_one(&mut *connection).await?;
        if current_oid != i64::from(broker_oid) {
            return Err(BackupError::Invalid("full-C broker identity drift"));
        }
        let bounded: bool = sqlx::query_scalar(
            "SELECT (SELECT count(*)=2 AND bool_and(p.prokind='f' AND p.prosqlbody IS NULL AND p.proargdefaults IS NULL AND pg_catalog.octet_length(p.prosrc)<=32768 AND pg_catalog.pg_column_size(p.proconfig)<=4096 AND pg_catalog.pg_column_size(p.proargnames)<=4096 AND coalesce(pg_catalog.pg_column_size(p.probin),0)<=4096) FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE p.proname IN ('p0c4_snapshot_claim_locked_observation','p0c4_snapshot_publication_observe_v1','p0c4_snapshot_locked_observation') AND n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_') AND (SELECT count(*)<=1 FROM pg_catalog.pg_namespace n,LATERAL pg_catalog.aclexplode(n.nspacl)a WHERE a.grantee=$1::pg_catalog.int8::pg_catalog.oid) AND (SELECT count(*)<=4 FROM pg_catalog.pg_proc p,LATERAL pg_catalog.aclexplode(p.proacl)a WHERE a.grantee=$1::pg_catalog.int8::pg_catalog.oid) AND (SELECT count(*)<=1 FROM pg_catalog.pg_database d,LATERAL pg_catalog.aclexplode(d.datacl)a WHERE a.grantee=$1::pg_catalog.int8::pg_catalog.oid)",
        ).bind(i64::from(broker_oid)).fetch_one(&mut *connection).await?;
        if !bounded {
            return Err(BackupError::Capacity("full-C source profile catalog"));
        }
        let actual: serde_json::Value = sqlx::query_scalar(include_str!("c0_d1_catalog.sql"))
            .fetch_one(connection)
            .await?;
        let mut expected = crate::full_restore::SchemaContract::c0_d1_profile_facts()?;
        if !connect {
            expected["broker"]["connect"] = serde_json::json!(false);
            expected["broker"]["database_acls"] = serde_json::Value::Null;
        }
        if actual != expected {
            return Err(BackupError::Invalid(
                "full-C source helper or broker profile differs",
            ));
        }
        Ok(())
    }

    pub(super) fn revoke_statement(self, database: &str) -> String {
        match self {
            Self::BaseV1 => {
                format!("REVOKE CONNECT ON DATABASE \"{database}\" FROM PUBLIC, learning_runtime")
            }
            Self::C0D1 { .. } => format!(
                "REVOKE CONNECT ON DATABASE \"{database}\" FROM PUBLIC, learning_runtime, learning_snapshot_claim_broker"
            ),
        }
    }

    pub(super) fn grant_statement(self, database: &str) -> String {
        match self {
            Self::BaseV1 => format!("GRANT CONNECT ON DATABASE \"{database}\" TO learning_runtime"),
            Self::C0D1 { .. } => format!(
                "GRANT CONNECT ON DATABASE \"{database}\" TO learning_runtime, learning_snapshot_claim_broker"
            ),
        }
    }
}

#[cfg(target_os = "linux")]
pub(super) async fn role_recipe(
    connection: &mut sqlx::PgConnection,
) -> Result<Vec<u8>, BackupError> {
    let one_membership: bool = sqlx::query_scalar("SELECT count(*)=1 FROM pg_catalog.pg_auth_members a JOIN pg_catalog.pg_roles r ON r.oid=a.roleid JOIN pg_catalog.pg_roles m ON m.oid=a.member WHERE r.rolname IN ('learning_admin','learning_auth_lock','learning_runtime','learning_snapshot_claim_broker') OR m.rolname IN ('learning_admin','learning_auth_lock','learning_runtime','learning_snapshot_claim_broker')")
        .fetch_one(&mut *connection).await?;
    if !one_membership {
        return Err(BackupError::Invalid("full-C role membership roster"));
    }
    let mut recipe: serde_json::Value = sqlx::query_scalar(
        "SELECT pg_catalog.jsonb_build_object('format_version',2,'roles',(SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object('name',rolname::pg_catalog.text,'login',rolcanlogin,'inherit',rolinherit,'superuser',rolsuper,'createdb',rolcreatedb,'createrole',rolcreaterole,'bypassrls',rolbypassrls,'replication',rolreplication,'connection_limit',rolconnlimit) ORDER BY rolname) FROM pg_catalog.pg_roles WHERE rolname IN ('learning_admin','learning_auth_lock','learning_runtime','learning_snapshot_claim_broker')),'memberships',(SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object('role',r.rolname::pg_catalog.text,'member',m.rolname::pg_catalog.text,'inherit',a.inherit_option,'set',a.set_option,'admin',a.admin_option) ORDER BY r.rolname,m.rolname) FROM pg_catalog.pg_auth_members a JOIN pg_catalog.pg_roles r ON r.oid=a.roleid JOIN pg_catalog.pg_roles m ON m.oid=a.member WHERE r.rolname IN ('learning_admin','learning_auth_lock','learning_runtime','learning_snapshot_claim_broker') OR m.rolname IN ('learning_admin','learning_auth_lock','learning_runtime','learning_snapshot_claim_broker')))",
    ).fetch_one(connection).await?;
    recipe["components"] = serde_json::json!([{
        "id":"snapshot_c0_d1", "version":1,
        "schema_contract_sha256":crate::full_restore::SchemaContract::c0_d1_contract_sha256()?,
    }]);
    let bytes = serde_json::to_vec(&recipe)?;
    crate::validate_role_recipe(&bytes)?;
    Ok(bytes)
}

/// Content validator for the destination's single source-released reader.
/// This does not replace its outer manifest/protection/signature/hash checks.
pub(crate) fn validate_full_c_released_record(
    bytes: &[u8],
    id: Uuid,
    manifest_sha256: &str,
) -> Result<SourceGateRecord, BackupError> {
    let digest = crate::full_restore::SchemaContract::c0_d1_contract_sha256()?;
    let record = decode(bytes, id, digest)?;
    if !valid_digest(manifest_sha256)
        || record.base.phase() != GatePhase::Released
        || record.base.pinned_manifest_sha256() != Some(manifest_sha256)
        || record.base.source_manifest_sha256() != Some(manifest_sha256)
    {
        return Err(BackupError::Invalid("full-C released record manifest"));
    }
    Ok(record.base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_c_gate_requires_observed_phase_and_one_exact_contract() {
        let id = Uuid::new_v4();
        let hash = "a".repeat(64);
        let mut base = SourceGateRecord::new(id);
        for phase in [
            GatePhase::Intent,
            GatePhase::Closed,
            GatePhase::Drained,
            GatePhase::DumpAndIndexDurable,
            GatePhase::PinsDurable,
            GatePhase::ReleaseReady,
            GatePhase::Released,
        ] {
            if phase != GatePhase::Intent {
                base.advance(
                    phase,
                    matches!(
                        phase,
                        GatePhase::DumpAndIndexDurable | GatePhase::PinsDurable
                    )
                    .then_some(hash.as_str()),
                )
                .unwrap();
            }
            let wire = SourceGateRecordV2::from_observed_base(base.clone(), 42, &hash);
            let bytes = serde_json::to_vec(&wire).unwrap();
            decode(&bytes, id, &hash).unwrap();
            assert!(decode(&bytes, Uuid::new_v4(), &hash).is_err());
            assert!(decode(&bytes, id, &"b".repeat(64)).is_err());
            for key in [
                "initial_connect",
                "capture_connect_closed",
                "capture_sessions_drained",
                "release_connect_restored",
            ] {
                let mut changed = serde_json::to_value(&wire).unwrap();
                changed[key] = serde_json::json!(!changed[key].as_bool().unwrap());
                assert!(decode(&serde_json::to_vec(&changed).unwrap(), id, &hash).is_err());
            }
        }
        let legacy = serde_json::to_vec(&base).unwrap();
        assert!(decode(&legacy, id, &hash).is_err());
        let mut changed = SourceGateRecordV2::from_observed_base(base, 0, &hash);
        assert!(decode(&serde_json::to_vec(&changed).unwrap(), id, &hash).is_err());
        changed.broker_oid = 42;
        changed.profile = "observation_v1".into();
        assert!(decode(&serde_json::to_vec(&changed).unwrap(), id, &hash).is_err());
    }
}
