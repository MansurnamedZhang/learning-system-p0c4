-- Preserve the applied v1/v2 validator byte-for-byte under its original OID.
-- The new dispatch adds only the v3 bodies and delegates their block/relation
-- dependency semantics to that validator after replacing asset bodies with an
-- empty text body. Asset use is checked separately below.
ALTER FUNCTION public.b3_content_dependencies(integer,jsonb)
 RENAME TO p0c_legacy_content_dependencies;

CREATE FUNCTION public.b3_content_dependencies(version integer, value jsonb)
RETURNS jsonb LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE body jsonb; asset_ref jsonb; body_kind text;
BEGIN
 IF version IN (1,2) THEN
  RETURN public.p0c_legacy_content_dependencies(version,value);
 END IF;
 IF version<>3 THEN
  RAISE EXCEPTION 'invalid content version' USING ERRCODE='23514';
 END IF;
 body:=value->'body';
 body_kind:=body->>'kind';
 CASE body_kind
 WHEN 'figure' THEN
  PERFORM public.b3_shape(body,ARRAY['kind','asset','usage','caption','alt','decorative']);
  IF jsonb_typeof(body->'usage') IS DISTINCT FROM 'string'
     OR char_length(btrim(body->>'usage'))=0 OR char_length(body->>'usage')>100
     OR jsonb_typeof(body->'caption') IS DISTINCT FROM 'string'
     OR char_length(body->>'caption')>1000
     OR jsonb_typeof(body->'alt') IS DISTINCT FROM 'string'
     OR char_length(body->>'alt')>500
     OR jsonb_typeof(body->'decorative') IS DISTINCT FROM 'boolean'
     OR (body->>'decorative')::boolean AND body->>'alt'<>''
     OR NOT (body->>'decorative')::boolean AND char_length(btrim(body->>'alt'))=0 THEN
   RAISE EXCEPTION 'invalid figure body' USING ERRCODE='23514';
  END IF;
 WHEN 'attachment' THEN
  PERFORM public.b3_shape(body,ARRAY['kind','asset','display_name']);
  IF jsonb_typeof(body->'display_name') IS DISTINCT FROM 'string'
     OR char_length(btrim(body->>'display_name'))=0
     OR char_length(body->>'display_name')>255 THEN
   RAISE EXCEPTION 'invalid attachment body' USING ERRCODE='23514';
  END IF;
 WHEN 'text','reference','relation_view' THEN
  RETURN public.p0c_legacy_content_dependencies(2,value);
 ELSE
  RAISE EXCEPTION 'invalid v3 body' USING ERRCODE='23514';
 END CASE;
 asset_ref:=body->'asset';
 PERFORM public.b3_shape(asset_ref,ARRAY['space_id','asset_id']);
 IF jsonb_typeof(asset_ref->'space_id') IS DISTINCT FROM 'string'
    OR jsonb_typeof(asset_ref->'asset_id') IS DISTINCT FROM 'string' THEN
  RAISE EXCEPTION 'invalid asset reference' USING ERRCODE='23514';
 END IF;
 PERFORM (asset_ref->>'space_id')::uuid,(asset_ref->>'asset_id')::uuid;
 RETURN public.p0c_legacy_content_dependencies(
  2,value||jsonb_build_object('body',
     jsonb_build_object('kind','text','payload',jsonb_build_object('format','markdown','text',''))));
END $$;

-- A deferred check sees the final state of the transaction, including the
-- revision inserted before its use row. It also forbids a later deletion,
-- replacement or extra use row that would contradict immutable JSON.
CREATE FUNCTION public.p0c_check_block_asset_use(checked_space uuid, checked_block uuid, checked_revision uuid)
RETURNS void LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE saved record; use_count integer; linked_asset uuid; expected_space uuid; expected_asset uuid;
BEGIN
 SELECT contract_version,content INTO saved FROM public.block_revision
  WHERE space_id=checked_space AND block_id=checked_block AND id=checked_revision;
 IF NOT FOUND THEN RETURN; END IF;
 SELECT asset_id INTO linked_asset FROM public.block_asset_use
  WHERE space_id=checked_space AND block_id=checked_block AND revision_id=checked_revision;
 IF FOUND THEN use_count:=1; ELSE use_count:=0; END IF;
 IF saved.contract_version=3 AND saved.content#>>'{body,kind}' IN ('figure','attachment') THEN
  expected_space:=(saved.content#>>'{body,asset,space_id}')::uuid;
  expected_asset:=(saved.content#>>'{body,asset,asset_id}')::uuid;
  IF expected_space IS DISTINCT FROM checked_space OR use_count<>1
     OR linked_asset IS DISTINCT FROM expected_asset THEN
   RAISE EXCEPTION 'block asset use does not match v3 content' USING ERRCODE='23514';
  END IF;
 ELSIF use_count<>0 THEN
  RAISE EXCEPTION 'non-asset content has asset use' USING ERRCODE='23514';
 END IF;
END $$;

CREATE FUNCTION public.p0c_block_asset_revision_trigger()
RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM public.p0c_check_block_asset_use(NEW.space_id,NEW.block_id,NEW.id);
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER p0c_block_asset_revision_consistent
 AFTER INSERT OR UPDATE ON public.block_revision DEFERRABLE INITIALLY DEFERRED
 FOR EACH ROW EXECUTE FUNCTION public.p0c_block_asset_revision_trigger();

CREATE FUNCTION public.p0c_block_asset_use_trigger()
RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF TG_OP IN ('UPDATE','DELETE') THEN
  PERFORM public.p0c_check_block_asset_use(OLD.space_id,OLD.block_id,OLD.revision_id);
 END IF;
 IF TG_OP IN ('UPDATE','INSERT') THEN
  PERFORM public.p0c_check_block_asset_use(NEW.space_id,NEW.block_id,NEW.revision_id);
 END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER p0c_block_asset_use_consistent
 AFTER INSERT OR UPDATE OR DELETE ON public.block_asset_use DEFERRABLE INITIALLY DEFERRED
 FOR EACH ROW EXECUTE FUNCTION public.p0c_block_asset_use_trigger();

REVOKE ALL ON FUNCTION public.b3_content_dependencies(integer,jsonb),
 public.p0c_check_block_asset_use(uuid,uuid,uuid),
 public.p0c_block_asset_revision_trigger(),public.p0c_block_asset_use_trigger() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.b3_content_dependencies(integer,jsonb),
 public.p0c_check_block_asset_use(uuid,uuid,uuid),
 public.p0c_block_asset_revision_trigger(),public.p0c_block_asset_use_trigger() TO learning_runtime;
