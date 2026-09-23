-- Original bytes live in the immutable file store. These rows give them
-- independent, space-scoped logical identities and precise use locations.
ALTER TABLE block_revision DROP CONSTRAINT block_revision_contract_version_check;
ALTER TABLE block_revision ADD CONSTRAINT block_revision_contract_version_check
 CHECK(contract_version IN (1,2,3));

ALTER TABLE request_key DROP CONSTRAINT request_key_operation_check;
ALTER TABLE request_key ADD CONSTRAINT request_key_operation_check CHECK(operation IN
 ('content_v1','composition_save','release_publish','reading_create','reading_edit',
  'migration_propose','migration_decide','content_v2','relation_save','relation_review','epistemic_review',
  'reading_select','release_publish_evidence','lineage_apply','asset_register','content_v3'));

CREATE TABLE asset (
 space_id uuid NOT NULL REFERENCES space(id),
 id uuid NOT NULL,
 sha256 text NOT NULL CHECK(sha256 ~ '^[0-9a-f]{64}$'),
 byte_size bigint NOT NULL CHECK(byte_size>=0),
 storage_key text NOT NULL,
 media_type text NOT NULL CHECK(char_length(media_type) BETWEEN 3 AND 200 AND media_type ~ '^[A-Za-z0-9.+_-]+/[A-Za-z0-9.+_-]+$'),
 original_file_name text NOT NULL CHECK(char_length(original_file_name) BETWEEN 1 AND 255 AND btrim(original_file_name)<>''),
 status text NOT NULL CHECK(status='ready'),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(space_id,id),
 CHECK(storage_key='sha256/'||substring(sha256 from 1 for 2)||'/'||sha256)
);
CREATE INDEX asset_digest_idx ON asset(sha256);

CREATE TABLE resource (
 space_id uuid NOT NULL REFERENCES space(id),
 id uuid NOT NULL,
 display_name text NOT NULL CHECK(char_length(display_name) BETWEEN 1 AND 255 AND btrim(display_name)<>''),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(space_id,id)
);

CREATE TABLE resource_version (
 space_id uuid NOT NULL,
 resource_id uuid NOT NULL,
 id uuid NOT NULL,
 asset_id uuid NOT NULL,
 version_no integer NOT NULL CHECK(version_no>0),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(space_id,id),
 UNIQUE(space_id,resource_id,id),
 UNIQUE(space_id,resource_id,version_no),
 FOREIGN KEY(space_id,resource_id) REFERENCES resource(space_id,id),
 FOREIGN KEY(space_id,asset_id) REFERENCES asset(space_id,id)
);
CREATE INDEX resource_version_asset_idx ON resource_version(space_id,asset_id);

CREATE TABLE source_segment (
 space_id uuid NOT NULL,
 resource_id uuid NOT NULL,
 resource_version_id uuid NOT NULL,
 id uuid NOT NULL,
 selector jsonb NOT NULL CHECK(jsonb_typeof(selector)='object' AND octet_length(selector::text)<=16000),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(space_id,id),
 FOREIGN KEY(space_id,resource_id,resource_version_id)
  REFERENCES resource_version(space_id,resource_id,id)
);
CREATE INDEX source_segment_version_idx ON source_segment(space_id,resource_id,resource_version_id);

CREATE TABLE upload_receipt (
 actor_id uuid NOT NULL REFERENCES app_user(id),
 request_id uuid NOT NULL,
 space_id uuid NOT NULL,
 asset_id uuid NOT NULL,
 PRIMARY KEY(actor_id,request_id),
 UNIQUE(space_id,asset_id),
 FOREIGN KEY(actor_id,request_id) REFERENCES request_key(actor_id,request_id),
 FOREIGN KEY(space_id,asset_id) REFERENCES asset(space_id,id)
);

CREATE TABLE block_asset_use (
 space_id uuid NOT NULL,
 block_id uuid NOT NULL,
 revision_id uuid NOT NULL,
 asset_id uuid NOT NULL,
 PRIMARY KEY(space_id,block_id,revision_id),
 FOREIGN KEY(space_id,block_id,revision_id)
  REFERENCES block_revision(space_id,block_id,id),
 FOREIGN KEY(space_id,asset_id) REFERENCES asset(space_id,id)
);
CREATE INDEX block_asset_use_asset_idx ON block_asset_use(space_id,asset_id);

REVOKE ALL ON asset,resource,resource_version,source_segment,upload_receipt,block_asset_use FROM PUBLIC;
GRANT SELECT,INSERT ON asset,resource,resource_version,source_segment,upload_receipt,block_asset_use TO learning_runtime;
