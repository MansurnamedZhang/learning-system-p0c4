-- Optional A1 component; NOT part of canonical migrations 0001..0015.
-- Invoke only through the owned installer transaction. No role is created or
-- repaired here. This database is not exportable until A2 schema support.
DO $snapshot_observation_install$
BEGIN
 IF current_user <> 'learning_admin' OR session_user <> 'learning_admin'
    OR current_setting('knowweave.snapshot_observation_install', true) IS DISTINCT FROM 'v1'
    OR NOT EXISTS (SELECT FROM pg_catalog.pg_roles r WHERE r.rolname=current_user
      AND NOT r.rolsuper AND NOT r.rolbypassrls AND NOT r.rolcreatedb AND NOT r.rolcreaterole)
    OR NOT EXISTS (SELECT FROM pg_catalog.pg_database d
                   WHERE d.datname=current_database() AND d.datdba=current_user::regrole)
 THEN RAISE EXCEPTION 'snapshot observation owner transaction required' USING ERRCODE='42501'; END IF;
 IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles r
   WHERE r.rolname='learning_snapshot_broker' AND r.rolcanlogin AND NOT r.rolinherit
     AND NOT r.rolsuper AND NOT r.rolcreatedb AND NOT r.rolcreaterole
     AND NOT r.rolreplication AND NOT r.rolbypassrls AND r.rolconnlimit=4
     AND NOT EXISTS (SELECT FROM pg_catalog.pg_auth_members m WHERE m.member=r.oid OR m.roleid=r.oid)
     AND NOT EXISTS (SELECT FROM pg_catalog.pg_db_role_setting s WHERE s.setrole=r.oid)
     AND NOT EXISTS (SELECT FROM pg_catalog.pg_shdepend d WHERE d.refclassid='pg_catalog.pg_authid'::regclass AND d.refobjid=r.oid AND d.deptype='o'))
 THEN RAISE EXCEPTION 'snapshot observation broker role mismatch' USING ERRCODE='42501'; END IF;
 IF EXISTS (SELECT FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
            WHERE n.nspname='public' AND p.proname='p0c4_snapshot_locked_observation')
 THEN RAISE EXCEPTION 'snapshot observation extension already present' USING ERRCODE='55000'; END IF;
 IF pg_catalog.has_database_privilege('learning_snapshot_broker',current_database(),'TEMP')
    OR pg_catalog.has_database_privilege('learning_snapshot_broker',current_database(),'CREATE')
    OR pg_catalog.has_schema_privilege('learning_snapshot_broker','public','CREATE')
    OR EXISTS (SELECT FROM pg_catalog.pg_database d,
      LATERAL pg_catalog.aclexplode(COALESCE(d.datacl,pg_catalog.acldefault('d',d.datdba))) a
      WHERE d.datname=current_database() AND a.grantee=0 AND a.privilege_type IN ('CONNECT','TEMPORARY'))
    OR EXISTS (SELECT FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
      WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_' AND c.relkind IN ('r','p','v','m','f')
        AND CASE WHEN c.relkind IN ('r','p','v','m','f') THEN
          (pg_catalog.has_table_privilege('learning_snapshot_broker',c.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER')
          OR pg_catalog.has_any_column_privilege('learning_snapshot_broker',c.oid,'SELECT,INSERT,UPDATE,REFERENCES'))
          ELSE false END)
    OR EXISTS (SELECT FROM pg_catalog.pg_namespace n WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_'
      AND (pg_catalog.has_schema_privilege('learning_snapshot_broker',n.oid,'CREATE')
        OR (n.nspname<>'public' AND pg_catalog.has_schema_privilege('learning_snapshot_broker',n.oid,'USAGE'))))
    OR EXISTS (SELECT FROM pg_catalog.pg_class c WHERE CASE WHEN c.relkind='S' THEN
      pg_catalog.has_sequence_privilege('learning_snapshot_broker',c.oid,'USAGE,SELECT,UPDATE')
      ELSE false END)
    OR EXISTS (SELECT FROM pg_catalog.pg_default_acl d, LATERAL pg_catalog.aclexplode(d.defaclacl) a
      WHERE a.grantee='learning_snapshot_broker'::regrole)
    OR EXISTS (SELECT FROM pg_catalog.pg_proc p,
      LATERAL pg_catalog.aclexplode(p.proacl) a
      WHERE a.grantee='learning_snapshot_broker'::regrole)
    OR EXISTS (SELECT FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
      WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_' AND p.prosecdef
        AND pg_catalog.has_function_privilege('learning_snapshot_broker',p.oid,'EXECUTE'))
 THEN RAISE EXCEPTION 'snapshot observation unexpected broker privileges' USING ERRCODE='42501'; END IF;
END
$snapshot_observation_install$;

-- INSTALL_HELPER
CREATE FUNCTION public.p0c4_snapshot_locked_observation(p_job_id uuid,p_include_body boolean)
RETURNS TABLE(job_octets bigint,outbox_octets bigint,observed_at timestamptz,job_row jsonb,outbox_row jsonb)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path=pg_catalog SET row_security=on SET TimeZone='UTC'
AS $snapshot_observation_v1$
DECLARE j public.job%ROWTYPE; e public.job_outbox%ROWTYPE; jb jsonb; eb jsonb;
BEGIN
 IF session_user <> 'learning_snapshot_broker' THEN
  RAISE EXCEPTION 'snapshot observation broker login required' USING ERRCODE='42501';
 END IF;
 IF p_job_id IS NULL OR p_job_id='00000000-0000-0000-0000-000000000000'::uuid OR p_include_body IS NULL THEN
  RAISE EXCEPTION 'invalid snapshot observation request' USING ERRCODE='22023';
 END IF;
 IF (SELECT array_agg(a.attname::text ORDER BY a.attnum) FROM pg_catalog.pg_attribute a
     WHERE a.attrelid='public.job'::regclass AND a.attnum>0 AND NOT a.attisdropped)
    IS DISTINCT FROM ARRAY['id','outbox_id','idempotency_key','status','attempt_count','lease_token','lease_expires_at','next_attempt_at','last_error_class','output_digest','checkpoint','created_at','updated_at']::text[]
 OR (SELECT array_agg(a.attname::text ORDER BY a.attnum) FROM pg_catalog.pg_attribute a
     WHERE a.attrelid='public.job_outbox'::regclass AND a.attnum>0 AND NOT a.attisdropped)
    IS DISTINCT FROM ARRAY['id','business_key','event_type','payload_version','payload','actor_id','processor_version','created_at','dispatched_at']::text[]
 THEN RAISE EXCEPTION 'snapshot observation canonical columns mismatch' USING ERRCODE='23514'; END IF;
 SELECT x.* INTO j FROM public.job x JOIN public.job_outbox o ON o.id=x.outbox_id
  WHERE x.id=p_job_id AND o.event_type='snapshot_export_requested' FOR UPDATE OF x;
 IF NOT FOUND THEN RETURN; END IF;
 SELECT o.* INTO STRICT e FROM public.job_outbox o WHERE o.id=j.outbox_id;
 IF j.id<>j.outbox_id OR j.idempotency_key<>e.business_key OR e.payload_version<>1 OR e.processor_version<>1
    OR NOT public.p0c3_valid_job_payload(e.event_type,e.payload_version,e.payload,e.actor_id,e.business_key)
 THEN RAISE EXCEPTION 'snapshot observation binding mismatch' USING ERRCODE='23514'; END IF;
 -- Exact 13 job columns and 9 outbox columns, never to_jsonb(row).* expansion.
 jb:=jsonb_build_object('id',j.id,'outbox_id',j.outbox_id,'idempotency_key',j.idempotency_key,
  'status',j.status,'attempt_count',j.attempt_count,'lease_token',j.lease_token,
  'lease_expires_at',j.lease_expires_at,'next_attempt_at',j.next_attempt_at,
  'last_error_class',j.last_error_class,'output_digest',j.output_digest,'checkpoint',j.checkpoint,
  'created_at',j.created_at,'updated_at',j.updated_at);
 eb:=jsonb_build_object('id',e.id,'business_key',e.business_key,'event_type',e.event_type,
  'payload_version',e.payload_version,'payload',e.payload,'actor_id',e.actor_id,
  'processor_version',e.processor_version,'created_at',e.created_at,'dispatched_at',e.dispatched_at);
 job_octets:=octet_length(convert_to(jb::text,'UTF8'));
 outbox_octets:=octet_length(convert_to(eb::text,'UTF8'));
 IF job_octets>1048576 OR outbox_octets>1048576 THEN
  RAISE EXCEPTION 'snapshot observation row budget exceeded' USING ERRCODE='54000';
 END IF;
 observed_at:=clock_timestamp();
 job_row:=CASE WHEN p_include_body THEN jb ELSE NULL END;
 outbox_row:=CASE WHEN p_include_body THEN eb ELSE NULL END;
 RETURN NEXT;
END
$snapshot_observation_v1$;
-- INSTALL_ACL
REVOKE ALL ON FUNCTION public.p0c4_snapshot_locked_observation(uuid,boolean) FROM PUBLIC,learning_runtime,learning_auth_lock;
GRANT EXECUTE ON FUNCTION public.p0c4_snapshot_locked_observation(uuid,boolean) TO learning_snapshot_broker;
GRANT USAGE ON SCHEMA public TO learning_snapshot_broker;
DO $snapshot_observation_connect$
BEGIN
 EXECUTE format('GRANT CONNECT ON DATABASE %I TO learning_snapshot_broker',current_database());
END
$snapshot_observation_connect$;
COMMENT ON FUNCTION public.p0c4_snapshot_locked_observation(uuid,boolean)
 IS 'knowweave.snapshot-observation.v1:A1_NOT_EXPORTABLE_REQUIRES_A2';
