-- Dedicated success receipts; normal write receipts and jobs are untouched.
CREATE TABLE public.snapshot_import_batch (
 actor_id uuid NOT NULL REFERENCES public.app_user(id),
 request_id uuid NOT NULL,
 manifest_sha256 text NOT NULL CHECK(manifest_sha256 ~ '^[0-9a-f]{64}$'),
 status text NOT NULL DEFAULT 'succeeded' CHECK(status='succeeded'),
 PRIMARY KEY(actor_id,request_id)
);
CREATE INDEX snapshot_import_actor_manifest ON public.snapshot_import_batch(actor_id,manifest_sha256);
REVOKE ALL ON public.snapshot_import_batch FROM PUBLIC;
GRANT SELECT,INSERT ON public.snapshot_import_batch TO learning_runtime;
