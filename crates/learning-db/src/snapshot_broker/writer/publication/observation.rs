//! Private D1 row observation: explicit owner install, catalog-only validation.
use learning_core::ContentError;
use sqlx::{PgPool, Postgres, Transaction};

pub(super) const SIGNATURE: &str =
    "public.p0c4_snapshot_publication_observe_v1(uuid,uuid,uuid,timestamptz,text,boolean)";
pub(super) const SOURCE: &str = include_str!("observation.sql");

const OWNER: &str = r#"
SELECT EXISTS (
 SELECT FROM pg_catalog.pg_roles a CROSS JOIN pg_catalog.pg_roles r
 JOIN pg_catalog.pg_database d ON d.datname=pg_catalog.current_database()
 JOIN pg_catalog.pg_namespace n ON n.nspname='public'
 WHERE a.rolname='learning_admin' AND r.rolname='learning_runtime'
 AND current_user=a.rolname AND session_user=a.rolname AND d.datdba=a.oid
 AND a.rolcanlogin AND a.rolinherit AND NOT a.rolsuper AND NOT a.rolcreatedb
 AND NOT a.rolcreaterole AND NOT a.rolreplication AND NOT a.rolbypassrls AND a.rolvaliduntil IS NULL
 AND r.rolcanlogin AND r.rolinherit AND NOT r.rolsuper AND NOT r.rolcreatedb
 AND NOT r.rolcreaterole AND NOT r.rolreplication AND NOT r.rolbypassrls AND r.rolvaliduntil IS NULL
 AND n.nspowner IN (a.oid,'pg_database_owner'::pg_catalog.regrole)
 AND pg_catalog.has_schema_privilege(a.oid,n.oid,'USAGE')
 AND pg_catalog.has_schema_privilege(a.oid,n.oid,'CREATE')
 AND NOT pg_catalog.has_schema_privilege(r.oid,n.oid,'CREATE')
 AND NOT EXISTS (SELECT FROM pg_catalog.pg_db_role_setting s WHERE s.setrole IN (a.oid,r.oid))
 AND NOT EXISTS (SELECT FROM pg_catalog.pg_auth_members m
  WHERE m.member=r.oid OR m.roleid IN (a.oid,r.oid)
   OR (m.member=a.oid AND NOT (m.roleid='learning_auth_lock'::pg_catalog.regrole
        AND NOT m.admin_option AND NOT m.inherit_option AND m.set_option)))
)
"#;
const PROFILE: &str = r#"
SELECT pg_catalog.count(*)=1 AND COALESCE(pg_catalog.bool_and(
 p.oid=pg_catalog.to_regprocedure($1) AND n.nspname='public'
 AND p.proowner='learning_admin'::pg_catalog.regrole AND l.lanname='plpgsql'
 AND p.prokind='f' AND p.provolatile='v' AND p.proparallel='u'
 AND p.prosecdef AND NOT p.proisstrict AND NOT p.proleakproof
 AND p.pronargdefaults=0 AND p.provariadic=0 AND p.prosupport=0
 AND p.probin IS NULL AND p.prosqlbody IS NULL AND p.proargdefaults IS NULL AND p.protrftypes IS NULL
 AND p.procost=100 AND p.prorows=1000 AND p.pronargs=6
 AND p.proargtypes='2950 2950 2950 1184 25 16'::pg_catalog.oidvector
 AND p.proretset AND p.prorettype=2249
 AND p.proallargtypes=ARRAY[2950,2950,2950,1184,25,16,20,20,20,20,20,1184,3802,3802,3802,3802,3802]::pg_catalog.oid[]
 AND p.proargmodes=ARRAY['i','i','i','i','i','i','t','t','t','t','t','t','t','t','t','t','t']::pg_catalog."char"[]
 AND p.proargnames=ARRAY['p_job_id','p_token','p_request','p_expiry','p_phase','p_include_body',
  'job_octets','outbox_octets','request_octets','key_octets','receipt_octets','observed_at',
  'job_row','outbox_row','request_row','key_row','receipt_row']::pg_catalog.text[]
 AND pg_catalog.cardinality(p.proconfig)=3
 AND p.proconfig @> ARRAY['search_path=pg_catalog','row_security=on','TimeZone=UTC']::pg_catalog.text[]
 AND p.prosrc=$2
 AND NOT EXISTS (SELECT FROM pg_catalog.pg_depend d
  WHERE d.classid='pg_catalog.pg_proc'::pg_catalog.regclass AND d.objid=p.oid AND d.deptype='e')
 AND (SELECT pg_catalog.count(*)=2 AND pg_catalog.count(DISTINCT a.grantee)=2
       AND pg_catalog.bool_and(a.privilege_type='EXECUTE' AND NOT a.is_grantable
         AND a.grantor=p.proowner AND a.grantee IN (p.proowner,'learning_runtime'::pg_catalog.regrole))
      FROM pg_catalog.aclexplode(p.proacl) a)
),false)
FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
JOIN pg_catalog.pg_language l ON l.oid=p.prolang
WHERE p.proname='p0c4_snapshot_publication_observe_v1'
"#;

fn section<'a>(source: &'a str, begin: &str, end: &str) -> Result<&'a str, ContentError> {
    let (prefix, tail) = source.split_once(begin).ok_or(ContentError::Storage)?;
    let (body, suffix) = tail.split_once(end).ok_or(ContentError::Storage)?;
    if prefix.contains(end)
        || body.trim().is_empty()
        || body.contains(begin)
        || suffix.contains(begin)
        || suffix.contains(end)
    {
        return Err(ContentError::Storage);
    }
    Ok(body)
}

fn function_body(source: &str) -> Result<&str, ContentError> {
    const BEGIN: &str = "AS $snapshot_publication_observation_v1$";
    const END: &str = "$snapshot_publication_observation_v1$;";
    let body = section(source, BEGIN, END)?;
    let (head, tail) = source.split_once(BEGIN).ok_or(ContentError::Storage)?;
    let (_, suffix) = tail.split_once(END).ok_or(ContentError::Storage)?;
    if !head.starts_with("CREATE FUNCTION public.p0c4_snapshot_publication_observe_v1(")
        || head.contains(';')
        || body.contains("$snapshot_publication_observation_v1$")
        || !suffix.trim().is_empty()
    {
        return Err(ContentError::Storage);
    }
    Ok(body)
}

async fn schema(tx: &mut Transaction<'_, Postgres>) -> Result<(), ContentError> {
    let sql = section(
        SOURCE,
        "-- D1_SCHEMA_QUERY_BEGIN\n",
        "-- D1_SCHEMA_QUERY_END\n",
    )?;
    let valid: bool = sqlx::query_scalar(sql)
        .fetch_one(&mut **tx)
        .await
        .map_err(crate::storage)?;
    if valid {
        Ok(())
    } else {
        Err(ContentError::Storage)
    }
}

/// Only catalog reads, on the caller's original transaction. No repair or commit.
pub(super) async fn validate(tx: &mut Transaction<'_, Postgres>) -> Result<(), ContentError> {
    schema(tx).await?;
    let valid: bool = sqlx::query_scalar(PROFILE)
        .bind(SIGNATURE)
        .bind(function_body(SOURCE)?)
        .fetch_one(&mut **tx)
        .await
        .map_err(crate::storage)?;
    if valid {
        Ok(())
    } else {
        Err(ContentError::Storage)
    }
}

async fn install_owned(tx: &mut Transaction<'_, Postgres>) -> Result<(), ContentError> {
    let owner: bool = sqlx::query_scalar(OWNER)
        .fetch_one(&mut **tx)
        .await
        .map_err(crate::storage)?;
    if !owner {
        return Err(ContentError::Storage);
    }
    schema(tx).await?;
    // An unexpected overload anywhere is drift, never an absent canonical slot.
    let named: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_proc WHERE proname='p0c4_snapshot_publication_observe_v1'",
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(crate::storage)?;
    if named == 0 {
        function_body(SOURCE)?;
        sqlx::raw_sql(SOURCE)
            .execute(&mut **tx)
            .await
            .map_err(crate::storage)?;
        for sql in [
            "ALTER FUNCTION public.p0c4_snapshot_publication_observe_v1(uuid,uuid,uuid,timestamptz,text,boolean) OWNER TO learning_admin",
            "REVOKE ALL ON FUNCTION public.p0c4_snapshot_publication_observe_v1(uuid,uuid,uuid,timestamptz,text,boolean) FROM PUBLIC,learning_runtime,learning_snapshot_claim_broker,learning_auth_lock",
            "GRANT EXECUTE ON FUNCTION public.p0c4_snapshot_publication_observe_v1(uuid,uuid,uuid,timestamptz,text,boolean) TO learning_runtime",
        ] {
            sqlx::query(sql)
                .execute(&mut **tx)
                .await
                .map_err(crate::storage)?;
        }
    } else if named != 1 {
        return Err(ContentError::Storage);
    }
    validate(tx).await
}

/// Own explicit transaction; every rejected path waits for rollback, without retry.
pub(in crate::snapshot_broker) async fn install(pool: &PgPool) -> Result<(), ContentError> {
    let mut tx = pool.begin().await.map_err(crate::storage)?;
    let outcome = async {
        for sql in [
            "SET LOCAL search_path=pg_catalog",
            "SET LOCAL lock_timeout='5s'",
            "SET LOCAL statement_timeout='15s'",
        ] {
            sqlx::query(sql)
                .execute(&mut *tx)
                .await
                .map_err(crate::storage)?;
        }
        install_owned(&mut tx).await
    }
    .await;
    match outcome {
        Ok(()) => tx.commit().await.map_err(crate::storage),
        Err(error) => {
            tx.rollback().await.map_err(crate::storage)?;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_prosrc_preserves_the_exact_body_bytes() {
        let sql = "CREATE FUNCTION public.p0c4_snapshot_publication_observe_v1() RETURNS boolean LANGUAGE plpgsql AS $snapshot_publication_observation_v1$\nBEGIN RETURN false; END\n$snapshot_publication_observation_v1$;\n";
        assert_eq!(function_body(sql).unwrap(), "\nBEGIN RETURN false; END\n");
    }

    #[test]
    fn canonical_prosrc_rejects_missing_ambiguous_or_following_sql() {
        for sql in [
            "BEGIN RETURN false; END",
            "CREATE FUNCTION public.p0c4_snapshot_publication_observe_v1() AS $snapshot_publication_observation_v1$BEGIN",
            "CREATE FUNCTION public.p0c4_snapshot_publication_observe_v1() AS $snapshot_publication_observation_v1$ $snapshot_publication_observation_v1$;",
            "CREATE FUNCTION public.p0c4_snapshot_publication_observe_v1() AS $snapshot_publication_observation_v1$BEGIN END$snapshot_publication_observation_v1$; SELECT 1;",
            "CREATE FUNCTION public.p0c4_snapshot_publication_observe_v1() AS $snapshot_publication_observation_v1$BEGIN $snapshot_publication_observation_v1$ END$snapshot_publication_observation_v1$;",
        ] {
            assert!(function_body(sql).is_err());
        }
    }
}
