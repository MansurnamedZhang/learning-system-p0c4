//! Optional A1 observation component. It cannot claim, export or mint write
//! authority. Canonical migrations and the default worker never install it.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction, types::Json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::{Instant, timeout_at};
use uuid::Uuid;

pub const BROKER_ROLE: &str = "learning_snapshot_broker";
pub const INSTALL_SQL: &str =
    include_str!("../../../extensions/snapshot-observation-v1/install.sql");
const COMMENT: &str = "knowweave.snapshot-observation.v1:A1_NOT_EXPORTABLE_REQUIRES_A2";
const ROW_CAP: u64 = 1_048_576;
const HEADER_CHARGE: u64 = 512;
const MAX_BUDGET: u64 = 67_108_864;

#[derive(Debug)]
pub enum ObservationError {
    Invalid(&'static str),
    Capacity,
    Timeout,
    Database(sqlx::Error),
}
impl std::fmt::Display for ObservationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "{reason}"),
            Self::Capacity => f.write_str("snapshot observation budget exceeded"),
            Self::Timeout => f.write_str("snapshot observation deadline exceeded"),
            Self::Database(_) => f.write_str("snapshot observation database operation failed"),
        }
    }
}
impl std::error::Error for ObservationError {}
impl From<sqlx::Error> for ObservationError {
    fn from(e: sqlx::Error) -> Self {
        Self::Database(e)
    }
}

/// One caller-owned meter shared by all observations; clones never reset it.
#[derive(Clone, Debug)]
pub struct ObservationBudget(Arc<Mutex<(u64, u64)>>);
impl ObservationBudget {
    pub fn new(limit: u64) -> Result<Self, ObservationError> {
        if limit > MAX_BUDGET {
            return Err(ObservationError::Capacity);
        }
        Ok(Self(Arc::new(Mutex::new((0, limit)))))
    }
    pub fn charged_bytes(&self) -> u64 {
        self.0.lock().expect("observation budget").0
    }
    fn charge(&self, bytes: u64) -> Result<(), ObservationError> {
        let mut meter = self
            .0
            .lock()
            .map_err(|_| ObservationError::Invalid("observation budget poisoned"))?;
        let next = meter
            .0
            .checked_add(bytes)
            .ok_or(ObservationError::Capacity)?;
        if next > meter.1 {
            return Err(ObservationError::Capacity);
        }
        meter.0 = next;
        Ok(())
    }
}

fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct JobObservation {
    pub id: Uuid,
    pub outbox_id: Uuid,
    pub idempotency_key: String,
    pub status: String,
    pub attempt_count: i32,
    #[serde(deserialize_with = "required_option")]
    pub lease_token: Option<Uuid>,
    #[serde(deserialize_with = "required_option")]
    pub lease_expires_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "required_option")]
    pub next_attempt_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "required_option")]
    pub last_error_class: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub output_digest: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub checkpoint: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OutboxObservation {
    pub id: Uuid,
    pub business_key: String,
    pub event_type: String,
    pub payload_version: i32,
    pub payload: serde_json::Value,
    pub actor_id: Uuid,
    pub processor_version: i32,
    pub created_at: DateTime<Utc>,
    #[serde(deserialize_with = "required_option")]
    pub dispatched_at: Option<DateTime<Utc>>,
}
#[derive(Debug)]
pub struct SnapshotObservation {
    pub job: JobObservation,
    pub outbox: OutboxObservation,
    /// Database observation metadata, not a fabricated tenth outbox column.
    pub observed_at: DateTime<Utc>,
}
#[derive(sqlx::FromRow)]
struct RawObservation {
    job_octets: i64,
    outbox_octets: i64,
    observed_at: DateTime<Utc>,
    job_row: Option<Json<JobObservation>>,
    outbox_row: Option<Json<OutboxObservation>>,
}

const ROLE_VALID: &str = "SELECT EXISTS(SELECT FROM pg_catalog.pg_roles r WHERE r.rolname='learning_snapshot_broker' AND r.rolcanlogin AND NOT r.rolinherit AND NOT r.rolsuper AND NOT r.rolcreatedb AND NOT r.rolcreaterole AND NOT r.rolreplication AND NOT r.rolbypassrls AND r.rolconnlimit=4 AND NOT EXISTS(SELECT FROM pg_catalog.pg_auth_members m WHERE m.member=r.oid OR m.roleid=r.oid) AND NOT EXISTS(SELECT FROM pg_catalog.pg_db_role_setting s WHERE s.setrole=r.oid) AND NOT EXISTS(SELECT FROM pg_catalog.pg_shdepend d WHERE d.refclassid='pg_catalog.pg_authid'::regclass AND d.refobjid=r.oid AND d.deptype='o')) AND pg_catalog.has_database_privilege('learning_snapshot_broker',current_database(),'CONNECT') AND NOT pg_catalog.has_database_privilege('learning_snapshot_broker',current_database(),'CREATE') AND NOT pg_catalog.has_database_privilege('learning_snapshot_broker',current_database(),'TEMP') AND NOT pg_catalog.has_schema_privilege('learning_snapshot_broker','public','CREATE') AND NOT EXISTS(SELECT FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_' AND c.relkind IN ('r','p','v','m','f') AND CASE WHEN c.relkind IN ('r','p','v','m','f') THEN (pg_catalog.has_table_privilege('learning_snapshot_broker',c.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER') OR pg_catalog.has_any_column_privilege('learning_snapshot_broker',c.oid,'SELECT,INSERT,UPDATE,REFERENCES')) ELSE false END)";
const CATALOG_VALID: &str = "SELECT count(*)=1 AND COALESCE(bool_and(p.proowner='learning_admin'::regrole AND p.prosecdef AND NOT p.proisstrict AND NOT p.proleakproof AND p.provolatile='v' AND p.proparallel='u' AND p.prokind='f' AND p.pronargdefaults=0 AND p.provariadic=0 AND p.proretset AND p.prorettype=2249 AND p.prolang=(SELECT oid FROM pg_catalog.pg_language WHERE lanname='plpgsql') AND p.pronargs=2 AND p.proargtypes='2950 16'::oidvector AND p.proallargtypes=ARRAY[2950,16,20,20,1184,3802,3802]::oid[] AND p.proargmodes::text[]=ARRAY['i','i','t','t','t','t','t']::text[] AND p.proargnames=ARRAY['p_job_id','p_include_body','job_octets','outbox_octets','observed_at','job_row','outbox_row']::text[] AND cardinality(p.proconfig)=3 AND p.proconfig @> ARRAY['TimeZone=UTC','row_security=on','search_path=pg_catalog']::text[] AND p.prosrc=$1 AND pg_catalog.obj_description(p.oid,'pg_proc')=$2 AND (SELECT count(*)=2 AND bool_and(a.privilege_type='EXECUTE' AND a.grantor=p.proowner AND NOT a.is_grantable AND a.grantee IN ('learning_admin'::regrole,'learning_snapshot_broker'::regrole)) FROM pg_catalog.aclexplode(p.proacl) a)),false) FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public' AND p.proname='p0c4_snapshot_locked_observation'";

fn body() -> &'static str {
    INSTALL_SQL
        .split("$snapshot_observation_v1$")
        .nth(1)
        .expect("fixed helper body")
}
async fn verify(tx: &mut Transaction<'_, Postgres>) -> Result<(), ObservationError> {
    let role: bool = sqlx::query_scalar(ROLE_VALID).fetch_one(&mut **tx).await?;
    let catalog: bool = sqlx::query_scalar(CATALOG_VALID)
        .bind(body())
        .bind(COMMENT)
        .fetch_one(&mut **tx)
        .await?;
    let only_helper:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_' AND p.prosecdef AND p.proname<>'p0c4_snapshot_locked_observation' AND pg_catalog.has_function_privilege('learning_snapshot_broker',p.oid,'EXECUTE'))").fetch_one(&mut **tx).await?;
    let no_extra_direct_execute:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT FROM pg_catalog.pg_proc p, LATERAL pg_catalog.aclexplode(p.proacl) a WHERE a.grantee='learning_snapshot_broker'::regrole AND p.oid<>'public.p0c4_snapshot_locked_observation(uuid,boolean)'::regprocedure)").fetch_one(&mut **tx).await?;
    let no_extra_scope:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT FROM pg_catalog.pg_namespace n WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_' AND (pg_catalog.has_schema_privilege('learning_snapshot_broker',n.oid,'CREATE') OR (n.nspname<>'public' AND pg_catalog.has_schema_privilege('learning_snapshot_broker',n.oid,'USAGE')))) AND NOT EXISTS(SELECT FROM pg_catalog.pg_class c WHERE CASE WHEN c.relkind='S' THEN pg_catalog.has_sequence_privilege('learning_snapshot_broker',c.oid,'USAGE,SELECT,UPDATE') ELSE false END) AND NOT EXISTS(SELECT FROM pg_catalog.pg_default_acl d, LATERAL pg_catalog.aclexplode(d.defaclacl) a WHERE a.grantee='learning_snapshot_broker'::regrole)").fetch_one(&mut **tx).await?;
    let exact_scope_acl:bool=sqlx::query_scalar("SELECT (SELECT count(*)=1 AND bool_and(a.privilege_type='CONNECT' AND NOT a.is_grantable AND a.grantor=d.datdba) FROM pg_catalog.pg_database d,LATERAL pg_catalog.aclexplode(d.datacl) a WHERE d.datname=current_database() AND a.grantee='learning_snapshot_broker'::regrole) AND (SELECT count(*)=1 AND bool_and(a.privilege_type='USAGE' AND NOT a.is_grantable AND a.grantor=n.nspowner) FROM pg_catalog.pg_namespace n,LATERAL pg_catalog.aclexplode(n.nspacl) a WHERE n.nspname='public' AND a.grantee='learning_snapshot_broker'::regrole)").fetch_one(&mut **tx).await?;
    if !role
        || !catalog
        || !only_helper
        || !no_extra_direct_execute
        || !no_extra_scope
        || !exact_scope_acl
    {
        return Err(ObservationError::Invalid(
            "snapshot observation role or extension mismatch",
        ));
    }
    Ok(())
}

/// Explicit DB-owner installer. Cluster role provisioning is a separate trusted
/// operation; this function never creates, repairs or changes a role.
pub async fn install(pool: &PgPool) -> Result<(), ObservationError> {
    timeout_at(Instant::now() + Duration::from_secs(30), async {
        let mut tx = pool.begin().await?;
        for sql in [
            "SET LOCAL lock_timeout='5s'",
            "SET LOCAL statement_timeout='15s'",
            "SET LOCAL knowweave.snapshot_observation_install='v1'",
        ] {
            sqlx::query(sql).execute(&mut *tx).await?;
        }
        sqlx::raw_sql(INSTALL_SQL).execute(&mut *tx).await?;
        verify(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    })
    .await
    .map_err(|_| ObservationError::Timeout)?
}

/// Owned READ COMMITTED transaction. No executor/connection/commit/claim is
/// exposed. Its only ending operation is rollback; Drop also queues rollback.
/// A1 observations carry no exportability, claim cap or write authority.
pub struct ObservationTransaction {
    tx: Transaction<'static, Postgres>,
    budget: ObservationBudget,
    deadline: Instant,
}
impl ObservationTransaction {
    pub async fn begin(
        pool: &PgPool,
        budget: ObservationBudget,
        duration: Duration,
    ) -> Result<Self, ObservationError> {
        if duration.is_zero() || duration > Duration::from_secs(60) {
            return Err(ObservationError::Invalid("snapshot observation duration"));
        }
        let deadline = Instant::now() + duration;
        budget.charge(HEADER_CHARGE)?;
        timeout_at(deadline, async {
            let mut tx = pool.begin().await?;
            for sql in [
                "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
                "SET LOCAL lock_timeout='5s'",
                "SET LOCAL statement_timeout='15s'",
                "SET LOCAL TimeZone='UTC'",
            ] {
                sqlx::query(sql).execute(&mut *tx).await?;
            }
            let identity: (String, String) =
                sqlx::query_as("SELECT session_user::text,current_user::text")
                    .fetch_one(&mut *tx)
                    .await?;
            if identity != (BROKER_ROLE.into(), BROKER_ROLE.into()) {
                return Err(ObservationError::Invalid(
                    "snapshot observation requires real broker login",
                ));
            }
            verify(&mut tx).await?;
            Ok(Self {
                tx,
                budget,
                deadline,
            })
        })
        .await
        .map_err(|_| ObservationError::Timeout)?
    }
    pub async fn observe(
        &mut self,
        id: Uuid,
    ) -> Result<Option<SnapshotObservation>, ObservationError> {
        if id.is_nil() {
            return Err(ObservationError::Invalid("nil snapshot observation job"));
        }
        timeout_at(self.deadline, async {
            self.budget.charge(HEADER_CHARGE)?;
            verify(&mut self.tx).await?;
            let header: Option<RawObservation> =
                sqlx::query_as("SELECT * FROM public.p0c4_snapshot_locked_observation($1,false)")
                    .bind(id)
                    .fetch_optional(&mut *self.tx)
                    .await?;
            let Some(header) = header else {
                return Ok(None);
            };
            if header.job_row.is_some() || header.outbox_row.is_some() {
                return Err(ObservationError::Invalid(
                    "snapshot observation header contains body",
                ));
            }
            let (jb, eb) = (
                u64::try_from(header.job_octets).map_err(|_| ObservationError::Capacity)?,
                u64::try_from(header.outbox_octets).map_err(|_| ObservationError::Capacity)?,
            );
            if jb == 0 || eb == 0 || jb > ROW_CAP || eb > ROW_CAP {
                return Err(ObservationError::Capacity);
            }
            self.budget.charge(
                jb.checked_add(eb)
                    .and_then(|n| n.checked_add(HEADER_CHARGE))
                    .ok_or(ObservationError::Capacity)?,
            )?;
            let row: RawObservation =
                sqlx::query_as("SELECT * FROM public.p0c4_snapshot_locked_observation($1,true)")
                    .bind(id)
                    .fetch_one(&mut *self.tx)
                    .await?;
            if row.job_octets != header.job_octets || row.outbox_octets != header.outbox_octets {
                return Err(ObservationError::Invalid(
                    "snapshot observation size changed under lock",
                ));
            }
            let (Some(Json(job)), Some(Json(outbox))) = (row.job_row, row.outbox_row) else {
                return Err(ObservationError::Invalid(
                    "snapshot observation missing body",
                ));
            };
            if job.id != id
                || job.id != job.outbox_id
                || job.outbox_id != outbox.id
                || job.idempotency_key != outbox.business_key
                || outbox.event_type != "snapshot_export_requested"
                || outbox.payload_version != 1
                || outbox.processor_version != 1
            {
                return Err(ObservationError::Invalid(
                    "snapshot observation identity mismatch",
                ));
            }
            verify(&mut self.tx).await?;
            Ok(Some(SnapshotObservation {
                job,
                outbox,
                observed_at: row.observed_at,
            }))
        })
        .await
        .map_err(|_| ObservationError::Timeout)?
    }
    pub async fn rollback(self) -> Result<(), ObservationError> {
        timeout_at(self.deadline, self.tx.rollback())
            .await
            .map_err(|_| ObservationError::Timeout)??;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_observation_budget_never_resets_on_clone() {
        let b = ObservationBudget::new(100).unwrap();
        let next = b.clone();
        b.charge(60).unwrap();
        assert!(matches!(next.charge(41), Err(ObservationError::Capacity)));
        assert_eq!(next.charged_bytes(), 60);
        next.charge(40).unwrap();
        assert_eq!(b.charged_bytes(), 100);
    }
    #[test]
    fn strict_rows_reject_extra_columns_and_installer_is_separate_from_migrations() {
        let value = serde_json::json!({"id":Uuid::nil(),"outbox_id":Uuid::nil(),"idempotency_key":"fixture","status":"snapshot_queued","attempt_count":0,"lease_token":null,"lease_expires_at":null,"next_attempt_at":null,"last_error_class":null,"output_digest":null,"checkpoint":null,"created_at":"2026-10-20T00:00:00Z","updated_at":"2026-10-20T00:00:00Z"});
        assert!(serde_json::from_value::<JobObservation>(value.clone()).is_ok());
        let mut extra = value.clone();
        extra["extra"] = serde_json::json!(1);
        assert!(serde_json::from_value::<JobObservation>(extra).is_err());
        let mut missing = value;
        missing.as_object_mut().unwrap().remove("lease_token");
        assert!(
            serde_json::from_value::<JobObservation>(missing).is_err(),
            "nullable does not mean an optional schema key"
        );
        let outbox = serde_json::json!({"id":Uuid::nil(),"business_key":"fixture","event_type":"snapshot_export_requested","payload_version":1,"payload":{},"actor_id":Uuid::nil(),"processor_version":1,"created_at":"2026-10-20T00:00:00Z","dispatched_at":null});
        assert!(serde_json::from_value::<OutboxObservation>(outbox.clone()).is_ok());
        let mut missing_outbox = outbox;
        missing_outbox
            .as_object_mut()
            .unwrap()
            .remove("dispatched_at");
        assert!(serde_json::from_value::<OutboxObservation>(missing_outbox).is_err());
        assert!(INSTALL_SQL.contains("A1_NOT_EXPORTABLE_REQUIRES_A2"));
        assert!(!INSTALL_SQL.contains("CREATE ROLE"));
        assert_eq!(INSTALL_SQL.matches("CREATE FUNCTION ").count(), 1);
    }
}
