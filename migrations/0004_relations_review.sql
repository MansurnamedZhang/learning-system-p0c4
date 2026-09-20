-- Exact immutable references. Registration is automatic so old v1 writers remain valid.
ALTER TABLE block_revision ADD COLUMN contract_version integer NOT NULL DEFAULT 1
 CHECK(contract_version IN (1,2));
ALTER TABLE request_key DROP CONSTRAINT request_key_operation_check;
ALTER TABLE request_key ADD CONSTRAINT request_key_operation_check CHECK(operation IN
 ('content_v1','composition_save','release_publish','reading_create','reading_edit',
  'migration_propose','migration_decide','content_v2','relation_save','relation_review','epistemic_review'));

CREATE TABLE reference_object (
 kind text NOT NULL CHECK(kind IN ('block','relation','relation_review','epistemic_review')),
 object_id uuid NOT NULL, revision_id uuid NOT NULL, space_id uuid NOT NULL REFERENCES space(id),
 PRIMARY KEY(kind,object_id,revision_id)
);
CREATE TABLE reference_dependency (
 source_kind text NOT NULL, source_object_id uuid NOT NULL, source_revision_id uuid NOT NULL,
 position integer NOT NULL CHECK(position>=0),
 role text NOT NULL CHECK(role IN ('basis','target','requires_context','source_run','selected_relation','selected_review')),
 target_kind text NOT NULL, target_object_id uuid NOT NULL, target_revision_id uuid NOT NULL,
 PRIMARY KEY(source_kind,source_object_id,source_revision_id,position),
 FOREIGN KEY(source_kind,source_object_id,source_revision_id)
  REFERENCES reference_object(kind,object_id,revision_id) DEFERRABLE INITIALLY DEFERRED,
 FOREIGN KEY(target_kind,target_object_id,target_revision_id)
  REFERENCES reference_object(kind,object_id,revision_id) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX reference_dependency_target ON reference_dependency(target_kind,target_object_id,target_revision_id);

CREATE TABLE relation (
 id uuid PRIMARY KEY, space_id uuid NOT NULL REFERENCES space(id), overlay_id uuid,
 type text NOT NULL CHECK(type IN ('annotates','questions','answers','inspired_by','supports','opposes','tests','related_to')),
 origin text NOT NULL DEFAULT 'user_asserted' CHECK(origin='user_asserted'),
 from_block_id uuid NOT NULL REFERENCES block(id), to_block_id uuid NOT NULL REFERENCES block(id),
 head_revision_id uuid NOT NULL, created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(space_id,id), UNIQUE(space_id,id,from_block_id,to_block_id),
 UNIQUE NULLS NOT DISTINCT(space_id,overlay_id,type,from_block_id,to_block_id),
 FOREIGN KEY(space_id,overlay_id) REFERENCES overlay(space_id,id) DEFERRABLE INITIALLY DEFERRED,
 CHECK(from_block_id<>to_block_id), CHECK(type<>'related_to' OR from_block_id<to_block_id)
);
CREATE TABLE relation_revision (
 id uuid PRIMARY KEY, space_id uuid NOT NULL, relation_id uuid NOT NULL, parent_revision_id uuid,
 from_space_id uuid NOT NULL, from_block_id uuid NOT NULL, from_revision_id uuid NOT NULL,
 to_space_id uuid NOT NULL, to_block_id uuid NOT NULL, to_revision_id uuid NOT NULL,
 rationale text NOT NULL, conditions text NOT NULL CHECK(octet_length(conditions)<=10000),
 content_sha256 text NOT NULL CHECK(content_sha256 ~ '^[0-9a-f]{64}$'),
 author_id uuid NOT NULL REFERENCES app_user(id), created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(space_id,relation_id,id), UNIQUE(relation_id,id),
 CHECK(parent_revision_id IS NULL OR parent_revision_id<>id),
 FOREIGN KEY(space_id,relation_id,from_block_id,to_block_id)
  REFERENCES relation(space_id,id,from_block_id,to_block_id) DEFERRABLE INITIALLY DEFERRED,
 FOREIGN KEY(space_id,relation_id,parent_revision_id)
  REFERENCES relation_revision(space_id,relation_id,id) DEFERRABLE INITIALLY DEFERRED,
 FOREIGN KEY(from_space_id,from_block_id,from_revision_id)
  REFERENCES block_revision(space_id,block_id,id) DEFERRABLE INITIALLY DEFERRED,
 FOREIGN KEY(to_space_id,to_block_id,to_revision_id)
  REFERENCES block_revision(space_id,block_id,id) DEFERRABLE INITIALLY DEFERRED
);
ALTER TABLE relation ADD CONSTRAINT relation_head_same_object FOREIGN KEY(space_id,id,head_revision_id)
 REFERENCES relation_revision(space_id,relation_id,id) DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE relation_review_head (
 relation_id uuid NOT NULL, relation_revision_id uuid NOT NULL, head_review_id uuid NOT NULL,
 PRIMARY KEY(relation_id,relation_revision_id),
 FOREIGN KEY(relation_id,relation_revision_id) REFERENCES relation_revision(relation_id,id) DEFERRABLE INITIALLY DEFERRED
);
CREATE TABLE relation_review (
 id uuid PRIMARY KEY, space_id uuid NOT NULL, relation_id uuid NOT NULL, relation_revision_id uuid NOT NULL,
 previous_review_id uuid, state text NOT NULL CHECK(state IN ('unreviewed','reviewed','needs_recheck','withdrawn')),
 explanation text NOT NULL CHECK(octet_length(explanation)<=10000),
 reviewer_id uuid NOT NULL REFERENCES app_user(id), created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(relation_id,relation_revision_id,id), CHECK(previous_review_id IS NULL OR previous_review_id<>id),
 FOREIGN KEY(space_id,relation_id,relation_revision_id)
  REFERENCES relation_revision(space_id,relation_id,id) DEFERRABLE INITIALLY DEFERRED,
 FOREIGN KEY(relation_id,relation_revision_id)
  REFERENCES relation_review_head(relation_id,relation_revision_id) DEFERRABLE INITIALLY DEFERRED,
 FOREIGN KEY(relation_id,relation_revision_id,previous_review_id)
  REFERENCES relation_review(relation_id,relation_revision_id,id) DEFERRABLE INITIALLY DEFERRED
);
ALTER TABLE relation_review_head ADD CONSTRAINT relation_review_head_same_stream
 FOREIGN KEY(relation_id,relation_revision_id,head_review_id)
 REFERENCES relation_review(relation_id,relation_revision_id,id) DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE epistemic_stream (
 id uuid PRIMARY KEY, space_id uuid NOT NULL REFERENCES space(id), overlay_id uuid,
 target_space_id uuid NOT NULL, target_block_id uuid NOT NULL, target_revision_id uuid NOT NULL,
 actor_id uuid NOT NULL REFERENCES app_user(id), head_review_id uuid NOT NULL,
 UNIQUE(space_id,id,actor_id),
 UNIQUE NULLS NOT DISTINCT(space_id,overlay_id,target_block_id,target_revision_id,actor_id),
 FOREIGN KEY(space_id,overlay_id) REFERENCES overlay(space_id,id) DEFERRABLE INITIALLY DEFERRED,
 FOREIGN KEY(target_space_id,target_block_id,target_revision_id)
  REFERENCES block_revision(space_id,block_id,id) DEFERRABLE INITIALLY DEFERRED
);
CREATE TABLE epistemic_review (
 id uuid PRIMARY KEY, space_id uuid NOT NULL, stream_id uuid NOT NULL, previous_review_id uuid,
 state text NOT NULL CHECK(state IN ('untested','testing','inconclusive','supported_within_scope','refuted_within_scope','superseded')),
 relations jsonb NOT NULL, evidence jsonb NOT NULL,
 conditions text NOT NULL CHECK(octet_length(conditions)<=10000),
 explanation text NOT NULL CHECK(octet_length(explanation)<=10000),
 reviewer_id uuid NOT NULL REFERENCES app_user(id), created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(stream_id,id), CHECK(previous_review_id IS NULL OR previous_review_id<>id),
 FOREIGN KEY(space_id,stream_id,reviewer_id) REFERENCES epistemic_stream(space_id,id,actor_id) DEFERRABLE INITIALLY DEFERRED,
 FOREIGN KEY(stream_id,previous_review_id) REFERENCES epistemic_review(stream_id,id) DEFERRABLE INITIALLY DEFERRED
);
ALTER TABLE epistemic_stream ADD CONSTRAINT epistemic_head_same_stream FOREIGN KEY(id,head_review_id)
 REFERENCES epistemic_review(stream_id,id) DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE relation_receipt (
 actor_id uuid NOT NULL, request_id uuid NOT NULL, revision_id uuid NOT NULL REFERENCES relation_revision(id),
 PRIMARY KEY(actor_id,request_id), FOREIGN KEY(actor_id,request_id) REFERENCES request_key(actor_id,request_id)
);
CREATE TABLE relation_review_receipt (
 actor_id uuid NOT NULL, request_id uuid NOT NULL, review_id uuid NOT NULL REFERENCES relation_review(id),
 PRIMARY KEY(actor_id,request_id), FOREIGN KEY(actor_id,request_id) REFERENCES request_key(actor_id,request_id)
);
CREATE TABLE epistemic_review_receipt (
 actor_id uuid NOT NULL, request_id uuid NOT NULL, review_id uuid NOT NULL REFERENCES epistemic_review(id),
 PRIMARY KEY(actor_id,request_id), FOREIGN KEY(actor_id,request_id) REFERENCES request_key(actor_id,request_id)
);
CREATE INDEX relation_history ON relation_revision(relation_id,created_at,id);
CREATE INDEX relation_review_history ON relation_review(relation_id,relation_revision_id,created_at,id);
CREATE INDEX epistemic_review_history ON epistemic_review(stream_id,created_at,id);

-- Every JSON decoder rejects missing, mistyped, and unknown fields. Helpers are
-- invoker-rights and all table/function references are schema-qualified.
CREATE FUNCTION public.b3_shape(value jsonb, required text[], optional text[] DEFAULT ARRAY[]::text[])
RETURNS void LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF jsonb_typeof(value) IS DISTINCT FROM 'object' THEN
  RAISE EXCEPTION 'invalid structured reference payload' USING ERRCODE='23514';
 END IF;
 IF NOT value ?& required
    OR EXISTS(SELECT 1 FROM jsonb_object_keys(value) k WHERE NOT k=ANY(required||optional)) THEN
  RAISE EXCEPTION 'invalid structured reference payload' USING ERRCODE='23514';
 END IF;
END $$;

-- Converts a strict wire reference to its registry key, checking the nested
-- relation identity which is intentionally not part of a review's registry key.
CREATE FUNCTION public.b3_reference(value jsonb, kind text)
RETURNS jsonb LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE object_id uuid; revision_id uuid; expected_relation_id uuid; object_field text; revision_field text;
BEGIN
 CASE kind
 WHEN 'block' THEN
  PERFORM public.b3_shape(value,ARRAY['block_id','revision_id']);
  object_field:='block_id'; revision_field:='revision_id';
  object_id:=(value->>'block_id')::uuid; revision_id:=(value->>'revision_id')::uuid;
 WHEN 'relation' THEN
  PERFORM public.b3_shape(value,ARRAY['relation_id','revision_id']);
  object_field:='relation_id'; revision_field:='revision_id';
  object_id:=(value->>'relation_id')::uuid; revision_id:=(value->>'revision_id')::uuid;
 WHEN 'relation_review' THEN
  PERFORM public.b3_shape(value,ARRAY['relation','review_id']);
  revision_field:='review_id';
  PERFORM public.b3_reference(value->'relation','relation');
  object_id:=(value->'relation'->>'revision_id')::uuid; revision_id:=(value->>'review_id')::uuid;
  expected_relation_id:=(value->'relation'->>'relation_id')::uuid;
  IF NOT EXISTS(SELECT 1 FROM public.relation_review r WHERE r.id=revision_id
    AND r.relation_revision_id=object_id AND r.relation_id=expected_relation_id) THEN
   RAISE EXCEPTION 'review reference identity mismatch' USING ERRCODE='23514';
  END IF;
 WHEN 'epistemic_review' THEN
  PERFORM public.b3_shape(value,ARRAY['stream_id','review_id']);
  object_field:='stream_id'; revision_field:='review_id';
  object_id:=(value->>'stream_id')::uuid; revision_id:=(value->>'review_id')::uuid;
 ELSE RAISE EXCEPTION 'invalid reference kind' USING ERRCODE='23514';
 END CASE;
 IF object_id IS NULL OR revision_id IS NULL
    OR jsonb_typeof(value->revision_field) IS DISTINCT FROM 'string'
    OR (object_field IS NOT NULL AND jsonb_typeof(value->object_field) IS DISTINCT FROM 'string') THEN
  RAISE EXCEPTION 'null reference identity' USING ERRCODE='23514';
 END IF;
 RETURN jsonb_build_object('kind',kind,'object_id',object_id,'revision_id',revision_id);
EXCEPTION WHEN invalid_text_representation THEN
 RAISE EXCEPTION 'invalid reference identity' USING ERRCODE='23514';
END $$;

CREATE FUNCTION public.b3_edge(role text, target jsonb)
RETURNS jsonb LANGUAGE sql IMMUTABLE SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT jsonb_build_array(jsonb_build_object('role',role,'target',target))
$$;

CREATE FUNCTION public.b3_selections(value jsonb)
RETURNS jsonb LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE result jsonb:='[]'; selection jsonb; seen jsonb:='[]';
BEGIN
 IF jsonb_typeof(value) IS DISTINCT FROM 'array' OR jsonb_array_length(value)>256 THEN
  RAISE EXCEPTION 'invalid relation selections' USING ERRCODE='23514';
 END IF;
 FOR selection IN SELECT jsonb_array_elements(value) LOOP
  PERFORM public.b3_shape(selection,ARRAY['relation'],ARRAY['review']);
  IF seen @> jsonb_build_array(selection->'relation') THEN
   RAISE EXCEPTION 'duplicate relation selection' USING ERRCODE='23514';
  END IF;
  seen:=seen||jsonb_build_array(selection->'relation');
  result:=result||public.b3_edge('selected_relation',public.b3_reference(selection->'relation','relation'));
  IF selection->'review' IS NOT NULL AND selection->'review'<>'null'::jsonb THEN
   IF selection->'review'->'relation' IS DISTINCT FROM selection->'relation' THEN
    RAISE EXCEPTION 'selection review mismatch' USING ERRCODE='23514';
   END IF;
   result:=result||public.b3_edge('selected_review',public.b3_reference(selection->'review','relation_review'));
  END IF;
 END LOOP;
 RETURN result;
END $$;

CREATE FUNCTION public.b3_content_dependencies(version integer, value jsonb)
RETURNS jsonb LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE result jsonb:='[]'; reference jsonb; body jsonb; payload jsonb; seen jsonb:='[]';
BEGIN
 IF version=1 THEN
  PERFORM public.b3_shape(value,ARRAY['kind','intent','language','title','payload']);
  IF value->>'kind' IS DISTINCT FROM 'text' THEN
   RAISE EXCEPTION 'invalid v1 body' USING ERRCODE='23514';
  END IF;
  payload:=value->'payload';
 ELSIF version=2 THEN
  PERFORM public.b3_shape(value,ARRAY['intent','language','title','body','basis_refs','requires_context'],ARRAY['source_run']);
  IF jsonb_typeof(value->'basis_refs') IS DISTINCT FROM 'array'
    OR jsonb_typeof(value->'requires_context') IS DISTINCT FROM 'array' THEN
   RAISE EXCEPTION 'invalid dependencies' USING ERRCODE='23514';
  END IF;
  FOR reference IN SELECT jsonb_array_elements(value->'basis_refs') LOOP
   IF jsonb_typeof(reference) IS DISTINCT FROM 'object' OR jsonb_typeof(reference->'type') IS DISTINCT FROM 'string' THEN
    RAISE EXCEPTION 'invalid exact reference' USING ERRCODE='23514';
   END IF;
   IF seen @> jsonb_build_array(reference) THEN
    RAISE EXCEPTION 'duplicate basis reference' USING ERRCODE='23514';
   END IF;
   seen:=seen||jsonb_build_array(reference);
   result:=result||public.b3_edge('basis',public.b3_reference(reference-'type',reference->>'type'));
  END LOOP;
  FOR reference IN SELECT jsonb_array_elements(value->'requires_context') LOOP
   result:=result||public.b3_edge('requires_context',public.b3_reference(reference,'block'));
  END LOOP;
  IF value->'source_run' IS NOT NULL AND value->'source_run'<>'null'::jsonb THEN
   result:=result||public.b3_edge('source_run',public.b3_reference(value->'source_run','block'));
  END IF;
  body:=value->'body';
  CASE body->>'kind'
  WHEN 'text' THEN
   PERFORM public.b3_shape(body,ARRAY['kind','payload']); payload:=body->'payload';
  WHEN 'reference' THEN
   PERFORM public.b3_shape(body,ARRAY['kind','target']);
   result:=result||public.b3_edge('target',public.b3_reference(body->'target','block'));
  WHEN 'relation_view' THEN
   PERFORM public.b3_shape(body,ARRAY['kind','selections']);
   result:=result||public.b3_selections(body->'selections');
  ELSE RAISE EXCEPTION 'invalid v2 body' USING ERRCODE='23514';
  END CASE;
 ELSE RAISE EXCEPTION 'invalid content version' USING ERRCODE='23514';
 END IF;
 IF jsonb_typeof(value->'intent') IS DISTINCT FROM 'string'
    OR NOT (value->>'intent'=ANY(ARRAY['knowledge','note','question','idea','conjecture','observation','evidence','conclusion']))
    OR jsonb_typeof(value->'language') IS DISTINCT FROM 'string'
    OR char_length(value->>'language') NOT BETWEEN 1 AND 35
    OR octet_length(value->>'language')<>char_length(value->>'language')
    OR jsonb_typeof(value->'title') IS DISTINCT FROM 'string' OR char_length(value->>'title')>300 THEN
  RAISE EXCEPTION 'invalid content metadata' USING ERRCODE='23514';
 END IF;
 IF payload IS NOT NULL THEN
  PERFORM public.b3_shape(payload,ARRAY['format','text']);
  IF payload->>'format' IS DISTINCT FROM 'markdown' OR jsonb_typeof(payload->'text') IS DISTINCT FROM 'string'
     OR octet_length(payload->>'text')>200000 THEN
   RAISE EXCEPTION 'invalid text payload' USING ERRCODE='23514';
  END IF;
 END IF;
 IF jsonb_array_length(result)>256 THEN
  RAISE EXCEPTION 'too many direct dependencies' USING ERRCODE='23514';
 END IF;
 RETURN result;
END $$;

-- Return only dependencies dictated by actual immutable rows. Predecessors are
-- intentionally not evidence: they remain same-stream audit foreign keys.
CREATE FUNCTION public.b3_expected_dependencies(kind text, object_id uuid, revision_id uuid)
RETURNS jsonb LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE result jsonb:='[]'; row_data record; reference jsonb;
BEGIN
 CASE kind
 WHEN 'block' THEN
  SELECT r.contract_version,r.content INTO STRICT row_data FROM public.block_revision r
   WHERE r.block_id=object_id AND r.id=revision_id;
  RETURN public.b3_content_dependencies(row_data.contract_version,row_data.content);
 WHEN 'relation' THEN
  SELECT * INTO STRICT row_data FROM public.relation_revision r WHERE r.relation_id=object_id AND r.id=revision_id;
  result:=public.b3_edge('target',public.b3_reference(jsonb_build_object('block_id',row_data.from_block_id,'revision_id',row_data.from_revision_id),'block'))
   ||public.b3_edge('target',public.b3_reference(jsonb_build_object('block_id',row_data.to_block_id,'revision_id',row_data.to_revision_id),'block'));
 WHEN 'relation_review' THEN
  SELECT * INTO STRICT row_data FROM public.relation_review r WHERE r.relation_revision_id=object_id AND r.id=revision_id;
  result:=public.b3_edge('target',public.b3_reference(jsonb_build_object('relation_id',row_data.relation_id,'revision_id',row_data.relation_revision_id),'relation'));
 WHEN 'epistemic_review' THEN
  SELECT r.*,s.target_block_id,s.target_revision_id INTO STRICT row_data FROM public.epistemic_review r
   JOIN public.epistemic_stream s ON s.id=r.stream_id WHERE r.stream_id=object_id AND r.id=revision_id;
  result:=public.b3_edge('target',public.b3_reference(jsonb_build_object('block_id',row_data.target_block_id,'revision_id',row_data.target_revision_id),'block'))
   ||public.b3_selections(row_data.relations);
  IF jsonb_typeof(row_data.evidence) IS DISTINCT FROM 'array' OR jsonb_array_length(row_data.evidence)>256 THEN
   RAISE EXCEPTION 'invalid review evidence' USING ERRCODE='23514';
  END IF;
  FOR reference IN SELECT jsonb_array_elements(row_data.evidence) LOOP
   result:=result||public.b3_edge('basis',public.b3_reference(reference,'block'));
  END LOOP;
 ELSE RAISE EXCEPTION 'invalid source kind' USING ERRCODE='23514';
 END CASE;
 RETURN result;
EXCEPTION WHEN no_data_found THEN
 RAISE EXCEPTION 'registry has no matching immutable object' USING ERRCODE='23514';
END $$;

CREATE FUNCTION public.b3_check_object(kind text, object_id uuid, revision_id uuid)
RETURNS void LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE actual_space uuid; registered_space uuid; expected jsonb; indexed jsonb;
BEGIN
 CASE kind
 WHEN 'block' THEN SELECT r.space_id INTO actual_space FROM public.block_revision r WHERE r.block_id=object_id AND r.id=revision_id;
 WHEN 'relation' THEN SELECT r.space_id INTO actual_space FROM public.relation_revision r WHERE r.relation_id=object_id AND r.id=revision_id;
 WHEN 'relation_review' THEN SELECT r.space_id INTO actual_space FROM public.relation_review r WHERE r.relation_revision_id=object_id AND r.id=revision_id;
 WHEN 'epistemic_review' THEN SELECT r.space_id INTO actual_space FROM public.epistemic_review r WHERE r.stream_id=object_id AND r.id=revision_id;
 ELSE RAISE EXCEPTION 'invalid source kind' USING ERRCODE='23514';
 END CASE;
 SELECT r.space_id INTO registered_space FROM public.reference_object r
  WHERE r.kind=b3_check_object.kind AND r.object_id=b3_check_object.object_id AND r.revision_id=b3_check_object.revision_id;
 IF actual_space IS NULL OR registered_space IS DISTINCT FROM actual_space THEN
  RAISE EXCEPTION 'registry identity or space mismatch' USING ERRCODE='23514';
 END IF;
 SELECT coalesce(jsonb_agg(value||jsonb_build_object('position',ordinality-1) ORDER BY ordinality),'[]') INTO expected
  FROM jsonb_array_elements(public.b3_expected_dependencies(kind,object_id,revision_id)) WITH ORDINALITY;
 SELECT coalesce(jsonb_agg(jsonb_build_object('position',d.position,'role',d.role,
  'target',jsonb_build_object('kind',d.target_kind,'object_id',d.target_object_id,'revision_id',d.target_revision_id)) ORDER BY d.position),'[]')
  INTO indexed FROM public.reference_dependency d
  WHERE d.source_kind=kind AND d.source_object_id=object_id AND d.source_revision_id=revision_id;
 IF indexed IS DISTINCT FROM expected THEN
  RAISE EXCEPTION 'dependency index differs from immutable payload' USING ERRCODE='23514';
 END IF;
END $$;

CREATE FUNCTION public.b3_register_object()
RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE kind text; object_id uuid;
BEGIN
 CASE TG_TABLE_NAME
 WHEN 'block_revision' THEN kind:='block'; object_id:=NEW.block_id;
 WHEN 'relation_revision' THEN kind:='relation'; object_id:=NEW.relation_id;
 WHEN 'relation_review' THEN kind:='relation_review'; object_id:=NEW.relation_revision_id;
 WHEN 'epistemic_review' THEN kind:='epistemic_review'; object_id:=NEW.stream_id;
 ELSE RAISE EXCEPTION 'unexpected registry source' USING ERRCODE='23514';
 END CASE;
 INSERT INTO public.reference_object(kind,object_id,revision_id,space_id)
 VALUES(kind,object_id,NEW.id,NEW.space_id) ON CONFLICT DO NOTHING;
 RETURN NEW;
END $$;
CREATE TRIGGER block_register AFTER INSERT ON block_revision FOR EACH ROW EXECUTE FUNCTION public.b3_register_object();
CREATE TRIGGER relation_register AFTER INSERT ON relation_revision FOR EACH ROW EXECUTE FUNCTION public.b3_register_object();
CREATE TRIGGER relation_review_register AFTER INSERT ON relation_review FOR EACH ROW EXECUTE FUNCTION public.b3_register_object();
CREATE TRIGGER epistemic_review_register AFTER INSERT ON epistemic_review FOR EACH ROW EXECUTE FUNCTION public.b3_register_object();

INSERT INTO reference_object(kind,object_id,revision_id,space_id) SELECT 'block',block_id,id,space_id FROM block_revision;

CREATE FUNCTION public.b3_check_registry_trigger()
RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM public.b3_check_object(NEW.kind,NEW.object_id,NEW.revision_id);
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER reference_object_consistent AFTER INSERT ON reference_object
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.b3_check_registry_trigger();
CREATE FUNCTION public.b3_check_dependency_trigger()
RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM public.b3_check_object(NEW.source_kind,NEW.source_object_id,NEW.source_revision_id);
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER reference_dependency_consistent AFTER INSERT ON reference_dependency
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.b3_check_dependency_trigger();

GRANT SELECT,INSERT ON reference_object,reference_dependency,relation,relation_revision,relation_review_head,
 relation_review,epistemic_stream,epistemic_review,relation_receipt,relation_review_receipt,epistemic_review_receipt TO learning_runtime;
GRANT UPDATE(head_revision_id) ON relation TO learning_runtime;
GRANT UPDATE(head_review_id) ON relation_review_head,epistemic_stream TO learning_runtime;
REVOKE ALL ON FUNCTION public.b3_shape(jsonb,text[],text[]),public.b3_reference(jsonb,text),public.b3_edge(text,jsonb),
 public.b3_selections(jsonb),public.b3_content_dependencies(integer,jsonb),public.b3_expected_dependencies(text,uuid,uuid),
 public.b3_check_object(text,uuid,uuid),public.b3_register_object(),public.b3_check_registry_trigger(),public.b3_check_dependency_trigger() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.b3_shape(jsonb,text[],text[]),public.b3_reference(jsonb,text),public.b3_edge(text,jsonb),
 public.b3_selections(jsonb),public.b3_content_dependencies(integer,jsonb),public.b3_expected_dependencies(text,uuid,uuid),
 public.b3_check_object(text,uuid,uuid),public.b3_register_object(),public.b3_check_registry_trigger(),public.b3_check_dependency_trigger() TO learning_runtime;
