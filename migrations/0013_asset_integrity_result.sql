-- P0-C2 Task 4: a versioned, immutable result for the first real processor.
-- The runtime cannot insert it directly. Its only write route is the fenced
-- completion function, called after a current-authorization check in one tx.
ALTER TABLE public.job ADD CONSTRAINT p0c2_job_identity_pair
    UNIQUE (id, idempotency_key);

CREATE TABLE public.asset_integrity_result (
    job_id uuid PRIMARY KEY,
    idempotency_key text NOT NULL UNIQUE,
    processor_version integer NOT NULL CHECK (processor_version = 1),
    result_version integer NOT NULL CHECK (result_version = 1),
    space_id uuid NOT NULL,
    block_id uuid NOT NULL,
    revision_id uuid NOT NULL,
    asset_id uuid NOT NULL,
    sha256 text NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    byte_size bigint NOT NULL CHECK (byte_size >= 0),
    media_type text NOT NULL CHECK (char_length(media_type) BETWEEN 1 AND 200),
    output_digest text NOT NULL CHECK (output_digest ~ '^[0-9a-f]{64}$'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    FOREIGN KEY (job_id, idempotency_key)
        REFERENCES public.job(id, idempotency_key),
    FOREIGN KEY (space_id, block_id, revision_id)
        REFERENCES public.block_asset_use(space_id, block_id, revision_id),
    FOREIGN KEY (space_id, asset_id)
        REFERENCES public.asset(space_id, id)
);

CREATE FUNCTION public.p0c2_guard_asset_integrity_result()
RETURNS trigger LANGUAGE plpgsql
SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF TG_OP <> 'INSERT' THEN
        RAISE EXCEPTION 'asset integrity result is immutable' USING ERRCODE = '23514';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM public.job AS j
         WHERE j.id = NEW.job_id AND j.idempotency_key = NEW.idempotency_key
           AND j.status = 'succeeded' AND j.output_digest = NEW.output_digest
    ) THEN
        RAISE EXCEPTION 'asset result does not match completed job'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER p0c2_guard_asset_integrity_result_change
    BEFORE INSERT OR UPDATE OR DELETE ON public.asset_integrity_result
    FOR EACH ROW EXECUTE FUNCTION public.p0c2_guard_asset_integrity_result();

CREATE FUNCTION public.p0c2_complete_asset_job(
    p_job_id uuid, p_token uuid, p_space_id uuid, p_block_id uuid,
    p_revision_id uuid, p_asset_id uuid, p_sha256 text,
    p_byte_size bigint, p_media_type text, p_digest text
)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp AS $$
DECLARE inserted integer;
BEGIN
    IF p_job_id IS NULL OR p_token IS NULL OR p_space_id IS NULL
       OR p_block_id IS NULL OR p_revision_id IS NULL OR p_asset_id IS NULL
       OR p_sha256 IS NULL OR p_sha256 !~ '^[0-9a-f]{64}$'
       OR p_byte_size IS NULL OR p_byte_size < 0
       OR p_media_type IS NULL OR char_length(p_media_type) NOT BETWEEN 1 AND 200
       OR p_digest IS NULL OR p_digest !~ '^[0-9a-f]{64}$' THEN
        RAISE EXCEPTION 'invalid asset result' USING ERRCODE = '22023';
    END IF;
    -- A caller cannot substitute a different space, exact revision, asset,
    -- fingerprint, or media identity for the immutable business event.
    IF NOT EXISTS (
        SELECT 1 FROM public.job AS j
        JOIN public.job_outbox AS e ON e.id = j.outbox_id
        JOIN public.block_asset_use AS u
          ON (u.space_id, u.block_id, u.revision_id)
           = (p_space_id, p_block_id, p_revision_id)
        JOIN public.asset AS a
          ON (a.space_id, a.id) = (u.space_id, u.asset_id)
        JOIN public.space_grant AS g
          ON g.space_id = u.space_id AND g.actor_id = e.actor_id
        WHERE j.id = p_job_id AND j.status = 'running'
          AND e.event_type = 'asset_integrity_requested'
          AND e.payload_version = 1 AND e.processor_version = 1
          AND e.payload->>'space_id' = p_space_id::text
          AND e.payload->>'block_id' = p_block_id::text
          AND e.payload->>'revision_id' = p_revision_id::text
          AND e.payload->>'actor_id' = e.actor_id::text
          AND a.id = p_asset_id AND a.status = 'ready'
          AND a.sha256 = p_sha256 AND a.byte_size = p_byte_size
          AND a.media_type = p_media_type
    ) THEN
        RETURN false;
    END IF;
    IF NOT public.p0c2_succeed_job(p_job_id, p_token, p_digest) THEN
        RETURN false;
    END IF;
    INSERT INTO public.asset_integrity_result (
        job_id, idempotency_key, processor_version, result_version,
        space_id, block_id, revision_id, asset_id,
        sha256, byte_size, media_type, output_digest
    )
    SELECT j.id, j.idempotency_key, e.processor_version, 1,
           p_space_id, p_block_id, p_revision_id, p_asset_id,
           p_sha256, p_byte_size, p_media_type, p_digest
      FROM public.job AS j
      JOIN public.job_outbox AS e ON e.id = j.outbox_id
     WHERE j.id = p_job_id;
    GET DIAGNOSTICS inserted = ROW_COUNT;
    IF inserted <> 1 THEN
        RAISE EXCEPTION 'asset result insert did not affect one row'
            USING ERRCODE = '23514';
    END IF;
    RETURN true;
END $$;

REVOKE ALL ON public.asset_integrity_result FROM PUBLIC;
GRANT SELECT ON public.asset_integrity_result TO learning_runtime;
REVOKE ALL ON FUNCTION public.p0c2_guard_asset_integrity_result(),
    public.p0c2_complete_asset_job(uuid,uuid,uuid,uuid,uuid,uuid,text,bigint,text,text)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION
    public.p0c2_complete_asset_job(uuid,uuid,uuid,uuid,uuid,uuid,text,bigint,text,text)
    TO learning_runtime;
