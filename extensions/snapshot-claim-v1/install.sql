-- Optional C0 profile. NOT a canonical migration, source schema or claim capability.
-- Only the owned Rust installer may run this in its explicit owner transaction.
-- No role is created/repaired. The source remains unexportable/unsupported.
-- C0_PROFILE_BEGIN
DO $snapshot_claim_profile_v1$
DECLARE
 phase text := current_setting('knowweave.snapshot_claim_profile',true);
 installed boolean;
 broker_id oid;
 owner_id oid := 'learning_admin'::regrole;
 runtime_id oid := 'learning_runtime'::regrole;
 helper_id oid;
 allowed_exec oid[];
BEGIN
 IF phase IS NULL OR phase NOT IN ('install_before','install_after','observe') THEN
  RAISE EXCEPTION 'snapshot claim internal profile phase required' USING ERRCODE='42501';
 END IF;
 installed := phase<>'install_before';
 IF phase='observe' THEN
  IF session_user<>'learning_snapshot_claim_broker' OR current_user<>session_user THEN
   RAISE EXCEPTION 'snapshot claim real broker login required' USING ERRCODE='42501';
  END IF;
 ELSE
  IF session_user<>'learning_admin' OR current_user<>session_user
    OR current_setting('knowweave.snapshot_claim_install',true) IS DISTINCT FROM 'v1'
    OR NOT EXISTS(SELECT FROM pg_catalog.pg_database d WHERE d.datname=current_database() AND d.datdba=owner_id)
  THEN RAISE EXCEPTION 'snapshot claim explicit owner installer required' USING ERRCODE='42501'; END IF;
 END IF;
 IF NOT EXISTS(SELECT FROM pg_catalog.pg_roles r WHERE r.oid=owner_id AND NOT r.rolsuper AND NOT r.rolbypassrls
    AND NOT r.rolcreatedb AND NOT r.rolcreaterole)
 THEN RAISE EXCEPTION 'snapshot claim canonical owner flags differ' USING ERRCODE='42501'; END IF;
 SELECT r.oid INTO broker_id FROM pg_catalog.pg_roles r
 WHERE r.rolname='learning_snapshot_claim_broker' AND r.rolcanlogin AND NOT r.rolinherit
  AND NOT r.rolsuper AND NOT r.rolcreatedb AND NOT r.rolcreaterole AND NOT r.rolreplication AND NOT r.rolbypassrls
  AND r.rolconnlimit=4 AND r.rolvaliduntil IS NULL
  AND NOT EXISTS(SELECT FROM pg_catalog.pg_auth_members m WHERE m.member=r.oid OR m.roleid=r.oid)
  AND NOT EXISTS(SELECT FROM pg_catalog.pg_db_role_setting s WHERE s.setrole=r.oid)
  AND NOT EXISTS(SELECT FROM pg_catalog.pg_shdepend d WHERE d.refclassid='pg_catalog.pg_authid'::regclass AND d.refobjid=r.oid AND d.deptype='o');
 IF broker_id IS NULL THEN RAISE EXCEPTION 'snapshot claim broker role differs' USING ERRCODE='42501'; END IF;

 -- Current-DB direct relation/column grants are checked for EVERY namespace
 -- and relkind, including pg_catalog/pg_toast. PUBLIC default metadata reads
 -- are not mistaken for explicit broker ACLs; no sensitive data is selected.
 IF EXISTS(SELECT FROM pg_catalog.pg_class c,LATERAL pg_catalog.aclexplode(c.relacl) a WHERE a.grantee=broker_id)
 OR EXISTS(SELECT FROM pg_catalog.pg_attribute c,LATERAL pg_catalog.aclexplode(c.attacl) a WHERE a.grantee=broker_id)
 OR EXISTS(SELECT FROM pg_catalog.pg_type c,LATERAL pg_catalog.aclexplode(c.typacl) a WHERE a.grantee=broker_id)
 OR EXISTS(SELECT FROM pg_catalog.pg_language c,LATERAL pg_catalog.aclexplode(c.lanacl) a WHERE a.grantee=broker_id)
 OR EXISTS(SELECT FROM pg_catalog.pg_largeobject_metadata c,LATERAL pg_catalog.aclexplode(c.lomacl) a WHERE a.grantee=broker_id)
 OR EXISTS(SELECT FROM pg_catalog.pg_foreign_data_wrapper c,LATERAL pg_catalog.aclexplode(c.fdwacl) a WHERE a.grantee=broker_id)
 OR EXISTS(SELECT FROM pg_catalog.pg_foreign_server c,LATERAL pg_catalog.aclexplode(c.srvacl) a WHERE a.grantee=broker_id)
 OR EXISTS(SELECT FROM pg_catalog.pg_tablespace c,LATERAL pg_catalog.aclexplode(c.spcacl) a WHERE a.grantee=broker_id)
 OR EXISTS(SELECT FROM pg_catalog.pg_parameter_acl c,LATERAL pg_catalog.aclexplode(c.paracl) a WHERE a.grantee=broker_id)
 OR EXISTS(SELECT FROM pg_catalog.pg_default_acl c,LATERAL pg_catalog.aclexplode(c.defaclacl) a WHERE a.grantee=broker_id)
 OR EXISTS(SELECT FROM pg_catalog.pg_database d,LATERAL pg_catalog.aclexplode(d.datacl) a
    WHERE a.grantee=broker_id AND (d.datname<>current_database() OR NOT installed))
 OR EXISTS(SELECT FROM pg_catalog.pg_namespace n,LATERAL pg_catalog.aclexplode(n.nspacl) a
    WHERE a.grantee=broker_id AND (n.nspname<>'public' OR NOT installed))
 THEN RAISE EXCEPTION 'snapshot claim unexpected direct privilege' USING ERRCODE='42501'; END IF;
 IF pg_catalog.has_database_privilege(broker_id,current_database(),'CREATE')
 OR pg_catalog.has_database_privilege(broker_id,current_database(),'TEMP')
 -- Deny sensitive effective reads even when inherited from PUBLIC. These
 -- fixed table/view privilege checks inspect ACL metadata, never secret data.
 OR pg_catalog.has_table_privilege(broker_id,'pg_catalog.pg_authid','SELECT')
 OR pg_catalog.has_any_column_privilege(broker_id,'pg_catalog.pg_authid','SELECT')
 OR pg_catalog.has_table_privilege(broker_id,'pg_catalog.pg_shadow','SELECT')
 OR pg_catalog.has_any_column_privilege(broker_id,'pg_catalog.pg_shadow','SELECT')
 OR pg_catalog.has_database_privilege(broker_id,current_database(),'CONNECT') IS DISTINCT FROM installed
 OR (installed AND NOT (SELECT count(*)=1 AND bool_and(a.privilege_type='CONNECT' AND NOT a.is_grantable AND a.grantor=d.datdba)
     FROM pg_catalog.pg_database d,LATERAL pg_catalog.aclexplode(d.datacl) a WHERE d.datname=current_database() AND a.grantee=broker_id))
 OR (installed AND NOT (SELECT count(*)=1 AND bool_and(a.privilege_type='USAGE' AND NOT a.is_grantable AND a.grantor=n.nspowner)
     FROM pg_catalog.pg_namespace n,LATERAL pg_catalog.aclexplode(n.nspacl) a WHERE n.nspname='public' AND a.grantee=broker_id))
 OR EXISTS(SELECT FROM pg_catalog.pg_namespace n WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_'
    AND (pg_catalog.has_schema_privilege(broker_id,n.oid,'CREATE') OR (n.nspname<>'public' AND pg_catalog.has_schema_privilege(broker_id,n.oid,'USAGE'))))
 OR EXISTS(SELECT FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
    WHERE CASE WHEN n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_'
     AND c.relkind IN ('r','p','v','m','f') THEN
       (pg_catalog.has_table_privilege(broker_id,c.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')
        OR pg_catalog.has_any_column_privilege(broker_id,c.oid,'SELECT,INSERT,UPDATE,REFERENCES')) ELSE false END)
 OR EXISTS(SELECT FROM pg_catalog.pg_class c WHERE CASE WHEN c.relkind='S'
    THEN pg_catalog.has_sequence_privilege(broker_id,c.oid,'USAGE,SELECT,UPDATE') ELSE false END)
 THEN RAISE EXCEPTION 'snapshot claim effective privilege differs' USING ERRCODE='42501'; END IF;

 IF (SELECT count(*) FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
      WHERE p.proname='p0c3_claim_snapshot_job' AND n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_')<>1
 OR NOT EXISTS(SELECT FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_language l ON l.oid=p.prolang
      WHERE p.oid='public.p0c3_claim_snapshot_job(uuid,bigint)'::regprocedure AND p.proowner=owner_id
      AND l.lanname='plpgsql' AND p.prokind='f' AND p.provolatile='v' AND p.proparallel='u'
      AND p.prosecdef AND NOT p.proisstrict AND NOT p.proleakproof AND p.pronargdefaults=0 AND p.provariadic=0
      AND p.prosupport=0 AND p.probin IS NULL AND p.prosqlbody IS NULL AND p.proargdefaults IS NULL
      AND p.protrftypes IS NULL AND p.procost=100 AND p.prorows=1000
      AND p.pronargs=2 AND p.proargtypes='2950 20'::oidvector AND p.prorettype=2249 AND p.proretset=true
      AND p.proallargtypes IS NOT DISTINCT FROM ARRAY[2950,20,2950,2950,23,1184]::oid[] AND p.proargmodes IS NOT DISTINCT FROM ARRAY['i','i','t','t','t','t']::"char"[]
      AND p.proargnames=ARRAY['p_job_id','p_lease_ms','job_id','token','attempts','expires_at']::text[] AND p.proconfig=ARRAY['search_path=pg_catalog, public, pg_temp']::text[]
      AND p.prosrc=$canonical_p0c3_claim_snapshot_job$
BEGIN
    IF p_lease_ms IS NULL OR p_lease_ms NOT BETWEEN 1 AND 300000 THEN
        RAISE EXCEPTION 'invalid lease request' USING ERRCODE = '22023';
    END IF;

    UPDATE public.job AS j
       SET status = 'failed', lease_token = NULL, lease_expires_at = NULL,
           last_error_class = 'lease_expired', output_digest = NULL
     WHERE j.id = p_job_id AND EXISTS (SELECT 1 FROM public.job_outbox e WHERE e.id=j.outbox_id AND e.event_type='snapshot_export_requested') AND j.status = 'snapshot_running'
       AND j.lease_expires_at <= clock_timestamp() AND j.attempt_count >= 3;

    RETURN QUERY
    UPDATE public.job AS j
       SET status = 'snapshot_running', attempt_count = j.attempt_count + 1,
           lease_token = pg_catalog.gen_random_uuid(),
           lease_expires_at = clock_timestamp() + p_lease_ms * interval '1 millisecond',
           next_attempt_at = NULL, last_error_class = NULL,
           output_digest = NULL
     WHERE j.id = p_job_id AND EXISTS (SELECT 1 FROM public.job_outbox e WHERE e.id=j.outbox_id AND e.event_type='snapshot_export_requested') AND j.attempt_count < 3
       AND (j.status = 'snapshot_queued'
         OR (j.status = 'snapshot_retry_wait' AND j.next_attempt_at <= clock_timestamp())
         OR (j.status = 'snapshot_running' AND j.lease_expires_at <= clock_timestamp()))
     RETURNING j.id, j.lease_token, j.attempt_count, j.lease_expires_at;
END $canonical_p0c3_claim_snapshot_job$
      AND (SELECT count(*)=CASE WHEN installed THEN 3 ELSE 2 END AND count(DISTINCT a.grantee)=CASE WHEN installed THEN 3 ELSE 2 END AND bool_and(a.privilege_type='EXECUTE' AND NOT a.is_grantable
          AND a.grantor=owner_id AND a.grantee=ANY(CASE WHEN installed THEN ARRAY[owner_id,runtime_id,broker_id] ELSE ARRAY[owner_id,runtime_id] END))
          FROM pg_catalog.aclexplode(p.proacl) a))
 THEN RAISE EXCEPTION 'snapshot claim canonical function profile differs' USING ERRCODE='42501'; END IF;
 IF (SELECT count(*) FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
      WHERE p.proname='p0c3_renew_snapshot_job' AND n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_')<>1
 OR NOT EXISTS(SELECT FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_language l ON l.oid=p.prolang
      WHERE p.oid='public.p0c3_renew_snapshot_job(uuid,uuid,bigint)'::regprocedure AND p.proowner=owner_id
      AND l.lanname='plpgsql' AND p.prokind='f' AND p.provolatile='v' AND p.proparallel='u'
      AND p.prosecdef AND NOT p.proisstrict AND NOT p.proleakproof AND p.pronargdefaults=0 AND p.provariadic=0
      AND p.prosupport=0 AND p.probin IS NULL AND p.prosqlbody IS NULL AND p.proargdefaults IS NULL
      AND p.protrftypes IS NULL AND p.procost=100 AND p.prorows=0
      AND p.pronargs=3 AND p.proargtypes='2950 2950 20'::oidvector AND p.prorettype=16 AND p.proretset=false
      AND p.proallargtypes IS NOT DISTINCT FROM NULL::oid[] AND p.proargmodes IS NOT DISTINCT FROM NULL::"char"[]
      AND p.proargnames=ARRAY['p_job_id','p_token','p_lease_ms']::text[] AND p.proconfig=ARRAY['search_path=pg_catalog, public, pg_temp']::text[]
      AND p.prosrc=$canonical_p0c3_renew_snapshot_job$
BEGIN
    IF p_lease_ms IS NULL OR p_lease_ms NOT BETWEEN 1 AND 300000 THEN
        RAISE EXCEPTION 'invalid lease duration' USING ERRCODE = '22023';
    END IF;
    UPDATE public.job AS j
       SET lease_expires_at = clock_timestamp() + p_lease_ms * interval '1 millisecond'
     WHERE j.id = p_job_id AND j.status = 'snapshot_running' AND j.lease_token = p_token
       AND j.lease_expires_at > clock_timestamp();
    RETURN FOUND;
END $canonical_p0c3_renew_snapshot_job$
      AND (SELECT count(*)=CASE WHEN installed THEN 3 ELSE 2 END AND count(DISTINCT a.grantee)=CASE WHEN installed THEN 3 ELSE 2 END AND bool_and(a.privilege_type='EXECUTE' AND NOT a.is_grantable
          AND a.grantor=owner_id AND a.grantee=ANY(CASE WHEN installed THEN ARRAY[owner_id,runtime_id,broker_id] ELSE ARRAY[owner_id,runtime_id] END))
          FROM pg_catalog.aclexplode(p.proacl) a))
 THEN RAISE EXCEPTION 'snapshot claim canonical function profile differs' USING ERRCODE='42501'; END IF;
 IF (SELECT count(*) FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
      WHERE p.proname='p0c3_fail_snapshot_job' AND n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_')<>1
 OR NOT EXISTS(SELECT FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_language l ON l.oid=p.prolang
      WHERE p.oid='public.p0c3_fail_snapshot_job(uuid,uuid,text,boolean)'::regprocedure AND p.proowner=owner_id
      AND l.lanname='plpgsql' AND p.prokind='f' AND p.provolatile='v' AND p.proparallel='u'
      AND p.prosecdef AND NOT p.proisstrict AND NOT p.proleakproof AND p.pronargdefaults=0 AND p.provariadic=0
      AND p.prosupport=0 AND p.probin IS NULL AND p.prosqlbody IS NULL AND p.proargdefaults IS NULL
      AND p.protrftypes IS NULL AND p.procost=100 AND p.prorows=0
      AND p.pronargs=4 AND p.proargtypes='2950 2950 25 16'::oidvector AND p.prorettype=16 AND p.proretset=false
      AND p.proallargtypes IS NOT DISTINCT FROM NULL::oid[] AND p.proargmodes IS NOT DISTINCT FROM NULL::"char"[]
      AND p.proargnames=ARRAY['p_job_id','p_token','p_error_class','p_retryable']::text[] AND p.proconfig=ARRAY['search_path=pg_catalog, public, pg_temp']::text[]
      AND p.prosrc=$canonical_p0c3_fail_snapshot_job$
BEGIN
    IF ((p_error_class = 'transient_storage' AND p_retryable IS TRUE)
        OR (p_error_class = 'invalid_input' AND p_retryable IS FALSE)) IS NOT TRUE THEN
        RAISE EXCEPTION 'invalid failure class' USING ERRCODE = '22023';
    END IF;
    UPDATE public.job AS j
       SET status = CASE WHEN p_retryable AND j.attempt_count < 3
                         THEN 'snapshot_retry_wait' ELSE 'failed' END,
           lease_token = NULL, lease_expires_at = NULL,
           next_attempt_at = CASE WHEN p_retryable AND j.attempt_count < 3
                THEN clock_timestamp() + CASE j.attempt_count
                    WHEN 1 THEN interval '1 second' ELSE interval '2 seconds' END
                ELSE NULL END,
           last_error_class = p_error_class, output_digest = NULL
     WHERE j.id = p_job_id AND j.status = 'snapshot_running' AND j.lease_token = p_token
       AND j.lease_expires_at > clock_timestamp();
    RETURN FOUND;
END $canonical_p0c3_fail_snapshot_job$
      AND (SELECT count(*)=CASE WHEN installed THEN 3 ELSE 2 END AND count(DISTINCT a.grantee)=CASE WHEN installed THEN 3 ELSE 2 END AND bool_and(a.privilege_type='EXECUTE' AND NOT a.is_grantable
          AND a.grantor=owner_id AND a.grantee=ANY(CASE WHEN installed THEN ARRAY[owner_id,runtime_id,broker_id] ELSE ARRAY[owner_id,runtime_id] END))
          FROM pg_catalog.aclexplode(p.proacl) a))
 THEN RAISE EXCEPTION 'snapshot claim canonical function profile differs' USING ERRCODE='42501'; END IF;

 IF NOT installed THEN
  IF EXISTS(SELECT FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
     WHERE p.proname='p0c4_snapshot_claim_locked_observation' AND n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_')
  THEN RAISE EXCEPTION 'snapshot claim helper already exists' USING ERRCODE='55000'; END IF;
  allowed_exec := ARRAY[]::oid[];
 ELSE
  IF (SELECT count(*) FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
      WHERE p.proname='p0c4_snapshot_claim_locked_observation' AND n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_')<>1
  THEN RAISE EXCEPTION 'snapshot claim helper overload differs' USING ERRCODE='42501'; END IF;
  helper_id := 'public.p0c4_snapshot_claim_locked_observation(uuid,boolean)'::regprocedure;
  IF NOT EXISTS(SELECT FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_language l ON l.oid=p.prolang
    WHERE p.oid=helper_id AND p.proowner=owner_id AND l.lanname='plpgsql' AND p.prokind='f' AND p.provolatile='v' AND p.proparallel='u'
    AND p.prosecdef AND NOT p.proisstrict AND NOT p.proleakproof AND p.pronargdefaults=0 AND p.provariadic=0 AND p.prosupport=0
    AND p.probin IS NULL AND p.prosqlbody IS NULL AND p.proargdefaults IS NULL AND p.protrftypes IS NULL AND p.procost=100 AND p.prorows=1000
    AND p.pronargs=2 AND p.proargtypes='2950 16'::oidvector
    AND p.proretset AND p.prorettype=2249 AND p.proallargtypes=ARRAY[2950,16,20,20,1184,3802,3802]::oid[]
    AND p.proargmodes=ARRAY['i','i','t','t','t','t','t']::"char"[]
    AND p.proargnames=ARRAY['p_job_id','p_include_body','job_octets','outbox_octets','observed_at','job_row','outbox_row']::text[]
    AND cardinality(p.proconfig)=3 AND p.proconfig @> ARRAY['TimeZone=UTC','row_security=on','search_path=pg_catalog']::text[]
    AND p.prosrc=$expected_snapshot_claim_observation_v1$
DECLARE j public.job%ROWTYPE; e public.job_outbox%ROWTYPE; jb jsonb; eb jsonb;
BEGIN
 IF session_user <> 'learning_snapshot_claim_broker' THEN
  RAISE EXCEPTION 'snapshot claim observation broker login required' USING ERRCODE='42501';
 END IF;
 IF p_job_id IS NULL OR p_job_id='00000000-0000-0000-0000-000000000000'::uuid OR p_include_body IS NULL THEN
  RAISE EXCEPTION 'invalid snapshot claim observation request' USING ERRCODE='22023';
 END IF;
 IF (SELECT array_agg(a.attname::text ORDER BY a.attnum) FROM pg_catalog.pg_attribute a
     WHERE a.attrelid='public.job'::regclass AND a.attnum>0 AND NOT a.attisdropped)
    IS DISTINCT FROM ARRAY['id','outbox_id','idempotency_key','status','attempt_count','lease_token','lease_expires_at','next_attempt_at','last_error_class','output_digest','checkpoint','created_at','updated_at']::text[]
 OR (SELECT array_agg(a.attname::text ORDER BY a.attnum) FROM pg_catalog.pg_attribute a
     WHERE a.attrelid='public.job_outbox'::regclass AND a.attnum>0 AND NOT a.attisdropped)
    IS DISTINCT FROM ARRAY['id','business_key','event_type','payload_version','payload','actor_id','processor_version','created_at','dispatched_at']::text[]
 THEN RAISE EXCEPTION 'snapshot claim observation canonical columns mismatch' USING ERRCODE='23514'; END IF;
 SELECT x.* INTO j FROM public.job x JOIN public.job_outbox o ON o.id=x.outbox_id
  WHERE x.id=p_job_id AND o.event_type='snapshot_export_requested' FOR UPDATE OF x;
 IF NOT FOUND THEN RETURN; END IF;
 SELECT o.* INTO STRICT e FROM public.job_outbox o WHERE o.id=j.outbox_id;
 IF j.id<>j.outbox_id OR j.idempotency_key<>e.business_key OR e.payload_version<>1 OR e.processor_version<>1
    OR NOT public.p0c3_valid_job_payload(e.event_type,e.payload_version,e.payload,e.actor_id,e.business_key)
 THEN RAISE EXCEPTION 'snapshot claim observation binding mismatch' USING ERRCODE='23514'; END IF;
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
  RAISE EXCEPTION 'snapshot claim observation row budget exceeded' USING ERRCODE='54000';
 END IF;
 observed_at:=clock_timestamp();
 job_row:=CASE WHEN p_include_body THEN jb ELSE NULL END;
 outbox_row:=CASE WHEN p_include_body THEN eb ELSE NULL END;
 RETURN NEXT;
END
$expected_snapshot_claim_observation_v1$
    AND pg_catalog.obj_description(p.oid,'pg_proc')='knowweave.snapshot-claim.v1:C0_OBSERVATION_ONLY_NOT_EXPORTABLE'
    AND (SELECT count(*)=2 AND count(DISTINCT a.grantee)=2 AND bool_and(a.privilege_type='EXECUTE' AND NOT a.is_grantable
      AND a.grantor=owner_id AND a.grantee=ANY(ARRAY[owner_id,broker_id])) FROM pg_catalog.aclexplode(p.proacl) a))
  THEN RAISE EXCEPTION 'snapshot claim helper profile differs' USING ERRCODE='42501'; END IF;
  allowed_exec := ARRAY[helper_id,'public.p0c3_claim_snapshot_job(uuid,bigint)'::regprocedure::oid,
      'public.p0c3_renew_snapshot_job(uuid,uuid,bigint)'::regprocedure::oid,'public.p0c3_fail_snapshot_job(uuid,uuid,text,boolean)'::regprocedure::oid];
 END IF;
 IF EXISTS(SELECT FROM pg_catalog.pg_proc p,LATERAL pg_catalog.aclexplode(p.proacl) a
    WHERE a.grantee=broker_id AND NOT (p.oid=ANY(allowed_exec)))
 OR EXISTS(SELECT FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
    WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_' AND p.prosecdef
    AND NOT (p.oid=ANY(allowed_exec)) AND pg_catalog.has_function_privilege(broker_id,p.oid,'EXECUTE'))
 THEN RAISE EXCEPTION 'snapshot claim unexpected function authority' USING ERRCODE='42501'; END IF;
END
$snapshot_claim_profile_v1$;
-- C0_PROFILE_END
-- C0_DDL_BEGIN
CREATE FUNCTION public.p0c4_snapshot_claim_locked_observation(p_job_id uuid,p_include_body boolean)
RETURNS TABLE(job_octets bigint,outbox_octets bigint,observed_at timestamptz,job_row jsonb,outbox_row jsonb)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path=pg_catalog SET row_security=on SET TimeZone='UTC'
AS $snapshot_claim_observation_v1$
DECLARE j public.job%ROWTYPE; e public.job_outbox%ROWTYPE; jb jsonb; eb jsonb;
BEGIN
 IF session_user <> 'learning_snapshot_claim_broker' THEN
  RAISE EXCEPTION 'snapshot claim observation broker login required' USING ERRCODE='42501';
 END IF;
 IF p_job_id IS NULL OR p_job_id='00000000-0000-0000-0000-000000000000'::uuid OR p_include_body IS NULL THEN
  RAISE EXCEPTION 'invalid snapshot claim observation request' USING ERRCODE='22023';
 END IF;
 IF (SELECT array_agg(a.attname::text ORDER BY a.attnum) FROM pg_catalog.pg_attribute a
     WHERE a.attrelid='public.job'::regclass AND a.attnum>0 AND NOT a.attisdropped)
    IS DISTINCT FROM ARRAY['id','outbox_id','idempotency_key','status','attempt_count','lease_token','lease_expires_at','next_attempt_at','last_error_class','output_digest','checkpoint','created_at','updated_at']::text[]
 OR (SELECT array_agg(a.attname::text ORDER BY a.attnum) FROM pg_catalog.pg_attribute a
     WHERE a.attrelid='public.job_outbox'::regclass AND a.attnum>0 AND NOT a.attisdropped)
    IS DISTINCT FROM ARRAY['id','business_key','event_type','payload_version','payload','actor_id','processor_version','created_at','dispatched_at']::text[]
 THEN RAISE EXCEPTION 'snapshot claim observation canonical columns mismatch' USING ERRCODE='23514'; END IF;
 SELECT x.* INTO j FROM public.job x JOIN public.job_outbox o ON o.id=x.outbox_id
  WHERE x.id=p_job_id AND o.event_type='snapshot_export_requested' FOR UPDATE OF x;
 IF NOT FOUND THEN RETURN; END IF;
 SELECT o.* INTO STRICT e FROM public.job_outbox o WHERE o.id=j.outbox_id;
 IF j.id<>j.outbox_id OR j.idempotency_key<>e.business_key OR e.payload_version<>1 OR e.processor_version<>1
    OR NOT public.p0c3_valid_job_payload(e.event_type,e.payload_version,e.payload,e.actor_id,e.business_key)
 THEN RAISE EXCEPTION 'snapshot claim observation binding mismatch' USING ERRCODE='23514'; END IF;
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
  RAISE EXCEPTION 'snapshot claim observation row budget exceeded' USING ERRCODE='54000';
 END IF;
 observed_at:=clock_timestamp();
 job_row:=CASE WHEN p_include_body THEN jb ELSE NULL END;
 outbox_row:=CASE WHEN p_include_body THEN eb ELSE NULL END;
 RETURN NEXT;
END
$snapshot_claim_observation_v1$;
REVOKE ALL ON FUNCTION public.p0c4_snapshot_claim_locked_observation(uuid,boolean) FROM PUBLIC,learning_runtime,learning_auth_lock;
GRANT EXECUTE ON FUNCTION public.p0c4_snapshot_claim_locked_observation(uuid,boolean) TO learning_snapshot_claim_broker;
GRANT EXECUTE ON FUNCTION public.p0c3_claim_snapshot_job(uuid,bigint),public.p0c3_renew_snapshot_job(uuid,uuid,bigint),
 public.p0c3_fail_snapshot_job(uuid,uuid,text,boolean) TO learning_snapshot_claim_broker;
GRANT USAGE ON SCHEMA public TO learning_snapshot_claim_broker;
DO $snapshot_claim_connect_v1$
BEGIN EXECUTE format('GRANT CONNECT ON DATABASE %I TO learning_snapshot_claim_broker',current_database()); END
$snapshot_claim_connect_v1$;
COMMENT ON FUNCTION public.p0c4_snapshot_claim_locked_observation(uuid,boolean)
 IS 'knowweave.snapshot-claim.v1:C0_OBSERVATION_ONLY_NOT_EXPORTABLE';
-- C0_DDL_END
