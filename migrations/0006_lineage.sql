-- Provenance is an immutable content operation, outside user relation identity
-- and outside the necessary semantic dependency index.
ALTER TABLE request_key DROP CONSTRAINT request_key_operation_check;
ALTER TABLE request_key ADD CONSTRAINT request_key_operation_check CHECK(operation IN
 ('content_v1','composition_save','release_publish','reading_create','reading_edit',
  'migration_propose','migration_decide','content_v2','relation_save','relation_review','epistemic_review',
  'reading_select','release_publish_evidence','lineage_apply'));

CREATE TABLE lineage_operation (
 id uuid PRIMARY KEY,
 space_id uuid NOT NULL REFERENCES space(id),
 operation text NOT NULL CHECK(operation IN ('derive','split','merge')),
 author_id uuid NOT NULL REFERENCES app_user(id),
 reason text NOT NULL CHECK(char_length(reason) BETWEEN 1 AND 1000 AND btrim(reason)<>''),
 input_count integer NOT NULL, output_count integer NOT NULL,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(space_id,id), UNIQUE(id,author_id),
 CHECK((operation='derive' AND input_count=1 AND output_count=1)
    OR (operation='split' AND input_count=1 AND output_count BETWEEN 2 AND 32)
    OR (operation='merge' AND input_count BETWEEN 2 AND 32 AND output_count=1))
);
CREATE TABLE lineage_input (
 operation_id uuid NOT NULL REFERENCES lineage_operation(id),
 position integer NOT NULL CHECK(position BETWEEN 0 AND 31),
 space_id uuid NOT NULL, block_id uuid NOT NULL, revision_id uuid NOT NULL,
 PRIMARY KEY(operation_id,position), UNIQUE(operation_id,block_id),
 FOREIGN KEY(space_id,block_id,revision_id) REFERENCES block_revision(space_id,block_id,id)
);
CREATE TABLE lineage_output (
 operation_id uuid NOT NULL,
 position integer NOT NULL CHECK(position BETWEEN 0 AND 31),
 space_id uuid NOT NULL, block_id uuid NOT NULL UNIQUE, revision_id uuid NOT NULL UNIQUE,
 PRIMARY KEY(operation_id,position),
 FOREIGN KEY(space_id,operation_id) REFERENCES lineage_operation(space_id,id),
 FOREIGN KEY(space_id,block_id,revision_id) REFERENCES block_revision(space_id,block_id,id)
);
CREATE TABLE lineage_receipt (
 actor_id uuid NOT NULL, request_id uuid NOT NULL,
 operation_id uuid NOT NULL UNIQUE,
 request_sha256 text NOT NULL CHECK(request_sha256 ~ '^[0-9a-f]{64}$'),
 PRIMARY KEY(actor_id,request_id),
 FOREIGN KEY(actor_id,request_id) REFERENCES request_key(actor_id,request_id),
 FOREIGN KEY(operation_id,actor_id) REFERENCES lineage_operation(id,author_id)
);

-- A runtime cannot attach an old identity/revision to a new transformation.
-- xmin is examined only during INSERT in the creating transaction; it is never
-- persisted or used as long-lived identity, so vacuum freezing has no effect.
CREATE FUNCTION public.b3_lineage_new_member()
RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE current_xid text := (pg_current_xact_id()::text::numeric % 4294967296)::text;
BEGIN
 IF NOT EXISTS(SELECT 1 FROM public.lineage_operation o WHERE o.id=NEW.operation_id AND o.xmin::text=current_xid)
 THEN RAISE EXCEPTION 'lineage operation must be created atomically' USING ERRCODE='23514'; END IF;
 IF TG_TABLE_NAME='lineage_output' THEN
  IF NOT EXISTS(SELECT 1 FROM public.block b JOIN public.block_revision r ON r.block_id=b.id AND r.id=b.head_revision_id
    JOIN public.lineage_operation o ON o.id=NEW.operation_id
    WHERE b.id=NEW.block_id AND r.id=NEW.revision_id AND b.space_id=NEW.space_id
      AND b.xmin::text=current_xid AND r.xmin::text=current_xid
      AND r.parent_revision_id IS NULL AND r.author_id=o.author_id AND r.reason=o.reason
      AND NOT EXISTS(SELECT 1 FROM public.block_revision prior_revision WHERE prior_revision.block_id=b.id AND prior_revision.id<>r.id))
  THEN RAISE EXCEPTION 'lineage output must be new owned content' USING ERRCODE='23514'; END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER lineage_input_new BEFORE INSERT ON lineage_input FOR EACH ROW EXECUTE FUNCTION public.b3_lineage_new_member();
CREATE TRIGGER lineage_output_new BEFORE INSERT ON lineage_output FOR EACH ROW EXECUTE FUNCTION public.b3_lineage_new_member();
CREATE TRIGGER lineage_receipt_new BEFORE INSERT ON lineage_receipt FOR EACH ROW EXECUTE FUNCTION public.b3_lineage_new_member();

CREATE FUNCTION public.b3_lineage_complete()
RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE op uuid; o record; n integer; last_position integer;
BEGIN
 IF TG_TABLE_NAME='lineage_operation' THEN op:=NEW.id; ELSE op:=NEW.operation_id; END IF;
 SELECT * INTO STRICT o FROM public.lineage_operation WHERE id=op;
 SELECT count(*),max(position) INTO n,last_position FROM public.lineage_input WHERE operation_id=op;
 IF n<>o.input_count OR last_position<>n-1 THEN
  RAISE EXCEPTION 'incomplete lineage inputs' USING ERRCODE='23514'; END IF;
 SELECT count(*),max(position) INTO n,last_position FROM public.lineage_output WHERE operation_id=op;
 IF n<>o.output_count OR last_position<>n-1 THEN
  RAISE EXCEPTION 'incomplete lineage outputs' USING ERRCODE='23514'; END IF;
 IF EXISTS(SELECT 1 FROM public.lineage_input i JOIN public.lineage_output r USING(operation_id,block_id) WHERE i.operation_id=op)
 THEN RAISE EXCEPTION 'lineage output reuses input identity' USING ERRCODE='23514'; END IF;
 IF NOT EXISTS(SELECT 1 FROM public.lineage_receipt r JOIN public.request_key k USING(actor_id,request_id)
   WHERE r.operation_id=op AND r.actor_id=o.author_id AND k.operation='lineage_apply' AND k.request_sha256=r.request_sha256)
 THEN RAISE EXCEPTION 'missing or mismatched lineage receipt' USING ERRCODE='23514'; END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER lineage_operation_complete AFTER INSERT ON lineage_operation DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.b3_lineage_complete();
CREATE CONSTRAINT TRIGGER lineage_input_complete AFTER INSERT ON lineage_input DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.b3_lineage_complete();
CREATE CONSTRAINT TRIGGER lineage_output_complete AFTER INSERT ON lineage_output DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.b3_lineage_complete();
CREATE CONSTRAINT TRIGGER lineage_receipt_complete AFTER INSERT ON lineage_receipt DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.b3_lineage_complete();

-- This join view is not updatable; only complete operations can be committed.
-- The trusted service gates its projection on all input/output closures.
CREATE VIEW system_lineage AS
 SELECT o.id AS operation_id,
 CASE o.operation WHEN 'derive' THEN 'derived_from' WHEN 'split' THEN 'split_from' ELSE 'merged_from' END AS type,
 r.block_id AS from_block_id,r.revision_id AS from_revision_id,
 i.block_id AS to_block_id,i.revision_id AS to_revision_id,
 r.position AS output_position,i.position AS input_position
 FROM lineage_operation o JOIN lineage_input i ON i.operation_id=o.id JOIN lineage_output r ON r.operation_id=o.id;
GRANT SELECT,INSERT ON lineage_operation,lineage_input,lineage_output,lineage_receipt TO learning_runtime;
GRANT SELECT ON system_lineage TO learning_runtime;
REVOKE ALL ON FUNCTION public.b3_lineage_new_member(),public.b3_lineage_complete() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.b3_lineage_new_member(),public.b3_lineage_complete() TO learning_runtime;
