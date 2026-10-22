//! Independent native full-C content contract. A1 remains unsupported.
use super::SchemaContract;
use crate::BackupError;
#[cfg(any(test, target_os = "linux"))]
use serde_json::{Value, json};

const CONTRACT: &[u8] =
    include_bytes!("../../../tests/fixtures/c4-full-schema-c0-d1-v1/contract.json");
const CONTRACT_SHA256: &str = "3d510097d2ed7dcdf80ca7b756ae07c2ad0af3d3c6bedc259e57160e553b4f39";

impl SchemaContract {
    #[cfg(any(test, target_os = "linux"))]
    pub(crate) fn for_role_recipe(bytes: &[u8]) -> Result<Self, BackupError> {
        match crate::restore_policy::role_recipe_profile(bytes)? {
            crate::restore_policy::RoleProfile::BaseV1 => Self::embedded(),
            crate::restore_policy::RoleProfile::C0D1V1 => Self::c0_d1_v1(),
        }
    }
    pub(crate) fn c0_d1_contract_sha256() -> Result<&'static str, BackupError> {
        if crate::digest(CONTRACT) != CONTRACT_SHA256 {
            return Err(BackupError::Invalid("compiled full-C native contract hash"));
        }
        Ok(CONTRACT_SHA256)
    }

    #[cfg(any(test, target_os = "linux"))]
    pub(crate) fn c0_d1_v1() -> Result<Self, BackupError> {
        Self::c0_d1_contract_sha256()?;
        let mut selected: Value = super::canonical_json(
            CONTRACT,
            #[cfg(test)]
            None,
        )?;
        if selected["format_version"] != 2
            || selected["capability"] != "full_schema_content_c0_d1_v1"
            || selected["producer_sha256"]
                != "62b191b57f17ffdb44bf8e6255e9f41c7feba6712778b9df816f5646df4508da"
        {
            return Err(BackupError::Invalid(
                "compiled full-C native contract version",
            ));
        }
        // Reuse the private content validator after validating this independent
        // exact V2 include. This local metadata projection does not alias A1 or
        // admit caller contracts; all five native files remain byte-pinned.
        selected["format_version"] = json!(1);
        selected["capability"] = json!("full_schema_content_v1");
        super::load_inner(
            &serde_json::to_vec(&selected)?,
            [
                (
                    "pg18-schema.sql.in",
                    include_bytes!(
                        "../../../tests/fixtures/c4-full-schema-c0-d1-v1/pg18-schema.sql.in"
                    ),
                ),
                (
                    "pg18-toc.json",
                    include_bytes!("../../../tests/fixtures/c4-full-schema-c0-d1-v1/pg18-toc.json"),
                ),
                (
                    "columns.json",
                    include_bytes!("../../../tests/fixtures/c4-full-schema-c0-d1-v1/columns.json"),
                ),
                (
                    "acl.json",
                    include_bytes!("../../../tests/fixtures/c4-full-schema-c0-d1-v1/acl.json"),
                ),
            ],
            #[cfg(test)]
            None,
        )
    }

    #[cfg(any(test, target_os = "linux"))]
    pub(crate) fn c0_d1_profile_facts() -> Result<Value, BackupError> {
        let contract = Self::c0_d1_v1()?;
        let owned = std::str::from_utf8(&contract.owned_template)
            .map_err(|_| BackupError::Invalid("full-C owned SQL encoding"))?;
        let c0_names = [
            "p_job_id",
            "p_include_body",
            "job_octets",
            "outbox_octets",
            "observed_at",
            "job_row",
            "outbox_row",
        ];
        let d1_names = [
            "p_job_id",
            "p_token",
            "p_request",
            "p_expiry",
            "p_phase",
            "p_include_body",
            "job_octets",
            "outbox_octets",
            "request_octets",
            "key_octets",
            "receipt_octets",
            "observed_at",
            "job_row",
            "outbox_row",
            "request_row",
            "key_row",
            "receipt_row",
        ];
        let c0 = helper(
            owned,
            "p0c4_snapshot_claim_locked_observation",
            "2950 16",
            "{2950,16,20,20,1184,3802,3802}",
            &c0_names,
            "learning_snapshot_claim_broker",
            Some("knowweave.snapshot-claim.v1:C0_OBSERVATION_ONLY_NOT_EXPORTABLE"),
        )?;
        let d1 = helper(
            owned,
            "p0c4_snapshot_publication_observe_v1",
            "2950 2950 2950 1184 25 16",
            "{2950,2950,2950,1184,25,16,20,20,20,20,20,1184,3802,3802,3802,3802,3802}",
            &d1_names,
            "learning_runtime",
            None,
        )?;
        let functions = [("p0c3_claim_snapshot_job","2950 20"),("p0c3_fail_snapshot_job","2950 2950 25 16"),
            ("p0c3_renew_snapshot_job","2950 2950 20"),("p0c4_snapshot_claim_locked_observation","2950 16")]
            .map(|(name, types)| json!({"schema":"public","name":name,"types":types,
                "owner":"learning_admin","grantor":"learning_admin","privilege":"EXECUTE","grantable":false}));
        Ok(json!({"helpers":[c0,d1],"broker":{
            "name":"learning_snapshot_claim_broker","login":true,"inherit":false,
            "superuser":false,"createdb":false,"createrole":false,"replication":false,"bypassrls":false,
            "connection_limit":4,"valid_until_null":true,"connect":true,"create":false,"temp":false,
            "memberships":0,"settings":0,"ownerships":0,"direct_relation_acls":0,"direct_column_acls":0,"default_acls":0,
            "schema_acls":[{"schema":"public","grantor_is_owner":true,"privilege":"USAGE","grantable":false}],
            "function_acls":functions,
            "database_acls":[{"current_database":true,"grantor":"learning_admin","privilege":"CONNECT","grantable":false}],
            "extra_schema_access":false,"extra_definer_execute":false}}))
    }
}

#[cfg(any(test, target_os = "linux"))]
fn helper(
    owned: &str,
    name: &str,
    types: &str,
    all_types: &str,
    names: &[&str],
    grantee: &str,
    comment: Option<&str>,
) -> Result<Value, BackupError> {
    let marker = format!("CREATE FUNCTION public.{name}(");
    if owned.matches(&marker).count() != 1 {
        return Err(BackupError::Invalid("full-C fixed helper occurrence"));
    }
    let section = &owned[owned
        .find(&marker)
        .ok_or(BackupError::Invalid("full-C fixed helper missing"))?..];
    let (head, tail) = section
        .split_once("\n    AS $$")
        .ok_or(BackupError::Invalid("full-C native helper delimiter"))?;
    let (body, _) = tail
        .split_once("$$;\n")
        .ok_or(BackupError::Invalid("full-C native helper body"))?;
    let first = head
        .lines()
        .next()
        .ok_or(BackupError::Invalid("full-C native helper header"))?;
    let (signature, returns) = first
        .strip_prefix("CREATE FUNCTION public.")
        .and_then(|s| s.split_once(" RETURNS "))
        .ok_or(BackupError::Invalid("full-C native helper signature"))?;
    let definition = format!(
        "CREATE OR REPLACE FUNCTION public.{signature}\n RETURNS {returns}\n LANGUAGE plpgsql\n SECURITY DEFINER\n SET search_path TO 'pg_catalog'\n SET row_security TO 'on'\n SET \"TimeZone\" TO 'UTC'\nAS $function${body}$function$\n"
    );
    let nargs = types.split_whitespace().count();
    let modes = (0..names.len())
        .map(|i| if i < nargs { "i" } else { "t" })
        .collect::<Vec<_>>();
    Ok(
        json!({"schema":"public","name":name,"owner":"learning_admin","kind":"f","language":"plpgsql",
        "definition":definition,"body":body,"input_types":types,"all_types":all_types,"arg_names":names,"arg_modes":modes,
        "security_definer":true,"strict":false,"leakproof":false,"volatility":"v","parallel":"u",
        "config":["search_path=pg_catalog","row_security=on","TimeZone=UTC"],
        "acl":format!("{{learning_admin=X/learning_admin,{grantee}=X/learning_admin}}"),
        "nargs":nargs,"defaults":0,"variadic":0,"support":0,"cost":100,"rows":1000,
        "bin_null":true,"sqlbody_null":true,"argdefaults_null":true,"trftypes_null":true,
        "returns_set":true,"return_type":2249,"extension_member":false,"comment":comment}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reviewed_native_full_c_contract_is_independent_and_exact() {
        let legacy = SchemaContract::embedded().unwrap();
        let full = SchemaContract::c0_d1_v1().unwrap();
        assert_eq!(
            legacy.table_names().collect::<Vec<_>>(),
            full.table_names().collect::<Vec<_>>()
        );
        assert_eq!(full.migrations().len(), 15);
        assert_eq!(full.acl_catalog()["roles"].as_array().unwrap().len(), 4);
        assert_eq!(legacy.acl_catalog()["roles"].as_array().unwrap().len(), 3);
        let facts = SchemaContract::c0_d1_profile_facts().unwrap();
        assert_eq!(facts["helpers"].as_array().unwrap().len(), 2);
        assert!(
            facts["helpers"][0]["body"]
                .as_str()
                .unwrap()
                .starts_with("\nDECLARE")
        );
        assert!(
            facts["helpers"][1]["definition"]
                .as_str()
                .unwrap()
                .contains("p_expiry timestamp with time zone")
        );
    }
}
