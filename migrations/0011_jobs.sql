-- Durable work is separate from the release-only outbox_event.
CREATE FUNCTION public.p0c2_valid_job_payload(
    event_kind text, payload_ver integer, body jsonb, initiator uuid, stable_key text
)
RETURNS boolean LANGUAGE plpgsql IMMUTABLE
SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF event_kind <> 'asset_integrity_requested' OR payload_ver <> 1
        OR initiator = '00000000-0000-0000-0000-000000000000'::uuid
        OR jsonb_typeof(body) IS DISTINCT FROM 'object'
        OR octet_length(body::text) > 4096
        OR NOT (body ?& ARRAY['version','kind','actor_id','space_id','block_id','revision_id'])
        OR body - 'version' - 'kind' - 'actor_id' - 'space_id' - 'block_id' - 'revision_id' <> '{}'::jsonb
        OR jsonb_typeof(body->'version') IS DISTINCT FROM 'number'
        OR jsonb_typeof(body->'kind') IS DISTINCT FROM 'string'
        OR jsonb_typeof(body->'actor_id') IS DISTINCT FROM 'string'
        OR jsonb_typeof(body->'space_id') IS DISTINCT FROM 'string'
        OR jsonb_typeof(body->'block_id') IS DISTINCT FROM 'string'
        OR jsonb_typeof(body->'revision_id') IS DISTINCT FROM 'string'
        OR body->>'version' <> '1'
        OR body->>'kind' <> 'asset_integrity'
        OR body->>'actor_id' <> initiator::text THEN
        RETURN false;
    END IF;
    RETURN COALESCE((body->>'space_id')::uuid <> '00000000-0000-0000-0000-000000000000'::uuid
       AND (body->>'block_id')::uuid <> '00000000-0000-0000-0000-000000000000'::uuid
       AND (body->>'revision_id')::uuid <> '00000000-0000-0000-0000-000000000000'::uuid
       AND stable_key = ('asset-integrity:v1:' || (body->>'space_id')::uuid::text
           || ':' || (body->>'block_id')::uuid::text
           || ':' || (body->>'revision_id')::uuid::text), false);
EXCEPTION WHEN invalid_text_representation THEN
    RETURN false;
END $$;

CREATE TABLE public.job_outbox (
    id uuid PRIMARY KEY,
    business_key text NOT NULL UNIQUE
        CHECK (char_length(business_key) BETWEEN 1 AND 255 AND business_key = btrim(business_key)),
    event_type text NOT NULL CHECK (event_type = 'asset_integrity_requested'),
    payload_version integer NOT NULL CHECK (payload_version = 1),
    payload jsonb NOT NULL,
    actor_id uuid NOT NULL REFERENCES public.app_user(id),
    processor_version integer NOT NULL CHECK (processor_version = 1),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    dispatched_at timestamptz,
    UNIQUE (id, business_key),
    CHECK (public.p0c2_valid_job_payload(event_type, payload_version, payload, actor_id, business_key))
);
CREATE INDEX job_outbox_undispatched ON public.job_outbox(created_at, id)
    WHERE dispatched_at IS NULL;

CREATE TABLE public.job (
    id uuid PRIMARY KEY,
    outbox_id uuid NOT NULL UNIQUE,
    idempotency_key text NOT NULL UNIQUE,
    status text NOT NULL CHECK (status IN
        ('queued','running','retry_wait','succeeded','failed','cancelled')),
    attempt_count integer NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    lease_token uuid,
    lease_expires_at timestamptz,
    next_attempt_at timestamptz,
    last_error_class text CHECK (last_error_class IS NULL OR
        (char_length(last_error_class) BETWEEN 1 AND 80 AND
         last_error_class ~ '^[a-z][a-z0-9_]*$')),
    output_digest text CHECK (output_digest IS NULL OR output_digest ~ '^[0-9a-f]{64}$'),
    checkpoint jsonb CHECK (checkpoint IS NULL OR
        (jsonb_typeof(checkpoint) = 'object' AND octet_length(checkpoint::text) <= 16384)),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    FOREIGN KEY (outbox_id, idempotency_key)
        REFERENCES public.job_outbox(id, business_key),
    CHECK (
        (status = 'running' AND attempt_count > 0 AND lease_token IS NOT NULL
            AND lease_expires_at IS NOT NULL)
        OR (status <> 'running' AND lease_token IS NULL AND lease_expires_at IS NULL)
    ),
    CHECK ((status = 'retry_wait' AND next_attempt_at IS NOT NULL)
        OR (status <> 'retry_wait' AND next_attempt_at IS NULL)),
    CHECK (status <> 'queued' OR (attempt_count = 0
        AND last_error_class IS NULL AND output_digest IS NULL AND checkpoint IS NULL))
);
CREATE INDEX job_runnable ON public.job(status, next_attempt_at, created_at, id)
    WHERE status IN ('queued','retry_wait','running');

CREATE FUNCTION public.p0c2_guard_job_outbox()
RETURNS trigger LANGUAGE plpgsql
SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF NEW.dispatched_at IS NOT NULL THEN
            RAISE EXCEPTION 'new job event is already dispatched' USING ERRCODE = '23514';
        END IF;
        RETURN NEW;
    END IF;
    IF NEW.id IS DISTINCT FROM OLD.id
       OR NEW.business_key IS DISTINCT FROM OLD.business_key
       OR NEW.event_type IS DISTINCT FROM OLD.event_type
       OR NEW.payload_version IS DISTINCT FROM OLD.payload_version
       OR NEW.payload IS DISTINCT FROM OLD.payload
       OR NEW.actor_id IS DISTINCT FROM OLD.actor_id
       OR NEW.processor_version IS DISTINCT FROM OLD.processor_version
       OR NEW.created_at IS DISTINCT FROM OLD.created_at
       OR (OLD.dispatched_at IS NOT NULL
           AND NEW.dispatched_at IS DISTINCT FROM OLD.dispatched_at) THEN
        RAISE EXCEPTION 'job outbox history is immutable' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER p0c2_guard_job_outbox_update
    BEFORE INSERT OR UPDATE ON public.job_outbox FOR EACH ROW
    EXECUTE FUNCTION public.p0c2_guard_job_outbox();

CREATE FUNCTION public.p0c2_guard_job()
RETURNS trigger LANGUAGE plpgsql
SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF NEW.status <> 'queued' OR NEW.attempt_count <> 0 THEN
            RAISE EXCEPTION 'job must start queued' USING ERRCODE = '23514';
        END IF;
    ELSE
        IF NEW.id IS DISTINCT FROM OLD.id
           OR NEW.outbox_id IS DISTINCT FROM OLD.outbox_id
           OR NEW.idempotency_key IS DISTINCT FROM OLD.idempotency_key
           OR NEW.created_at IS DISTINCT FROM OLD.created_at
           OR OLD.status IN ('succeeded','failed','cancelled')
           OR (OLD.status = 'queued' AND NEW.status NOT IN ('running','cancelled'))
           OR (OLD.status = 'running' AND NEW.status NOT IN
               ('running','retry_wait','succeeded','failed','cancelled'))
           OR (OLD.status = 'retry_wait' AND NEW.status NOT IN
               ('running','cancelled'))
           OR (OLD.status IN ('queued','retry_wait') AND NEW.status = 'running'
               AND NEW.attempt_count <> OLD.attempt_count + 1)
           OR (OLD.status = 'running' AND NEW.status = 'running'
               AND ((NEW.lease_token IS NOT DISTINCT FROM OLD.lease_token
                        AND NEW.attempt_count <> OLD.attempt_count)
                   OR (NEW.lease_token IS DISTINCT FROM OLD.lease_token
                        AND (OLD.lease_expires_at > clock_timestamp()
                             OR NEW.attempt_count <> OLD.attempt_count + 1))))
           OR (NEW.status <> 'running' AND NEW.attempt_count <> OLD.attempt_count) THEN
            RAISE EXCEPTION 'invalid job transition' USING ERRCODE = '23514';
        END IF;
        NEW.updated_at := clock_timestamp();
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER p0c2_guard_job_change
    BEFORE INSERT OR UPDATE ON public.job FOR EACH ROW
    EXECUTE FUNCTION public.p0c2_guard_job();

-- Both directions are deferred: either insertion order is legal inside one
-- transaction, but a half-dispatched event cannot commit.
CREATE FUNCTION public.p0c2_check_dispatch()
RETURNS trigger LANGUAGE plpgsql
SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF TG_TABLE_NAME = 'job_outbox' THEN
        IF NEW.dispatched_at IS NOT NULL AND NOT EXISTS (
            SELECT 1 FROM public.job j
             WHERE j.outbox_id = NEW.id AND j.idempotency_key = NEW.business_key
        ) THEN
            RAISE EXCEPTION 'dispatched event has no job' USING ERRCODE = '23514';
        END IF;
    ELSIF NOT EXISTS (
        SELECT 1 FROM public.job_outbox e
         WHERE e.id = NEW.outbox_id AND e.dispatched_at IS NOT NULL
    ) THEN
        RAISE EXCEPTION 'job has no dispatched event' USING ERRCODE = '23514';
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER p0c2_outbox_dispatch_complete
    AFTER UPDATE ON public.job_outbox DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION public.p0c2_check_dispatch();
CREATE CONSTRAINT TRIGGER p0c2_job_dispatch_complete
    AFTER INSERT ON public.job DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION public.p0c2_check_dispatch();

REVOKE ALL ON public.job_outbox, public.job FROM PUBLIC;
GRANT SELECT, INSERT ON public.job_outbox, public.job TO learning_runtime;
GRANT UPDATE(dispatched_at) ON public.job_outbox TO learning_runtime;
GRANT UPDATE(status,attempt_count,lease_token,lease_expires_at,next_attempt_at,
             last_error_class,output_digest,checkpoint) ON public.job TO learning_runtime;
REVOKE ALL ON FUNCTION public.p0c2_valid_job_payload(text,integer,jsonb,uuid,text),
    public.p0c2_guard_job_outbox(), public.p0c2_guard_job(),
    public.p0c2_check_dispatch() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.p0c2_valid_job_payload(text,integer,jsonb,uuid,text),
    public.p0c2_guard_job_outbox(), public.p0c2_guard_job(),
    public.p0c2_check_dispatch() TO learning_runtime;
