-- Measured on Task 2's synthetic dense target/keyset fixture. These are
-- access paths over existing immutable facts, never new relationship tables.
CREATE INDEX occurrence_block_target_page
    ON public.composition_occurrence
    (block_revision_id, block_id, composition_revision_id, occurrence_id);

CREATE INDEX reference_dependency_target_page
    ON public.reference_dependency
    (target_kind, target_object_id, target_revision_id,
     source_kind, source_object_id, source_revision_id, position);
