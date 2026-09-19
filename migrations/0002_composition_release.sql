CREATE TABLE request_key (
 actor_id uuid NOT NULL REFERENCES app_user(id), request_id uuid NOT NULL,
 request_sha256 text NOT NULL CHECK(request_sha256 ~ '^[0-9a-f]{64}$'),
 operation text NOT NULL CHECK(operation IN ('content_v1','composition_save','release_publish')),
 PRIMARY KEY(actor_id,request_id)
);
INSERT INTO request_key SELECT actor_id,request_id,request_sha256,'content_v1' FROM mutation_receipt;
ALTER TABLE mutation_receipt ADD CONSTRAINT content_request_registered
 FOREIGN KEY(actor_id,request_id) REFERENCES request_key(actor_id,request_id);

CREATE TABLE composition (
 id uuid PRIMARY KEY, space_id uuid NOT NULL REFERENCES space(id),
 kind text NOT NULL CHECK(kind IN ('document','section')), head_revision_id uuid NOT NULL,
 published_revision_id uuid, last_release_id uuid,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(), UNIQUE(space_id,id),
 CHECK((published_revision_id IS NULL) = (last_release_id IS NULL))
);
CREATE TABLE composition_revision (
 id uuid PRIMARY KEY, space_id uuid NOT NULL, composition_id uuid NOT NULL,
 parent_revision_id uuid, kind text NOT NULL CHECK(kind IN ('document','section')),
 title text NOT NULL CHECK(char_length(title)<=300),
 content_sha256 text NOT NULL CHECK(content_sha256 ~ '^[0-9a-f]{64}$'),
 author_id uuid NOT NULL REFERENCES app_user(id),reason text NOT NULL CHECK(char_length(reason) BETWEEN 1 AND 1000),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(space_id,composition_id,id),
 CHECK(parent_revision_id IS NULL OR parent_revision_id<>id),
 FOREIGN KEY(space_id,composition_id) REFERENCES composition(space_id,id),
 FOREIGN KEY(space_id,composition_id,parent_revision_id) REFERENCES composition_revision(space_id,composition_id,id)
);
ALTER TABLE composition ADD CONSTRAINT composition_head_same_object
 FOREIGN KEY(space_id,id,head_revision_id) REFERENCES composition_revision(space_id,composition_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE composition ADD CONSTRAINT composition_published_same_object
 FOREIGN KEY(space_id,id,published_revision_id) REFERENCES composition_revision(space_id,composition_id,id);
CREATE TABLE composition_occurrence (
 composition_revision_id uuid NOT NULL,space_id uuid NOT NULL,composition_id uuid NOT NULL,
 occurrence_id uuid NOT NULL, position integer NOT NULL CHECK(position BETWEEN 0 AND 511),
 block_space_id uuid,block_id uuid,block_revision_id uuid,
 child_space_id uuid,child_composition_id uuid,child_revision_id uuid,
 PRIMARY KEY(composition_revision_id,occurrence_id),UNIQUE(composition_revision_id,position),
 FOREIGN KEY(space_id,composition_id,composition_revision_id) REFERENCES composition_revision(space_id,composition_id,id),
 CHECK((block_space_id IS NOT NULL AND block_id IS NOT NULL AND block_revision_id IS NOT NULL AND child_space_id IS NULL AND child_composition_id IS NULL AND child_revision_id IS NULL)
 OR (block_space_id IS NULL AND block_id IS NULL AND block_revision_id IS NULL AND child_space_id IS NOT NULL AND child_composition_id IS NOT NULL AND child_revision_id IS NOT NULL)),
 FOREIGN KEY(block_space_id,block_id,block_revision_id) REFERENCES block_revision(space_id,block_id,id),
 FOREIGN KEY(child_space_id,child_composition_id,child_revision_id) REFERENCES composition_revision(space_id,composition_id,id)
);
CREATE TABLE composition_receipt (
 actor_id uuid NOT NULL, request_id uuid NOT NULL,revision_id uuid NOT NULL REFERENCES composition_revision(id),
 PRIMARY KEY(actor_id,request_id),FOREIGN KEY(actor_id,request_id) REFERENCES request_key(actor_id,request_id)
);
CREATE TABLE release (
 id uuid PRIMARY KEY,space_id uuid NOT NULL REFERENCES space(id),author_id uuid NOT NULL REFERENCES app_user(id),
 reason text NOT NULL CHECK(char_length(reason) BETWEEN 1 AND 1000),created_at timestamptz NOT NULL DEFAULT clock_timestamp(),UNIQUE(space_id,id)
);
CREATE TABLE release_root (
 release_id uuid NOT NULL,space_id uuid NOT NULL,composition_id uuid NOT NULL,revision_id uuid NOT NULL,
 PRIMARY KEY(release_id,composition_id),UNIQUE(release_id,composition_id,revision_id),
 FOREIGN KEY(space_id,release_id) REFERENCES release(space_id,id),
 FOREIGN KEY(space_id,composition_id,revision_id) REFERENCES composition_revision(space_id,composition_id,id)
);
ALTER TABLE composition ADD CONSTRAINT published_root_matches_release
 FOREIGN KEY(last_release_id,id,published_revision_id) REFERENCES release_root(release_id,composition_id,revision_id) DEFERRABLE INITIALLY DEFERRED;
CREATE TABLE release_receipt (
 actor_id uuid NOT NULL,request_id uuid NOT NULL,release_id uuid NOT NULL REFERENCES release(id),
 PRIMARY KEY(actor_id,request_id),FOREIGN KEY(actor_id,request_id) REFERENCES request_key(actor_id,request_id)
);
CREATE TABLE outbox_event (
 id uuid PRIMARY KEY,event_type text NOT NULL CHECK(event_type='composition_released'),
 aggregate_id uuid NOT NULL REFERENCES release(id),payload_version integer NOT NULL CHECK(payload_version=1),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),UNIQUE(event_type,aggregate_id)
);
CREATE INDEX occurrence_block_target ON composition_occurrence(block_revision_id);
CREATE INDEX occurrence_composition_target ON composition_occurrence(child_revision_id);
CREATE INDEX composition_space ON composition(space_id,created_at DESC,id DESC);
CREATE INDEX composition_history ON composition_revision(composition_id,created_at DESC,id DESC);
CREATE INDEX release_consumers ON release_root(composition_id,release_id);
GRANT SELECT,INSERT ON request_key,composition,composition_revision,composition_occurrence,composition_receipt,release,release_root,release_receipt,outbox_event TO learning_runtime;
GRANT UPDATE(head_revision_id,published_revision_id,last_release_id) ON composition TO learning_runtime;
