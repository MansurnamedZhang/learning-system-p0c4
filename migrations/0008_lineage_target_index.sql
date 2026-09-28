-- The lineage reverse query starts at an exact input revision and pages by
-- operation/position. Keep the immutable lineage tables authoritative; this
-- is only the measured target-first access path for that query.
CREATE INDEX lineage_input_target_page
    ON public.lineage_input (block_id, revision_id, operation_id, position);
