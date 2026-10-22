//! Independent full-C target birth envelope. V1 bytes remain unchanged.
use super::TargetBirthAttestation;
use crate::BackupError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    format_version: u32,
    profile: String,
    base: TargetBirthAttestation,
    schema_contract_sha256: String,
    source_role_recipe_sha256: String,
    broker: BrokerBirth,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrokerBirth {
    name: String,
    oid: u32,
    login: bool,
    inherit: bool,
    superuser: bool,
    createdb: bool,
    createrole: bool,
    bypassrls: bool,
    replication: bool,
    connection_limit: i32,
    valid_until_null: bool,
    password_null: bool,
    connect: bool,
}

#[derive(Debug, Clone)]
pub(super) struct FullCBirthProfile {
    schema_contract_sha256: String,
    source_role_recipe_sha256: String,
    broker: BrokerBirth,
}
impl FullCBirthProfile {
    #[cfg(target_os = "linux")]
    pub(super) fn bind_roles(&self, raw: &[u8]) -> Result<(), BackupError> {
        if crate::restore_policy::role_recipe_profile(raw)?
            != crate::restore_policy::RoleProfile::C0D1V1
            || crate::digest(raw) != self.source_role_recipe_sha256
            || self.schema_contract_sha256
                != crate::full_restore::SchemaContract::c0_d1_contract_sha256()?
        {
            return Err(BackupError::Invalid("full-C birth role or schema binding"));
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    pub(super) async fn verify_live(
        &self,
        connection: &mut sqlx::PgConnection,
    ) -> Result<(), BackupError> {
        // Password NULL was measured twice by the root provisioner and pinned
        // in this envelope. The unprivileged owner never reads pg_authid.
        let actual: serde_json::Value = sqlx::query_scalar(LIVE_SQL).fetch_one(connection).await?;
        let mut expected = serde_json::to_value(&self.broker)?;
        expected
            .as_object_mut()
            .expect("broker object")
            .remove("password_null");
        expected["safe_acl"] = serde_json::json!(true);
        if actual != expected {
            return Err(BackupError::Invalid(
                "full-C target broker identity or ACL drift",
            ));
        }
        Ok(())
    }
}

pub(super) fn parse(bytes: &[u8]) -> Result<TargetBirthAttestation, BackupError> {
    parse_with_contract(
        bytes,
        crate::full_restore::SchemaContract::c0_d1_contract_sha256()?,
    )
}

fn parse_with_contract(
    bytes: &[u8],
    contract: &str,
) -> Result<TargetBirthAttestation, BackupError> {
    let wire: Envelope = serde_json::from_slice(bytes)?;
    if serde_json::to_vec(&wire)? != bytes
        || wire.format_version != 2
        || wire.profile != "snapshot_c0_d1_v1"
        || wire.base.format_version != 1
        || wire.base.full_c_profile.is_some()
        || !crate::valid_digest(contract)
        || wire.schema_contract_sha256 != contract
        || !crate::valid_digest(&wire.source_role_recipe_sha256)
        || wire.broker.name != "learning_snapshot_claim_broker"
        || wire.broker.oid == 0
        || wire.broker.login
        || wire.broker.inherit
        || wire.broker.superuser
        || wire.broker.createdb
        || wire.broker.createrole
        || wire.broker.bypassrls
        || wire.broker.replication
        || wire.broker.connection_limit != 4
        || !wire.broker.valid_until_null
        || !wire.broker.password_null
        || wire.broker.connect
    {
        return Err(BackupError::Invalid("full-C target birth profile"));
    }
    let mut birth = wire.base;
    birth.full_c_profile = Some(FullCBirthProfile {
        schema_contract_sha256: wire.schema_contract_sha256,
        source_role_recipe_sha256: wire.source_role_recipe_sha256,
        broker: wire.broker,
    });
    Ok(birth)
}

// A role may acquire only the fixed C0 schema/function grants during import.
// No direct row/column/default grant, membership, ownership, setting, database
// permission or unrelated function/schema ACL is admitted at any stage.
#[cfg(target_os = "linux")]
const LIVE_SQL: &str = r#"SELECT pg_catalog.jsonb_build_object(
'name',r.rolname,'oid',r.oid::pg_catalog.int8,'login',r.rolcanlogin,'inherit',r.rolinherit,
'superuser',r.rolsuper,'createdb',r.rolcreatedb,'createrole',r.rolcreaterole,
'bypassrls',r.rolbypassrls,'replication',r.rolreplication,'connection_limit',r.rolconnlimit,
'valid_until_null',r.rolvaliduntil IS NULL,
'connect',pg_catalog.has_database_privilege(r.oid,pg_catalog.current_database(),'CONNECT'),
'safe_acl',NOT EXISTS(SELECT FROM pg_catalog.pg_auth_members m WHERE m.member=r.oid OR m.roleid=r.oid)
 AND NOT EXISTS(SELECT FROM pg_catalog.pg_db_role_setting s WHERE s.setrole=r.oid)
 AND NOT EXISTS(SELECT FROM pg_catalog.pg_shdepend d WHERE d.refclassid='pg_catalog.pg_authid'::pg_catalog.regclass AND d.refobjid=r.oid AND d.deptype='o')
 AND NOT EXISTS(SELECT FROM pg_catalog.pg_class c,LATERAL pg_catalog.aclexplode(c.relacl) a WHERE a.grantee=r.oid)
 AND NOT EXISTS(SELECT FROM pg_catalog.pg_attribute c,LATERAL pg_catalog.aclexplode(c.attacl) a WHERE a.grantee=r.oid)
 AND NOT EXISTS(SELECT FROM pg_catalog.pg_default_acl d,LATERAL pg_catalog.aclexplode(d.defaclacl) a WHERE a.grantee=r.oid)
 AND NOT EXISTS(SELECT FROM pg_catalog.pg_database d,LATERAL pg_catalog.aclexplode(d.datacl) a WHERE a.grantee=r.oid)
 AND NOT EXISTS(SELECT FROM pg_catalog.pg_namespace n,LATERAL pg_catalog.aclexplode(n.nspacl) a WHERE a.grantee=r.oid AND NOT(n.nspname='public' AND a.grantor=n.nspowner AND a.privilege_type='USAGE' AND NOT a.is_grantable))
 AND NOT EXISTS(SELECT FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace,LATERAL pg_catalog.aclexplode(p.proacl) a WHERE a.grantee=r.oid AND NOT(n.nspname='public' AND pg_catalog.pg_get_userbyid(p.proowner)='learning_admin' AND a.grantor=p.proowner AND a.privilege_type='EXECUTE' AND NOT a.is_grantable AND ((p.proname='p0c3_claim_snapshot_job' AND p.proargtypes::pg_catalog.text='2950 20') OR (p.proname='p0c3_fail_snapshot_job' AND p.proargtypes::pg_catalog.text='2950 2950 25 16') OR (p.proname='p0c3_renew_snapshot_job' AND p.proargtypes::pg_catalog.text='2950 2950 20') OR (p.proname='p0c4_snapshot_claim_locked_observation' AND p.proargtypes::pg_catalog.text='2950 16'))))
 AND NOT pg_catalog.has_database_privilege(r.oid,pg_catalog.current_database(),'CREATE,TEMP')
 AND NOT EXISTS(SELECT FROM pg_catalog.pg_namespace n WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_' AND (pg_catalog.has_schema_privilege(r.oid,n.oid,'CREATE') OR (n.nspname<>'public' AND pg_catalog.has_schema_privilege(r.oid,n.oid,'USAGE'))))
 AND NOT EXISTS(SELECT FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_' AND p.prosecdef AND pg_catalog.has_function_privilege(r.oid,p.oid,'EXECUTE') AND NOT(n.nspname='public' AND ((p.proname='p0c3_claim_snapshot_job' AND p.proargtypes::pg_catalog.text='2950 20') OR (p.proname='p0c3_fail_snapshot_job' AND p.proargtypes::pg_catalog.text='2950 2950 25 16') OR (p.proname='p0c3_renew_snapshot_job' AND p.proargtypes::pg_catalog.text='2950 2950 20') OR (p.proname='p0c4_snapshot_claim_locked_observation' AND p.proargtypes::pg_catalog.text='2950 16'))))
) FROM pg_catalog.pg_roles r WHERE r.rolname='learning_snapshot_claim_broker'"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_c_birth_retains_one_profile_and_refuses_weaker_target_role() {
        // Typed metadata exercise only; this is not a native full-C fixture.
        let base: TargetBirthAttestation =
            serde_json::from_slice(include_bytes!("../../tests/fixtures/c4_birth_python.json"))
                .unwrap();
        let contract = "a".repeat(64);
        let wire = Envelope {
            format_version: 2,
            profile: "snapshot_c0_d1_v1".into(),
            base,
            schema_contract_sha256: contract.clone(),
            source_role_recipe_sha256: "b".repeat(64),
            broker: BrokerBirth {
                name: "learning_snapshot_claim_broker".into(),
                oid: 42,
                login: false,
                inherit: false,
                superuser: false,
                createdb: false,
                createrole: false,
                bypassrls: false,
                replication: false,
                connection_limit: 4,
                valid_until_null: true,
                password_null: true,
                connect: false,
            },
        };
        let bytes = serde_json::to_vec(&wire).unwrap();
        let accepted = parse_with_contract(&bytes, &contract).unwrap();
        let profile = accepted.full_c_profile.unwrap();
        assert_eq!(profile.broker.oid, 42);
        assert_eq!(profile.schema_contract_sha256, contract);
        assert_eq!(profile.source_role_recipe_sha256, "b".repeat(64));
        assert!(parse_with_contract(&bytes, &"c".repeat(64)).is_err());
        for key in [
            "login",
            "inherit",
            "superuser",
            "createdb",
            "createrole",
            "bypassrls",
            "replication",
            "connect",
            "password_null",
            "valid_until_null",
        ] {
            let mut bad = serde_json::to_value(&wire).unwrap();
            bad["broker"][key] = serde_json::json!(!bad["broker"][key].as_bool().unwrap());
            let bad: Envelope = serde_json::from_value(bad).unwrap();
            assert!(
                parse_with_contract(&serde_json::to_vec(&bad).unwrap(), &contract).is_err(),
                "{key}"
            );
        }
        let mut bad = serde_json::to_value(&wire).unwrap();
        bad["broker"]["extra_password"] = serde_json::json!("not admitted");
        assert!(serde_json::from_value::<Envelope>(bad).is_err());
        let mut newline = bytes;
        newline.push(b'\n');
        assert!(parse_with_contract(&newline, &contract).is_err());
    }
}
