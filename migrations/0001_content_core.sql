CREATE TABLE app_user (id uuid PRIMARY KEY);
CREATE TABLE space (id uuid PRIMARY KEY, owner_id uuid NOT NULL REFERENCES app_user(id));
CREATE TABLE space_grant (
    actor_id uuid NOT NULL REFERENCES app_user(id),
    space_id uuid NOT NULL REFERENCES space(id),
    can_write boolean NOT NULL DEFAULT false,
    PRIMARY KEY(actor_id, space_id)
);
CREATE TABLE block (
    id uuid PRIMARY KEY,
    space_id uuid NOT NULL REFERENCES space(id),
    head_revision_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(space_id, id)
);
CREATE TABLE block_revision (
    id uuid PRIMARY KEY,
    space_id uuid NOT NULL,
    block_id uuid NOT NULL,
    parent_revision_id uuid,
    content jsonb NOT NULL,
    content_sha256 text NOT NULL CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
    author_id uuid NOT NULL REFERENCES app_user(id),
    reason text NOT NULL CHECK (char_length(reason) BETWEEN 1 AND 1000),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(space_id, block_id, id),
    CHECK (parent_revision_id IS NULL OR parent_revision_id <> id),
    FOREIGN KEY(space_id, block_id) REFERENCES block(space_id, id),
    FOREIGN KEY(space_id, block_id, parent_revision_id) REFERENCES block_revision(space_id, block_id, id)
);
ALTER TABLE block ADD CONSTRAINT valid_head
    FOREIGN KEY(space_id, id, head_revision_id) REFERENCES block_revision(space_id, block_id, id)
    DEFERRABLE INITIALLY DEFERRED;
CREATE TABLE mutation_receipt (
    actor_id uuid NOT NULL REFERENCES app_user(id),
    request_id uuid NOT NULL,
    request_sha256 text NOT NULL CHECK (request_sha256 ~ '^[0-9a-f]{64}$'),
    revision_id uuid NOT NULL REFERENCES block_revision(id),
    PRIMARY KEY(actor_id, request_id)
);
CREATE INDEX block_space ON block(space_id, created_at DESC, id DESC);
CREATE INDEX revision_block ON block_revision(block_id, created_at DESC, id DESC);
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
GRANT USAGE ON SCHEMA public TO learning_runtime;
GRANT SELECT ON app_user, space, space_grant TO learning_runtime;
GRANT SELECT, INSERT ON block, block_revision, mutation_receipt TO learning_runtime;
GRANT UPDATE(head_revision_id) ON block TO learning_runtime;

-- Roles are provisioned separately. Runtime must never be a member of auth_lock.
-- This function locks grants without allowing the runtime to edit permissions.
GRANT USAGE, CREATE ON SCHEMA public TO learning_auth_lock;
GRANT SELECT, UPDATE(can_write) ON space_grant TO learning_auth_lock;
CREATE FUNCTION public.lock_space_grant(requested_actor uuid, requested_space uuid)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp
AS $$
DECLARE allowed boolean;
BEGIN
    SELECT g.can_write INTO allowed FROM public.space_grant AS g
    WHERE g.actor_id = requested_actor AND g.space_id = requested_space
    FOR SHARE;
    RETURN allowed;
END;
$$;
REVOKE ALL ON FUNCTION public.lock_space_grant(uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.lock_space_grant(uuid,uuid) TO learning_runtime;
ALTER FUNCTION public.lock_space_grant(uuid,uuid) OWNER TO learning_auth_lock;
REVOKE CREATE ON SCHEMA public FROM learning_auth_lock;
