CREATE FUNCTION public.p0c4_snapshot_publication_observe_v1(
 p_job_id uuid,p_token uuid,p_request uuid,p_expiry timestamptz,p_phase text,p_include_body boolean
)
RETURNS TABLE(job_octets bigint,outbox_octets bigint,request_octets bigint,key_octets bigint,
 receipt_octets bigint,observed_at timestamptz,job_row jsonb,outbox_row jsonb,
 request_row jsonb,key_row jsonb,receipt_row jsonb)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path=pg_catalog SET row_security=on SET TimeZone='UTC'
AS $snapshot_publication_observation_v1$
DECLARE
 j public.job%ROWTYPE; e public.job_outbox%ROWTYPE;
 r public.snapshot_export_request%ROWTYPE; k public.request_key%ROWTYPE;
 s public.snapshot_export_result%ROWTYPE;
 jb jsonb; eb jsonb; rb jsonb; kb jsonb; sb jsonb;
BEGIN
 IF current_user<>'learning_admin' OR session_user<>'learning_runtime' THEN
  RAISE EXCEPTION 'publication observation real runtime login required' USING ERRCODE='42501';
 END IF;
 IF p_job_id IS NULL OR p_token IS NULL OR p_request IS NULL OR p_expiry IS NULL
    OR p_phase IS NULL OR p_include_body IS NULL
    OR p_job_id='00000000-0000-0000-0000-000000000000'::pg_catalog.uuid
    OR p_token='00000000-0000-0000-0000-000000000000'::pg_catalog.uuid
    OR p_request='00000000-0000-0000-0000-000000000000'::pg_catalog.uuid
    OR NOT pg_catalog.isfinite(p_expiry) OR p_phase NOT IN ('before','after') THEN
  RAISE EXCEPTION 'invalid publication observation request' USING ERRCODE='22023';
 END IF;
 IF (
-- D1_SCHEMA_QUERY_BEGIN
WITH expected(relname,column_names,column_types,not_nulls) AS (VALUES
 ('job',ARRAY['id','outbox_id','idempotency_key','status','attempt_count','lease_token','lease_expires_at','next_attempt_at','last_error_class','output_digest','checkpoint','created_at','updated_at']::pg_catalog.text[],ARRAY[2950,2950,25,25,23,2950,1184,1184,25,25,3802,1184,1184]::pg_catalog.oid[],ARRAY[true,true,true,true,true,false,false,false,false,false,false,true,true]::pg_catalog.bool[]),
 ('job_outbox',ARRAY['id','business_key','event_type','payload_version','payload','actor_id','processor_version','created_at','dispatched_at']::pg_catalog.text[],ARRAY[2950,25,25,23,3802,2950,23,1184,1184]::pg_catalog.oid[],ARRAY[true,true,true,true,true,true,true,true,false]::pg_catalog.bool[]),
 ('snapshot_export_request',ARRAY['actor_id','request_id','outbox_id','job_id']::pg_catalog.text[],ARRAY[2950,2950,2950,2950]::pg_catalog.oid[],ARRAY[true,true,true,true]::pg_catalog.bool[]),
 ('request_key',ARRAY['actor_id','request_id','request_sha256','operation']::pg_catalog.text[],ARRAY[2950,2950,25,25]::pg_catalog.oid[],ARRAY[true,true,true,true]::pg_catalog.bool[]),
 ('snapshot_export_result',ARRAY['job_id','stage_token','capability','manifest_sha256','plan_sha256','required_spaces','created_at']::pg_catalog.text[],ARRAY[2950,2950,25,25,25,2951,1184]::pg_catalog.oid[],ARRAY[true,true,true,true,true,true,true]::pg_catalog.bool[])
)
SELECT pg_catalog.count(*)=5 AND COALESCE(pg_catalog.bool_and(
 c.oid IS NOT NULL AND c.relkind='r' AND c.relpersistence='p' AND NOT c.relispartition
 AND c.relowner='learning_admin'::pg_catalog.regrole AND NOT c.relrowsecurity AND NOT c.relforcerowsecurity
 AND (SELECT pg_catalog.array_agg(a.attname::pg_catalog.text ORDER BY a.attnum)
      FROM pg_catalog.pg_attribute a WHERE a.attrelid=c.oid AND a.attnum>0)=expected_schema.column_names
 AND (SELECT pg_catalog.array_agg(a.atttypid ORDER BY a.attnum)
      FROM pg_catalog.pg_attribute a WHERE a.attrelid=c.oid AND a.attnum>0)=expected_schema.column_types
 AND (SELECT pg_catalog.array_agg(a.attnotnull ORDER BY a.attnum)
      FROM pg_catalog.pg_attribute a WHERE a.attrelid=c.oid AND a.attnum>0)=expected_schema.not_nulls
 AND (SELECT pg_catalog.bool_and(NOT a.attisdropped AND a.attgenerated='' AND a.attidentity=''
       AND a.atttypmod=-1 AND a.attcollation=CASE WHEN a.atttypid=25 THEN 100 ELSE 0 END)
      FROM pg_catalog.pg_attribute a WHERE a.attrelid=c.oid AND a.attnum>0)
),false)
AND EXISTS (SELECT FROM pg_catalog.pg_roles a JOIN pg_catalog.pg_database d
 ON d.datname=pg_catalog.current_database()
 WHERE a.rolname='learning_admin' AND d.datdba=a.oid
 AND a.rolcanlogin AND a.rolinherit AND NOT a.rolsuper AND NOT a.rolcreatedb
 AND NOT a.rolcreaterole AND NOT a.rolreplication AND NOT a.rolbypassrls AND a.rolvaliduntil IS NULL)
FROM expected expected_schema LEFT JOIN pg_catalog.pg_class c
 ON c.oid=pg_catalog.to_regclass('public.'||expected_schema.relname)
-- D1_SCHEMA_QUERY_END
 ) IS NOT TRUE THEN
  RAISE EXCEPTION 'publication observation canonical row schema differs' USING ERRCODE='23514';
 END IF;
 -- Every original binding is a WHERE predicate below the row-lock operation.
 -- Wrong token/expiry/request and premature after return no row and take no job lock.
 SELECT x.* INTO j FROM public.job x
 JOIN public.job_outbox o ON o.id=x.outbox_id AND o.id=x.id AND o.business_key=x.idempotency_key
 JOIN public.snapshot_export_request q ON q.job_id=x.id AND q.outbox_id=o.id AND q.actor_id=o.actor_id
 JOIN public.request_key n ON n.actor_id=q.actor_id AND n.request_id=q.request_id
 LEFT JOIN public.snapshot_export_result t ON t.job_id=x.id
 WHERE x.id=p_job_id AND q.request_id=p_request AND n.operation='snapshot_export'
   AND o.event_type='snapshot_export_requested' AND o.payload_version=1 AND o.processor_version=1
   AND ((p_phase='before' AND x.status='snapshot_running' AND x.lease_token=p_token
         AND x.lease_expires_at=p_expiry AND x.lease_expires_at>pg_catalog.clock_timestamp()
         AND t.job_id IS NULL)
     OR (p_phase='after' AND x.status='succeeded' AND x.lease_token IS NULL
         AND x.lease_expires_at IS NULL AND t.stage_token=p_token))
 FOR UPDATE OF x;
 IF NOT FOUND THEN RETURN; END IF;
 SELECT o.* INTO STRICT e FROM public.job_outbox o WHERE o.id=j.outbox_id;
 SELECT q.* INTO STRICT r FROM public.snapshot_export_request q
  WHERE q.job_id=j.id AND q.outbox_id=e.id AND q.actor_id=e.actor_id AND q.request_id=p_request;
 SELECT n.* INTO STRICT k FROM public.request_key n
  WHERE n.actor_id=r.actor_id AND n.request_id=r.request_id AND n.operation='snapshot_export';
 SELECT t.* INTO s FROM public.snapshot_export_result t WHERE t.job_id=j.id;
 IF (p_phase='before' AND s.job_id IS NOT NULL)
    OR (p_phase='after' AND (s.job_id IS NULL OR s.stage_token IS DISTINCT FROM p_token)) THEN
  RAISE EXCEPTION 'publication observation receipt binding differs' USING ERRCODE='23514';
 END IF;
 -- The cleared expiry cannot prove history in after. The original Rust transaction
 -- retains its successful before check and lock through canonical complete/COMMIT.
 jb:=pg_catalog.jsonb_build_object('id',j.id,'outbox_id',j.outbox_id,'idempotency_key',j.idempotency_key,
  'status',j.status,'attempt_count',j.attempt_count,'lease_token',j.lease_token,
  'lease_expires_at',j.lease_expires_at,'next_attempt_at',j.next_attempt_at,
  'last_error_class',j.last_error_class,'output_digest',j.output_digest,'checkpoint',j.checkpoint,
  'created_at',j.created_at,'updated_at',j.updated_at);
 eb:=pg_catalog.jsonb_build_object('id',e.id,'business_key',e.business_key,'event_type',e.event_type,
  'payload_version',e.payload_version,'payload',e.payload,'actor_id',e.actor_id,
  'processor_version',e.processor_version,'created_at',e.created_at,'dispatched_at',e.dispatched_at);
 rb:=pg_catalog.jsonb_build_object('actor_id',r.actor_id,'request_id',r.request_id,
  'outbox_id',r.outbox_id,'job_id',r.job_id);
 kb:=pg_catalog.jsonb_build_object('actor_id',k.actor_id,'request_id',k.request_id,
  'request_sha256',k.request_sha256,'operation',k.operation);
 sb:=CASE WHEN s.job_id IS NULL THEN NULL ELSE pg_catalog.jsonb_build_object(
  'job_id',s.job_id,'stage_token',s.stage_token,'capability',s.capability,
  'manifest_sha256',s.manifest_sha256,'plan_sha256',s.plan_sha256,
  'required_spaces',s.required_spaces,'created_at',s.created_at) END;
 job_octets:=pg_catalog.octet_length(pg_catalog.convert_to(jb::pg_catalog.text,'UTF8'));
 outbox_octets:=pg_catalog.octet_length(pg_catalog.convert_to(eb::pg_catalog.text,'UTF8'));
 request_octets:=pg_catalog.octet_length(pg_catalog.convert_to(rb::pg_catalog.text,'UTF8'));
 key_octets:=pg_catalog.octet_length(pg_catalog.convert_to(kb::pg_catalog.text,'UTF8'));
 receipt_octets:=COALESCE(pg_catalog.octet_length(pg_catalog.convert_to(sb::pg_catalog.text,'UTF8')),0);
 IF job_octets NOT BETWEEN 1 AND 1048576 OR outbox_octets NOT BETWEEN 1 AND 1048576
    OR request_octets NOT BETWEEN 1 AND 1048576 OR key_octets NOT BETWEEN 1 AND 1048576
    OR receipt_octets NOT BETWEEN 0 AND 1048576 THEN
  RAISE EXCEPTION 'publication observation row budget exceeded' USING ERRCODE='54000';
 END IF;
 observed_at:=pg_catalog.clock_timestamp();
 job_row:=CASE WHEN p_include_body THEN jb ELSE NULL END;
 outbox_row:=CASE WHEN p_include_body THEN eb ELSE NULL END;
 request_row:=CASE WHEN p_include_body THEN rb ELSE NULL END;
 key_row:=CASE WHEN p_include_body THEN kb ELSE NULL END;
 receipt_row:=CASE WHEN p_include_body THEN sb ELSE NULL END;
 RETURN NEXT;
END
$snapshot_publication_observation_v1$;
