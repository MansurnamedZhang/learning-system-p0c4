-- Existing v1 rows, digests, receipts, and events retain their original bytes.
ALTER TABLE reading_view_revision ADD COLUMN contract_version integer NOT NULL DEFAULT 1 CHECK(contract_version IN (1,2));
ALTER TABLE reading_view_revision ADD COLUMN evidence jsonb;
ALTER TABLE reading_view_revision ADD CONSTRAINT reading_evidence_version CHECK((contract_version=1 AND evidence IS NULL) OR (contract_version=2 AND evidence IS NOT NULL));
ALTER TABLE request_key DROP CONSTRAINT request_key_operation_check;
ALTER TABLE request_key ADD CONSTRAINT request_key_operation_check CHECK(operation IN
 ('content_v1','composition_save','release_publish','reading_create','reading_edit',
  'migration_propose','migration_decide','content_v2','relation_save','relation_review','epistemic_review','reading_select','release_publish_evidence'));

CREATE TABLE reading_relation_selection (
 view_id uuid NOT NULL,view_revision_id uuid NOT NULL,position integer NOT NULL CHECK(position BETWEEN 0 AND 255),
 relation_id uuid NOT NULL,relation_revision_id uuid NOT NULL,review_id uuid,
 PRIMARY KEY(view_revision_id,position),UNIQUE(view_revision_id,relation_id,relation_revision_id),
 FOREIGN KEY(view_id,view_revision_id) REFERENCES reading_view_revision(view_id,id),
 FOREIGN KEY(relation_id,relation_revision_id) REFERENCES relation_revision(relation_id,id),
 FOREIGN KEY(relation_id,relation_revision_id,review_id) REFERENCES relation_review(relation_id,relation_revision_id,id)
);
CREATE TABLE reading_epistemic_selection (
 view_id uuid NOT NULL,view_revision_id uuid NOT NULL,position integer NOT NULL CHECK(position BETWEEN 0 AND 255),
 stream_id uuid NOT NULL,review_id uuid NOT NULL,
 PRIMARY KEY(view_revision_id,position),UNIQUE(view_revision_id,stream_id,review_id),
 FOREIGN KEY(view_id,view_revision_id) REFERENCES reading_view_revision(view_id,id),
 FOREIGN KEY(stream_id,review_id) REFERENCES epistemic_review(stream_id,id)
);
ALTER TABLE placement_migration ADD COLUMN old_view_revision_id uuid;
ALTER TABLE placement_migration ADD CONSTRAINT migration_fixed_view FOREIGN KEY(overlay_id,old_overlay_revision_id,old_view_revision_id)
 REFERENCES reading_view_revision(overlay_id,overlay_revision_id,id);

CREATE FUNCTION public.b3_check_reading_evidence(view_revision uuid)
RETURNS void LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE v record; selections jsonb; reviews jsonb;
BEGIN
 SELECT * INTO STRICT v FROM public.reading_view_revision WHERE id=view_revision;
 SELECT coalesce(jsonb_agg(jsonb_build_object('relation',jsonb_build_object('relation_id',s.relation_id,'revision_id',s.relation_revision_id),
  'review',CASE WHEN s.review_id IS NULL THEN 'null'::jsonb ELSE jsonb_build_object('relation',jsonb_build_object('relation_id',s.relation_id,'revision_id',s.relation_revision_id),'review_id',s.review_id) END) ORDER BY s.position),'[]')
 INTO selections FROM public.reading_relation_selection s WHERE s.view_revision_id=view_revision;
 SELECT coalesce(jsonb_agg(jsonb_build_object('stream_id',s.stream_id,'review_id',s.review_id) ORDER BY s.position),'[]')
 INTO reviews FROM public.reading_epistemic_selection s WHERE s.view_revision_id=view_revision;
 IF v.contract_version=1 THEN
  IF selections<>'[]'::jsonb OR reviews<>'[]'::jsonb THEN RAISE EXCEPTION 'v1 reading has evidence' USING ERRCODE='23514'; END IF;
 ELSE
  PERFORM public.b3_shape(v.evidence,ARRAY['selections','epistemic_reviews']);
  PERFORM public.b3_selections(v.evidence->'selections');
  IF v.evidence IS DISTINCT FROM jsonb_build_object('selections',selections,'epistemic_reviews',reviews) THEN
   RAISE EXCEPTION 'reading evidence index mismatch' USING ERRCODE='23514';
  END IF;
 END IF;
 IF EXISTS(SELECT 1 FROM public.reading_relation_selection s JOIN public.relation r ON r.id=s.relation_id
   WHERE s.view_revision_id=view_revision AND r.overlay_id IS NOT NULL AND r.overlay_id<>v.overlay_id)
 OR EXISTS(SELECT 1 FROM public.reading_epistemic_selection s JOIN public.epistemic_stream e ON e.id=s.stream_id
   WHERE s.view_revision_id=view_revision AND e.overlay_id IS NOT NULL AND e.overlay_id<>v.overlay_id)
 OR EXISTS(SELECT 1 FROM (SELECT position,row_number() OVER(ORDER BY position)-1 AS expected FROM public.reading_relation_selection WHERE view_revision_id=view_revision) p WHERE position<>expected)
 OR EXISTS(SELECT 1 FROM (SELECT position,row_number() OVER(ORDER BY position)-1 AS expected FROM public.reading_epistemic_selection WHERE view_revision_id=view_revision) p WHERE position<>expected)
 THEN RAISE EXCEPTION 'invalid reading evidence scope or positions' USING ERRCODE='23514'; END IF;
END $$;
CREATE FUNCTION public.b3_reading_evidence_trigger()
RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF TG_TABLE_NAME='reading_view_revision' THEN PERFORM public.b3_check_reading_evidence(NEW.id);
 ELSE PERFORM public.b3_check_reading_evidence(NEW.view_revision_id); END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER reading_evidence_complete AFTER INSERT ON reading_view_revision DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.b3_reading_evidence_trigger();
CREATE CONSTRAINT TRIGGER reading_relation_complete AFTER INSERT ON reading_relation_selection DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.b3_reading_evidence_trigger();
CREATE CONSTRAINT TRIGGER reading_epistemic_complete AFTER INSERT ON reading_epistemic_selection DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.b3_reading_evidence_trigger();

ALTER TABLE release ADD COLUMN contract_version integer NOT NULL DEFAULT 1 CHECK(contract_version IN (1,2));
ALTER TABLE release ADD COLUMN manifest_sha256 text;
ALTER TABLE release ADD CONSTRAINT release_manifest_version CHECK((contract_version=1 AND manifest_sha256 IS NULL) OR (contract_version=2 AND manifest_sha256 IS NOT NULL AND manifest_sha256 ~ '^[0-9a-f]{64}$'));
CREATE TABLE release_reading (
 release_id uuid NOT NULL REFERENCES release(id),view_id uuid NOT NULL,view_revision_id uuid NOT NULL,
 overlay_id uuid NOT NULL,overlay_revision_id uuid NOT NULL,
 PRIMARY KEY(release_id,view_id),
 FOREIGN KEY(view_id,overlay_id,overlay_revision_id,view_revision_id) REFERENCES reading_view_revision(view_id,overlay_id,overlay_revision_id,id)
);
CREATE TABLE release_manifest_object (
 release_id uuid NOT NULL REFERENCES release(id),kind text NOT NULL,object_id uuid NOT NULL,revision_id uuid NOT NULL,
 PRIMARY KEY(release_id,kind,object_id,revision_id),
 FOREIGN KEY(kind,object_id,revision_id) REFERENCES reference_object(kind,object_id,revision_id)
);
CREATE TABLE release_manifest_composition (
 release_id uuid NOT NULL REFERENCES release(id),space_id uuid NOT NULL,composition_id uuid NOT NULL,revision_id uuid NOT NULL,
 PRIMARY KEY(release_id,composition_id,revision_id),
 FOREIGN KEY(space_id,composition_id,revision_id) REFERENCES composition_revision(space_id,composition_id,id)
);
ALTER TABLE outbox_event DROP CONSTRAINT outbox_event_payload_version_check;
ALTER TABLE outbox_event ADD CONSTRAINT outbox_event_payload_version_check CHECK(payload_version IN (1,2));
GRANT SELECT,INSERT ON reading_relation_selection,reading_epistemic_selection,release_reading,release_manifest_object,release_manifest_composition TO learning_runtime;
REVOKE ALL ON FUNCTION public.b3_check_reading_evidence(uuid),public.b3_reading_evidence_trigger() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.b3_check_reading_evidence(uuid),public.b3_reading_evidence_trigger() TO learning_runtime;

-- Canonical JSON agrees with the core serializer: sorted object keys, no whitespace.
CREATE FUNCTION public.b3_canonical_json(value jsonb)
RETURNS text LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE result text;
BEGIN
 CASE jsonb_typeof(value)
 WHEN 'object' THEN SELECT '{'||coalesce(string_agg(to_jsonb(key)::text||':'||public.b3_canonical_json(v),',' ORDER BY key COLLATE "C"),'')||'}' INTO result FROM jsonb_each(value) AS e(key,v);
 WHEN 'array' THEN SELECT '['||coalesce(string_agg(public.b3_canonical_json(v),',' ORDER BY n),'')||']' INTO result FROM jsonb_array_elements(value) WITH ORDINALITY AS e(v,n);
 ELSE result:=value::text;
 END CASE;
 RETURN result;
END $$;
CREATE FUNCTION public.b3_release_manifest(released uuid)
RETURNS jsonb LANGUAGE sql STABLE SET search_path=pg_catalog,public,pg_temp AS $$
WITH RECURSIVE
 chosen AS (SELECT * FROM public.release_reading WHERE release_id=released),
 seeds(composition_id,revision_id) AS (
  SELECT composition_id,revision_id FROM public.release_root WHERE release_id=released
  UNION SELECT r.root_composition_id,r.base_revision_id FROM chosen c JOIN public.overlay_revision r ON r.overlay_id=c.overlay_id AND r.id=c.overlay_revision_id
  UNION SELECT g.root_composition_id,g.base_revision_id FROM chosen c JOIN public.overlay_group g ON g.overlay_id=c.overlay_id AND g.overlay_revision_id=c.overlay_revision_id
 ),
 compositions(composition_id,revision_id) AS (
  SELECT composition_id,revision_id FROM seeds
  UNION SELECT o.child_composition_id,o.child_revision_id FROM compositions c JOIN public.composition_occurrence o ON o.composition_id=c.composition_id AND o.composition_revision_id=c.revision_id WHERE o.child_composition_id IS NOT NULL
 ),
 object_seeds(kind,object_id,revision_id) AS (
  SELECT 'block'::text,o.block_id,o.block_revision_id FROM compositions c JOIN public.composition_occurrence o ON o.composition_id=c.composition_id AND o.composition_revision_id=c.revision_id WHERE o.block_id IS NOT NULL
  UNION SELECT 'block',p.block_id,p.block_revision_id FROM chosen c JOIN public.overlay_placement p ON p.overlay_id=c.overlay_id AND p.overlay_revision_id=c.overlay_revision_id
  UNION SELECT 'relation',s.relation_id,s.relation_revision_id FROM chosen c JOIN public.reading_relation_selection s ON s.view_id=c.view_id AND s.view_revision_id=c.view_revision_id
  UNION SELECT 'relation_review',s.relation_revision_id,s.review_id FROM chosen c JOIN public.reading_relation_selection s ON s.view_id=c.view_id AND s.view_revision_id=c.view_revision_id WHERE s.review_id IS NOT NULL
  UNION SELECT 'epistemic_review',s.stream_id,s.review_id FROM chosen c JOIN public.reading_epistemic_selection s ON s.view_id=c.view_id AND s.view_revision_id=c.view_revision_id
 ),
 objects(kind,object_id,revision_id) AS (
  SELECT kind,object_id,revision_id FROM object_seeds
  UNION SELECT d.target_kind,d.target_object_id,d.target_revision_id FROM objects o JOIN public.reference_dependency d ON d.source_kind=o.kind AND d.source_object_id=o.object_id AND d.source_revision_id=o.revision_id
 )
SELECT jsonb_build_object('domain','release-manifest-v2',
 'roots',(SELECT coalesce(jsonb_agg(jsonb_build_object('composition_id',composition_id,'revision_id',revision_id) ORDER BY composition_id,revision_id),'[]') FROM public.release_root WHERE release_id=released),
 'readings',(SELECT coalesce(jsonb_agg(jsonb_build_object('view_id',view_id,'revision_id',view_revision_id) ORDER BY view_id,view_revision_id),'[]') FROM chosen),
 'compositions',(SELECT coalesce(jsonb_agg(jsonb_build_object('composition_id',composition_id,'revision_id',revision_id) ORDER BY composition_id,revision_id),'[]') FROM compositions),
 'objects',(SELECT coalesce(jsonb_agg(jsonb_build_object('kind',kind,'object_id',object_id,'revision_id',revision_id) ORDER BY kind COLLATE "C",object_id,revision_id),'[]') FROM objects))
$$;
CREATE FUNCTION public.b3_check_release_evidence(released uuid)
RETURNS void LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE r record; expected jsonb; actual_compositions jsonb; actual_objects jsonb; n integer;
BEGIN
 SELECT * INTO STRICT r FROM public.release WHERE id=released;
 IF r.contract_version=1 THEN
  IF EXISTS(SELECT 1 FROM public.release_reading WHERE release_id=released)
   OR EXISTS(SELECT 1 FROM public.release_manifest_object WHERE release_id=released)
   OR EXISTS(SELECT 1 FROM public.release_manifest_composition WHERE release_id=released)
   THEN RAISE EXCEPTION 'v1 release has evidence' USING ERRCODE='23514'; END IF;
  IF EXISTS(
   WITH RECURSIVE compositions(composition_id,revision_id) AS (
    SELECT composition_id,revision_id FROM public.release_root WHERE release_id=released
    UNION SELECT o.child_composition_id,o.child_revision_id FROM compositions c
     JOIN public.composition_occurrence o ON o.composition_id=c.composition_id AND o.composition_revision_id=c.revision_id
     WHERE o.child_composition_id IS NOT NULL
   )
   SELECT 1 FROM compositions c
    JOIN public.composition_occurrence o ON o.composition_id=c.composition_id AND o.composition_revision_id=c.revision_id
    JOIN public.block_revision b ON b.block_id=o.block_id AND b.id=o.block_revision_id
    WHERE b.contract_version<>1
  ) THEN RAISE EXCEPTION 'v2 content requires an evidence release' USING ERRCODE='23514'; END IF;
 ELSE
  SELECT count(*) INTO n FROM public.release_root WHERE release_id=released;
  IF n NOT BETWEEN 1 AND 16 OR (SELECT count(*) FROM public.release_reading WHERE release_id=released)>16
   THEN RAISE EXCEPTION 'invalid release size' USING ERRCODE='23514'; END IF;
  IF EXISTS(SELECT 1 FROM public.release_reading c JOIN public.overlay o ON o.id=c.overlay_id WHERE c.release_id=released AND o.owner_id<>r.author_id)
   THEN RAISE EXCEPTION 'release reading owner mismatch' USING ERRCODE='23514'; END IF;
  expected:=public.b3_release_manifest(released);
  SELECT coalesce(jsonb_agg(jsonb_build_object('composition_id',composition_id,'revision_id',revision_id) ORDER BY composition_id,revision_id),'[]') INTO actual_compositions FROM public.release_manifest_composition WHERE release_id=released;
  SELECT coalesce(jsonb_agg(jsonb_build_object('kind',kind,'object_id',object_id,'revision_id',revision_id) ORDER BY kind COLLATE "C",object_id,revision_id),'[]') INTO actual_objects FROM public.release_manifest_object WHERE release_id=released;
  IF actual_compositions IS DISTINCT FROM expected->'compositions' OR actual_objects IS DISTINCT FROM expected->'objects'
   OR r.manifest_sha256<>encode(sha256(convert_to(public.b3_canonical_json(expected),'UTF8')),'hex')
   THEN RAISE EXCEPTION 'incomplete or altered release manifest' USING ERRCODE='23514'; END IF;
  IF NOT EXISTS(SELECT 1 FROM public.outbox_event WHERE aggregate_id=released AND payload_version=2)
   THEN RAISE EXCEPTION 'missing versioned release event' USING ERRCODE='23514'; END IF;
 END IF;
 IF EXISTS(SELECT 1 FROM public.outbox_event WHERE aggregate_id=released AND payload_version<>r.contract_version)
  THEN RAISE EXCEPTION 'release event version mismatch' USING ERRCODE='23514'; END IF;
END $$;
CREATE FUNCTION public.b3_release_evidence_trigger()
RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF TG_TABLE_NAME='release' THEN PERFORM public.b3_check_release_evidence(NEW.id);
 ELSIF TG_TABLE_NAME='outbox_event' THEN PERFORM public.b3_check_release_evidence(NEW.aggregate_id);
 ELSE PERFORM public.b3_check_release_evidence(NEW.release_id); END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER release_evidence_complete AFTER INSERT ON release DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.b3_release_evidence_trigger();
CREATE CONSTRAINT TRIGGER release_root_complete AFTER INSERT ON release_root DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.b3_release_evidence_trigger();
CREATE CONSTRAINT TRIGGER release_reading_complete AFTER INSERT ON release_reading DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.b3_release_evidence_trigger();
CREATE CONSTRAINT TRIGGER release_objects_complete AFTER INSERT ON release_manifest_object DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.b3_release_evidence_trigger();
CREATE CONSTRAINT TRIGGER release_compositions_complete AFTER INSERT ON release_manifest_composition DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.b3_release_evidence_trigger();
CREATE CONSTRAINT TRIGGER release_event_complete AFTER INSERT ON outbox_event DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.b3_release_evidence_trigger();
REVOKE ALL ON FUNCTION public.b3_canonical_json(jsonb),public.b3_release_manifest(uuid),public.b3_check_release_evidence(uuid),public.b3_release_evidence_trigger() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.b3_canonical_json(jsonb),public.b3_release_manifest(uuid),public.b3_check_release_evidence(uuid),public.b3_release_evidence_trigger() TO learning_runtime;
