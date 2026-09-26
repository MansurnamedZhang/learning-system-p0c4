-- C3 adds a separate event family without weakening the C2 payload validator.
CREATE FUNCTION public.p0c3_canonical_json(value jsonb) RETURNS text
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE answer text;
BEGIN
 CASE jsonb_typeof(value)
 WHEN 'object' THEN
   SELECT '{' || coalesce(string_agg(to_jsonb(key)::text || ':' || public.p0c3_canonical_json(v), ',' ORDER BY key COLLATE "C"), '') || '}' INTO answer FROM jsonb_each(value) AS e(key,v);
 WHEN 'array' THEN
   SELECT '[' || coalesce(string_agg(public.p0c3_canonical_json(v), ',' ORDER BY n), '') || ']' INTO answer FROM jsonb_array_elements(value) WITH ORDINALITY AS e(v,n);
 ELSE answer := value::text;
 END CASE;
 RETURN answer;
END $$;

CREATE FUNCTION public.p0c3_uuid_object(value jsonb, keys text[]) RETURNS boolean
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE key text; raw text;
BEGIN
 IF jsonb_typeof(value) IS DISTINCT FROM 'object' OR NOT (value ?& keys) OR value - keys <> '{}'::jsonb THEN RETURN false; END IF;
 FOREACH key IN ARRAY keys LOOP
   raw := value->>key;
   IF jsonb_typeof(value->key) IS DISTINCT FROM 'string' OR raw IS NULL
      OR raw !~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
      OR raw = '00000000-0000-0000-0000-000000000000' THEN RETURN false; END IF;
 END LOOP;
 RETURN true;
END $$;

CREATE FUNCTION public.p0c3_valid_job_payload(event_kind text, payload_ver integer, body jsonb, initiator uuid, stable_key text)
RETURNS boolean LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE req jsonb; item jsonb; expected text;
BEGIN
 IF event_kind = 'asset_integrity_requested' THEN RETURN public.p0c2_valid_job_payload(event_kind,payload_ver,body,initiator,stable_key); END IF;
 IF event_kind IS DISTINCT FROM 'snapshot_export_requested' OR payload_ver IS DISTINCT FROM 1
    OR initiator IS NULL OR stable_key IS NULL OR jsonb_typeof(body) IS DISTINCT FROM 'object'
    OR NOT (body ?& ARRAY['version','kind','actor_id','space_id','request'])
    OR body - ARRAY['version','kind','actor_id','space_id','request'] <> '{}'::jsonb
    OR body->>'version' IS DISTINCT FROM '1' OR body->'version' IS DISTINCT FROM '1'::jsonb OR body->'kind' IS DISTINCT FROM '"snapshot_export"'::jsonb
    OR NOT public.p0c3_uuid_object(body - ARRAY['version','kind','request'],ARRAY['actor_id','space_id'])
    OR body->>'actor_id' IS DISTINCT FROM initiator::text THEN RETURN false; END IF;
 req := body->'request';
 IF jsonb_typeof(req) IS DISTINCT FROM 'object'
    OR NOT (req ?& ARRAY['reading','mode','include_personal','include_originals','resource_versions','source_segments'])
    OR req - ARRAY['reading','mode','include_personal','include_originals','resource_versions','source_segments'] <> '{}'::jsonb
    OR NOT public.p0c3_uuid_object(req->'reading',ARRAY['view_id','revision_id'])
    OR jsonb_typeof(req->'mode') IS DISTINCT FROM 'string' OR req->>'mode' NOT IN ('original','fused','personal')
    OR jsonb_typeof(req->'include_personal') IS DISTINCT FROM 'boolean'
    OR jsonb_typeof(req->'include_originals') IS DISTINCT FROM 'boolean'
    OR jsonb_typeof(req->'resource_versions') IS DISTINCT FROM 'array'
    OR jsonb_typeof(req->'source_segments') IS DISTINCT FROM 'array' THEN RETURN false; END IF;
 IF jsonb_array_length(req->'resource_versions') + jsonb_array_length(req->'source_segments') > 2048 THEN RETURN false; END IF;
 FOR item IN SELECT * FROM jsonb_array_elements(req->'resource_versions') LOOP
   IF NOT public.p0c3_uuid_object(item,ARRAY['space_id','resource_id','version_id']) THEN RETURN false; END IF;
 END LOOP;
 FOR item IN SELECT * FROM jsonb_array_elements(req->'source_segments') LOOP
   IF NOT public.p0c3_uuid_object(item,ARRAY['space_id','resource_id','version_id','segment_id']) THEN RETURN false; END IF;
 END LOOP;
 expected := 'snapshot-export:v1:' || initiator::text || ':' || (body->>'space_id') || ':' || encode(sha256(convert_to(public.p0c3_canonical_json(req),'UTF8')),'hex');
 RETURN stable_key = expected;
END $$;

ALTER TABLE public.job_outbox DROP CONSTRAINT job_outbox_event_type_check;
ALTER TABLE public.job_outbox DROP CONSTRAINT job_outbox_check;
ALTER TABLE public.job_outbox ADD CONSTRAINT job_outbox_event_type_check CHECK(event_type IN ('asset_integrity_requested','snapshot_export_requested'));
ALTER TABLE public.job_outbox ADD CONSTRAINT job_outbox_check CHECK(public.p0c3_valid_job_payload(event_type,payload_version,payload,actor_id,business_key));
ALTER TABLE public.request_key DROP CONSTRAINT request_key_operation_check;
ALTER TABLE public.request_key ADD CONSTRAINT request_key_operation_check CHECK(operation IN
 ('content_v1','composition_save','release_publish','reading_create','reading_edit',
 'migration_propose','migration_decide','content_v2','relation_save','relation_review','epistemic_review',
 'reading_select','release_publish_evidence','lineage_apply','asset_register','content_v3','snapshot_export'));

-- The request receipt reserves a stable job ID before asynchronous dispatch.
CREATE TABLE public.snapshot_export_request (
 actor_id uuid NOT NULL, request_id uuid NOT NULL,
 outbox_id uuid NOT NULL REFERENCES public.job_outbox(id),
 job_id uuid NOT NULL,
 PRIMARY KEY(actor_id,request_id),
 FOREIGN KEY(actor_id,request_id) REFERENCES public.request_key(actor_id,request_id)
);
REVOKE ALL ON public.snapshot_export_request FROM PUBLIC;
GRANT SELECT,INSERT ON public.snapshot_export_request TO learning_runtime;

-- C3 runnable states are invisible to the deployed C2 binary's fixed scan.
ALTER TABLE public.job DROP CONSTRAINT job_status_check;
ALTER TABLE public.job ADD CONSTRAINT job_status_check CHECK(status IN
 ('queued','running','retry_wait','snapshot_queued','snapshot_running','snapshot_retry_wait','succeeded','failed','cancelled'));
ALTER TABLE public.job DROP CONSTRAINT job_check;
ALTER TABLE public.job DROP CONSTRAINT job_check1;
ALTER TABLE public.job DROP CONSTRAINT job_check2;
ALTER TABLE public.job ADD CONSTRAINT job_check CHECK (
 (status IN ('running','snapshot_running') AND attempt_count>0 AND lease_token IS NOT NULL AND lease_expires_at IS NOT NULL)
 OR (status NOT IN ('running','snapshot_running') AND lease_token IS NULL AND lease_expires_at IS NULL));
ALTER TABLE public.job ADD CONSTRAINT job_check1 CHECK (
 (status IN ('retry_wait','snapshot_retry_wait') AND next_attempt_at IS NOT NULL)
 OR (status NOT IN ('retry_wait','snapshot_retry_wait') AND next_attempt_at IS NULL));
ALTER TABLE public.job ADD CONSTRAINT job_check2 CHECK (status NOT IN ('queued','snapshot_queued') OR
 (attempt_count=0 AND last_error_class IS NULL AND output_digest IS NULL AND checkpoint IS NULL));
CREATE INDEX job_snapshot_runnable ON public.job(status,next_attempt_at,created_at,id)
 WHERE status IN ('snapshot_queued','snapshot_retry_wait','snapshot_running');

CREATE OR REPLACE FUNCTION public.p0c2_guard_job()
RETURNS trigger LANGUAGE plpgsql
SET search_path = pg_catalog, public, pg_temp AS $$
DECLARE is_snapshot boolean; new_state text; old_state text;
BEGIN
    SELECT e.event_type='snapshot_export_requested' INTO is_snapshot FROM public.job_outbox e WHERE e.id=NEW.outbox_id;
    IF is_snapshot IS TRUE THEN
        IF TG_OP='INSERT' THEN
            NEW.id := NEW.outbox_id;
            IF NEW.status='queued' THEN NEW.status := 'snapshot_queued'; END IF;
        END IF;
        IF NEW.status NOT IN ('snapshot_queued','snapshot_running','snapshot_retry_wait','succeeded','failed','cancelled') THEN
            RAISE EXCEPTION 'wrong snapshot job state family' USING ERRCODE='23514';
        END IF;
        new_state := replace(NEW.status,'snapshot_','');
        IF TG_OP='UPDATE' THEN old_state := replace(OLD.status,'snapshot_',''); END IF;
    ELSE
        IF NEW.status NOT IN ('queued','running','retry_wait','succeeded','failed','cancelled') THEN
            RAISE EXCEPTION 'wrong asset job state family' USING ERRCODE='23514';
        END IF;
        new_state := NEW.status;
        IF TG_OP='UPDATE' THEN old_state := OLD.status; END IF;
    END IF;
    IF TG_OP = 'INSERT' THEN
        IF new_state <> 'queued' OR NEW.attempt_count <> 0 THEN
            RAISE EXCEPTION 'job must start queued' USING ERRCODE = '23514';
        END IF;
    ELSE
        IF NEW.id IS DISTINCT FROM OLD.id
           OR NEW.outbox_id IS DISTINCT FROM OLD.outbox_id
           OR NEW.idempotency_key IS DISTINCT FROM OLD.idempotency_key
           OR NEW.created_at IS DISTINCT FROM OLD.created_at
           OR old_state IN ('succeeded','failed','cancelled')
           OR (old_state = 'queued' AND new_state NOT IN ('running','cancelled'))
           OR (old_state = 'running' AND new_state NOT IN
               ('running','retry_wait','succeeded','failed','cancelled'))
           OR (old_state = 'retry_wait' AND new_state NOT IN
               ('running','cancelled'))
           OR (old_state IN ('queued','retry_wait') AND new_state = 'running'
               AND NEW.attempt_count <> OLD.attempt_count + 1)
           OR (old_state = 'running' AND new_state = 'running'
               AND ((NEW.lease_token IS NOT DISTINCT FROM OLD.lease_token
                        AND NEW.attempt_count <> OLD.attempt_count)
                   OR (NEW.lease_token IS DISTINCT FROM OLD.lease_token
                        AND (OLD.lease_expires_at > clock_timestamp()
                             OR NEW.attempt_count <> OLD.attempt_count + 1))))
           OR (new_state <> 'running' AND NEW.attempt_count <> OLD.attempt_count) THEN
            RAISE EXCEPTION 'invalid job transition' USING ERRCODE = '23514';
        END IF;
        NEW.updated_at := clock_timestamp();
    END IF;
    RETURN NEW;
END $$;
CREATE OR REPLACE FUNCTION public.p0c2_claim_job(p_job_id uuid, p_lease_ms bigint)
RETURNS TABLE(job_id uuid, token uuid, attempts integer, expires_at timestamptz)
LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF p_lease_ms IS NULL OR p_lease_ms NOT BETWEEN 1 AND 300000 THEN
        RAISE EXCEPTION 'invalid lease request' USING ERRCODE = '22023';
    END IF;

    UPDATE public.job AS j
       SET status = 'failed', lease_token = NULL, lease_expires_at = NULL,
           last_error_class = 'lease_expired', output_digest = NULL
     WHERE j.id = p_job_id AND EXISTS (SELECT 1 FROM public.job_outbox e WHERE e.id=j.outbox_id AND e.event_type='asset_integrity_requested') AND j.status = 'running'
       AND j.lease_expires_at <= clock_timestamp() AND j.attempt_count >= 3;

    RETURN QUERY
    UPDATE public.job AS j
       SET status = 'running', attempt_count = j.attempt_count + 1,
           lease_token = pg_catalog.gen_random_uuid(),
           lease_expires_at = clock_timestamp() + p_lease_ms * interval '1 millisecond',
           next_attempt_at = NULL, last_error_class = NULL,
           output_digest = NULL
     WHERE j.id = p_job_id AND EXISTS (SELECT 1 FROM public.job_outbox e WHERE e.id=j.outbox_id AND e.event_type='asset_integrity_requested') AND j.attempt_count < 3
       AND (j.status = 'queued'
         OR (j.status = 'retry_wait' AND j.next_attempt_at <= clock_timestamp())
         OR (j.status = 'running' AND j.lease_expires_at <= clock_timestamp()))
     RETURNING j.id, j.lease_token, j.attempt_count, j.lease_expires_at;
END $$;

CREATE OR REPLACE FUNCTION public.p0c3_claim_snapshot_job(p_job_id uuid, p_lease_ms bigint)
RETURNS TABLE(job_id uuid, token uuid, attempts integer, expires_at timestamptz)
LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp AS $$
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
END $$;


CREATE TABLE public.snapshot_export_result (
 job_id uuid PRIMARY KEY REFERENCES public.job(id),
 stage_token uuid NOT NULL,
 capability text NOT NULL CHECK(capability IN ('exact_import_v1','reading_copy_v1')),
 manifest_sha256 text NOT NULL CHECK(manifest_sha256 ~ '^[0-9a-f]{64}$'),
 plan_sha256 text NOT NULL CHECK(plan_sha256 ~ '^[0-9a-f]{64}$'),
 required_spaces uuid[] NOT NULL CHECK(cardinality(required_spaces)>0),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE FUNCTION public.p0c3_guard_snapshot_result() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF TG_OP <> 'INSERT' THEN RAISE EXCEPTION 'snapshot result immutable' USING ERRCODE='23514'; END IF;
 IF NOT EXISTS(SELECT 1 FROM public.job j JOIN public.job_outbox e ON e.id=j.outbox_id
   WHERE j.id=NEW.job_id AND j.status='succeeded' AND j.output_digest=NEW.manifest_sha256
     AND e.event_type='snapshot_export_requested') THEN
   RAISE EXCEPTION 'snapshot result mismatches job' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER p0c3_guard_snapshot_result_change BEFORE INSERT OR UPDATE OR DELETE ON public.snapshot_export_result
 FOR EACH ROW EXECUTE FUNCTION public.p0c3_guard_snapshot_result();

CREATE FUNCTION public.p0c3_renew_snapshot_job(p_job_id uuid, p_token uuid, p_lease_ms bigint)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF p_lease_ms IS NULL OR p_lease_ms NOT BETWEEN 1 AND 300000 THEN
        RAISE EXCEPTION 'invalid lease duration' USING ERRCODE = '22023';
    END IF;
    UPDATE public.job AS j
       SET lease_expires_at = clock_timestamp() + p_lease_ms * interval '1 millisecond'
     WHERE j.id = p_job_id AND j.status = 'snapshot_running' AND j.lease_token = p_token
       AND j.lease_expires_at > clock_timestamp();
    RETURN FOUND;
END $$;

CREATE FUNCTION public.p0c3_checkpoint_snapshot_job(p_job_id uuid, p_token uuid, p_checkpoint jsonb)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF jsonb_typeof(p_checkpoint) IS DISTINCT FROM 'object'
       OR octet_length(p_checkpoint::text) > 16384 THEN
        RAISE EXCEPTION 'invalid checkpoint' USING ERRCODE = '22023';
    END IF;
    UPDATE public.job AS j SET checkpoint = p_checkpoint
     WHERE j.id = p_job_id AND j.status = 'snapshot_running' AND j.lease_token = p_token
       AND j.lease_expires_at > clock_timestamp();
    RETURN FOUND;
END $$;

CREATE FUNCTION public.p0c3_succeed_snapshot_job(p_job_id uuid, p_token uuid, p_digest text)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF p_digest IS NULL OR p_digest !~ '^[0-9a-f]{64}$' THEN
        RAISE EXCEPTION 'invalid output digest' USING ERRCODE = '22023';
    END IF;
    UPDATE public.job AS j
       SET status = 'succeeded', lease_token = NULL, lease_expires_at = NULL,
           next_attempt_at = NULL, last_error_class = NULL,
           output_digest = p_digest
     WHERE j.id = p_job_id AND j.status = 'snapshot_running' AND j.lease_token = p_token
       AND j.lease_expires_at > clock_timestamp();
    RETURN FOUND;
END $$;

CREATE FUNCTION public.p0c3_fail_snapshot_job(
    p_job_id uuid, p_token uuid, p_error_class text, p_retryable boolean
)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp AS $$
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
END $$;

CREATE FUNCTION public.p0c3_cancel_snapshot_job(p_job_id uuid)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    UPDATE public.job AS j
       SET status = 'cancelled', lease_token = NULL, lease_expires_at = NULL,
           next_attempt_at = NULL, output_digest = NULL
     WHERE j.id = p_job_id AND j.status IN ('snapshot_queued', 'snapshot_running', 'snapshot_retry_wait');
    RETURN FOUND;
END $$;

REVOKE ALL ON FUNCTION public.p0c3_renew_snapshot_job(uuid,uuid,bigint),
 public.p0c3_checkpoint_snapshot_job(uuid,uuid,jsonb),public.p0c3_succeed_snapshot_job(uuid,uuid,text),
 public.p0c3_fail_snapshot_job(uuid,uuid,text,boolean),public.p0c3_cancel_snapshot_job(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.p0c3_renew_snapshot_job(uuid,uuid,bigint),
 public.p0c3_checkpoint_snapshot_job(uuid,uuid,jsonb),public.p0c3_succeed_snapshot_job(uuid,uuid,text),
 public.p0c3_fail_snapshot_job(uuid,uuid,text,boolean),public.p0c3_cancel_snapshot_job(uuid) TO learning_runtime;

CREATE FUNCTION public.p0c3_complete_snapshot_job(p_job_id uuid,p_token uuid,p_capability text,p_manifest text,p_plan text,p_spaces uuid[])
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE initiator uuid; expected_capability text; required uuid;
BEGIN
 IF p_token IS NULL OR p_capability IS NULL OR p_manifest IS NULL OR p_manifest !~ '^[0-9a-f]{64}$'
    OR p_plan IS NULL OR p_plan !~ '^[0-9a-f]{64}$' OR p_spaces IS NULL OR cardinality(p_spaces)=0
    OR array_position(p_spaces,NULL) IS NOT NULL THEN RAISE EXCEPTION 'invalid snapshot result' USING ERRCODE='22023'; END IF;
 SELECT e.actor_id, CASE WHEN (e.payload->'request'->>'include_personal')::boolean THEN 'exact_import_v1' ELSE 'reading_copy_v1' END
 INTO initiator,expected_capability FROM public.job j JOIN public.job_outbox e ON e.id=j.outbox_id
 WHERE j.id=p_job_id AND e.event_type='snapshot_export_requested' AND e.processor_version=1;
 IF initiator IS NULL OR p_capability <> expected_capability THEN RETURN false; END IF;
 -- The trusted Rust boundary supplies the complete freshly replanned closure.
 -- Recheck locks here too; all are acquired in stable UUID order.
 FOR required IN SELECT DISTINCT unnest(p_spaces) ORDER BY 1 LOOP
   IF public.lock_space_grant(initiator,required) IS NULL THEN RETURN false; END IF;
 END LOOP;
 IF NOT public.p0c3_succeed_snapshot_job(p_job_id,p_token,p_manifest) THEN RETURN false; END IF;
 INSERT INTO public.snapshot_export_result(job_id,stage_token,capability,manifest_sha256,plan_sha256,required_spaces)
 VALUES(p_job_id,p_token,p_capability,p_manifest,p_plan,p_spaces);
 RETURN true;
END $$;
REVOKE ALL ON public.snapshot_export_result FROM PUBLIC;
GRANT SELECT ON public.snapshot_export_result TO learning_runtime;
REVOKE ALL ON FUNCTION public.p0c3_canonical_json(jsonb),public.p0c3_uuid_object(jsonb,text[]),
 public.p0c3_valid_job_payload(text,integer,jsonb,uuid,text),public.p0c3_claim_snapshot_job(uuid,bigint),
 public.p0c3_guard_snapshot_result(),public.p0c3_complete_snapshot_job(uuid,uuid,text,text,text,uuid[]) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.p0c3_canonical_json(jsonb),public.p0c3_uuid_object(jsonb,text[]),
 public.p0c3_valid_job_payload(text,integer,jsonb,uuid,text),public.p0c3_claim_snapshot_job(uuid,bigint),
 public.p0c3_complete_snapshot_job(uuid,uuid,text,text,text,uuid[]) TO learning_runtime;
