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
    head_revision_id uuid,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(space_id, id)
);
CREATE TABLE block_revision (
    id uuid PRIMARY KEY,
    space_id uuid NOT NULL,
    block_id uuid NOT NULL,
    parent_revision_id uuid,
    content jsonb NOT NULL,
    content_sha256 text NOT NULL CHECK (length(content_sha256) = 64),
    author_id uuid NOT NULL REFERENCES app_user(id),
    reason text NOT NULL,
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
    request_sha256 text NOT NULL,
    revision_id uuid NOT NULL REFERENCES block_revision(id),
    PRIMARY KEY(actor_id, request_id)
);
CREATE INDEX block_space ON block(space_id, created_at DESC, id);
CREATE INDEX revision_block ON block_revision(block_id, created_at DESC, id);
GRANT USAGE ON SCHEMA public TO learning_runtime;
GRANT SELECT ON app_user, space, space_grant TO learning_runtime;
GRANT SELECT, INSERT ON block, block_revision, mutation_receipt TO learning_runtime;
GRANT UPDATE(head_revision_id) ON block TO learning_runtime;

