-- The runtime may create a queued job during outbox transfer, but only the
-- trusted migration owner may implement its subsequent state transitions.
-- The functions below are the sole runtime mutation boundary for leases.
REVOKE UPDATE(status, attempt_count, lease_token, lease_expires_at,
              next_attempt_at, last_error_class, output_digest, checkpoint)
    ON public.job FROM learning_runtime;
REVOKE SELECT ON public.job FROM learning_runtime;
GRANT SELECT(id, outbox_id, idempotency_key, status, attempt_count,
             lease_expires_at, next_attempt_at, last_error_class,
             output_digest, checkpoint, created_at, updated_at)
    ON public.job TO learning_runtime;

-- Three attempts including the first claim. An expired third lease is made
-- terminal here, so an abandoned job cannot remain running forever.
CREATE FUNCTION public.p0c2_claim_job(p_job_id uuid, p_lease_ms bigint)
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
     WHERE j.id = p_job_id AND j.status = 'running'
       AND j.lease_expires_at <= clock_timestamp() AND j.attempt_count >= 3;

    RETURN QUERY
    UPDATE public.job AS j
       SET status = 'running', attempt_count = j.attempt_count + 1,
           lease_token = pg_catalog.gen_random_uuid(),
           lease_expires_at = clock_timestamp() + p_lease_ms * interval '1 millisecond',
           next_attempt_at = NULL, last_error_class = NULL,
           output_digest = NULL
     WHERE j.id = p_job_id AND j.attempt_count < 3
       AND (j.status = 'queued'
         OR (j.status = 'retry_wait' AND j.next_attempt_at <= clock_timestamp())
         OR (j.status = 'running' AND j.lease_expires_at <= clock_timestamp()))
     RETURNING j.id, j.lease_token, j.attempt_count, j.lease_expires_at;
END $$;

CREATE FUNCTION public.p0c2_renew_job(p_job_id uuid, p_token uuid, p_lease_ms bigint)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF p_lease_ms IS NULL OR p_lease_ms NOT BETWEEN 1 AND 300000 THEN
        RAISE EXCEPTION 'invalid lease duration' USING ERRCODE = '22023';
    END IF;
    UPDATE public.job AS j
       SET lease_expires_at = clock_timestamp() + p_lease_ms * interval '1 millisecond'
     WHERE j.id = p_job_id AND j.status = 'running' AND j.lease_token = p_token
       AND j.lease_expires_at > clock_timestamp();
    RETURN FOUND;
END $$;

CREATE FUNCTION public.p0c2_checkpoint_job(p_job_id uuid, p_token uuid, p_checkpoint jsonb)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    IF jsonb_typeof(p_checkpoint) IS DISTINCT FROM 'object'
       OR octet_length(p_checkpoint::text) > 16384 THEN
        RAISE EXCEPTION 'invalid checkpoint' USING ERRCODE = '22023';
    END IF;
    UPDATE public.job AS j SET checkpoint = p_checkpoint
     WHERE j.id = p_job_id AND j.status = 'running' AND j.lease_token = p_token
       AND j.lease_expires_at > clock_timestamp();
    RETURN FOUND;
END $$;

CREATE FUNCTION public.p0c2_succeed_job(p_job_id uuid, p_token uuid, p_digest text)
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
     WHERE j.id = p_job_id AND j.status = 'running' AND j.lease_token = p_token
       AND j.lease_expires_at > clock_timestamp();
    RETURN FOUND;
END $$;

CREATE FUNCTION public.p0c2_fail_job(
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
                         THEN 'retry_wait' ELSE 'failed' END,
           lease_token = NULL, lease_expires_at = NULL,
           next_attempt_at = CASE WHEN p_retryable AND j.attempt_count < 3
                THEN clock_timestamp() + CASE j.attempt_count
                    WHEN 1 THEN interval '1 second' ELSE interval '2 seconds' END
                ELSE NULL END,
           last_error_class = p_error_class, output_digest = NULL
     WHERE j.id = p_job_id AND j.status = 'running' AND j.lease_token = p_token
       AND j.lease_expires_at > clock_timestamp();
    RETURN FOUND;
END $$;

CREATE FUNCTION public.p0c2_cancel_job(p_job_id uuid)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    UPDATE public.job AS j
       SET status = 'cancelled', lease_token = NULL, lease_expires_at = NULL,
           next_attempt_at = NULL, output_digest = NULL
     WHERE j.id = p_job_id AND j.status IN ('queued', 'running', 'retry_wait');
    RETURN FOUND;
END $$;

REVOKE ALL ON FUNCTION public.p0c2_claim_job(uuid,bigint),
    public.p0c2_renew_job(uuid,uuid,bigint),
    public.p0c2_checkpoint_job(uuid,uuid,jsonb),
    public.p0c2_succeed_job(uuid,uuid,text),
    public.p0c2_fail_job(uuid,uuid,text,boolean),
    public.p0c2_cancel_job(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.p0c2_claim_job(uuid,bigint),
    public.p0c2_renew_job(uuid,uuid,bigint),
    public.p0c2_checkpoint_job(uuid,uuid,jsonb),
    public.p0c2_succeed_job(uuid,uuid,text),
    public.p0c2_fail_job(uuid,uuid,text,boolean),
    public.p0c2_cancel_job(uuid) TO learning_runtime;
