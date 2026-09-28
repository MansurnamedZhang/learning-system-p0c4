ALTER TABLE request_key DROP CONSTRAINT request_key_operation_check;
ALTER TABLE request_key ADD CONSTRAINT request_key_operation_check CHECK(operation IN
 ('content_v1','composition_save','release_publish','reading_create','reading_edit','migration_propose','migration_decide'));
CREATE TABLE overlay (
 id uuid PRIMARY KEY,space_id uuid NOT NULL REFERENCES space(id),owner_id uuid NOT NULL REFERENCES app_user(id),
 root_composition_id uuid NOT NULL REFERENCES composition(id),head_revision_id uuid NOT NULL,
 UNIQUE(space_id,id),UNIQUE(id,root_composition_id)
);
CREATE TABLE overlay_revision (
 id uuid PRIMARY KEY,overlay_id uuid NOT NULL,space_id uuid NOT NULL,root_composition_id uuid NOT NULL,
 source_space_id uuid NOT NULL,base_revision_id uuid NOT NULL,parent_revision_id uuid,
 title text NOT NULL CHECK(char_length(title)<=300),content_sha256 text NOT NULL CHECK(content_sha256 ~ '^[0-9a-f]{64}$'),
 author_id uuid NOT NULL REFERENCES app_user(id),reason text NOT NULL CHECK(char_length(reason) BETWEEN 1 AND 1000),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),UNIQUE(overlay_id,id),UNIQUE(overlay_id,id,root_composition_id,base_revision_id),
 FOREIGN KEY(space_id,overlay_id) REFERENCES overlay(space_id,id),
 FOREIGN KEY(overlay_id,root_composition_id) REFERENCES overlay(id,root_composition_id),
 FOREIGN KEY(source_space_id,root_composition_id,base_revision_id) REFERENCES composition_revision(space_id,composition_id,id),
 FOREIGN KEY(overlay_id,parent_revision_id) REFERENCES overlay_revision(overlay_id,id),
 CHECK(parent_revision_id IS NULL OR parent_revision_id<>id)
);
ALTER TABLE overlay ADD CONSTRAINT overlay_head_same_object FOREIGN KEY(id,head_revision_id) REFERENCES overlay_revision(overlay_id,id) DEFERRABLE INITIALLY DEFERRED;
CREATE TABLE overlay_group_identity (
 overlay_id uuid NOT NULL REFERENCES overlay(id),group_id uuid NOT NULL,PRIMARY KEY(overlay_id,group_id)
);
CREATE TABLE overlay_placement_identity (
 overlay_id uuid NOT NULL REFERENCES overlay(id),placement_id uuid NOT NULL,block_id uuid NOT NULL REFERENCES block(id),
 PRIMARY KEY(overlay_id,placement_id),UNIQUE(overlay_id,placement_id,block_id)
);
CREATE TABLE overlay_group (
 overlay_id uuid NOT NULL,overlay_revision_id uuid NOT NULL,group_id uuid NOT NULL,
 placed boolean NOT NULL,source_space_id uuid NOT NULL,root_composition_id uuid NOT NULL,base_revision_id uuid NOT NULL,
 placed_base_revision_id uuid, parent_path uuid[] NOT NULL CHECK(cardinality(parent_path)<=15 AND array_position(parent_path,NULL) IS NULL),
 left_id uuid,right_id uuid,affinity text NOT NULL CHECK(affinity IN ('after_left','before_right')),
 PRIMARY KEY(overlay_revision_id,group_id),UNIQUE(overlay_id,overlay_revision_id,group_id),
 FOREIGN KEY(overlay_id,overlay_revision_id) REFERENCES overlay_revision(overlay_id,id),
 FOREIGN KEY(overlay_id,group_id) REFERENCES overlay_group_identity(overlay_id,group_id),
 FOREIGN KEY(overlay_id,root_composition_id) REFERENCES overlay(id,root_composition_id),
 FOREIGN KEY(source_space_id,root_composition_id,base_revision_id) REFERENCES composition_revision(space_id,composition_id,id),
 FOREIGN KEY(overlay_id,overlay_revision_id,root_composition_id,placed_base_revision_id) REFERENCES overlay_revision(overlay_id,id,root_composition_id,base_revision_id),
 CHECK((placed AND placed_base_revision_id IS NOT NULL AND placed_base_revision_id=base_revision_id) OR (NOT placed AND placed_base_revision_id IS NULL)),
 CHECK(left_id IS NULL OR right_id IS NULL OR left_id<>right_id)
);
CREATE UNIQUE INDEX overlay_gap_unique ON overlay_group(overlay_revision_id,root_composition_id,base_revision_id,parent_path,left_id,right_id,affinity) NULLS NOT DISTINCT WHERE placed;
CREATE TABLE overlay_placement (
 overlay_id uuid NOT NULL,overlay_revision_id uuid NOT NULL,placement_id uuid NOT NULL,group_id uuid NOT NULL,
 position integer NOT NULL CHECK(position BETWEEN 0 AND 2047),block_space_id uuid NOT NULL,block_id uuid NOT NULL,block_revision_id uuid NOT NULL,
 PRIMARY KEY(overlay_revision_id,placement_id),UNIQUE(overlay_revision_id,group_id,position),
 FOREIGN KEY(overlay_id,overlay_revision_id,group_id) REFERENCES overlay_group(overlay_id,overlay_revision_id,group_id),
 FOREIGN KEY(overlay_id,placement_id,block_id) REFERENCES overlay_placement_identity(overlay_id,placement_id,block_id),
 FOREIGN KEY(block_space_id,block_id,block_revision_id) REFERENCES block_revision(space_id,block_id,id)
);
CREATE TABLE reading_view (
 id uuid PRIMARY KEY,overlay_id uuid NOT NULL UNIQUE REFERENCES overlay(id),head_revision_id uuid NOT NULL,UNIQUE(id,overlay_id)
);
CREATE TABLE reading_view_revision (
 id uuid PRIMARY KEY,view_id uuid NOT NULL,overlay_id uuid NOT NULL,overlay_revision_id uuid NOT NULL,parent_revision_id uuid,
 author_id uuid NOT NULL REFERENCES app_user(id),created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(view_id,id),UNIQUE(overlay_id,overlay_revision_id,id),UNIQUE(view_id,overlay_id,overlay_revision_id,id),
 FOREIGN KEY(view_id,overlay_id) REFERENCES reading_view(id,overlay_id),
 FOREIGN KEY(overlay_id,overlay_revision_id) REFERENCES overlay_revision(overlay_id,id),
 FOREIGN KEY(view_id,parent_revision_id) REFERENCES reading_view_revision(view_id,id),CHECK(parent_revision_id IS NULL OR parent_revision_id<>id)
);
ALTER TABLE reading_view ADD CONSTRAINT reading_head_same_object FOREIGN KEY(id,head_revision_id) REFERENCES reading_view_revision(view_id,id) DEFERRABLE INITIALLY DEFERRED;
CREATE TABLE reading_receipt (
 actor_id uuid NOT NULL,request_id uuid NOT NULL,overlay_id uuid NOT NULL,overlay_revision_id uuid NOT NULL,view_id uuid NOT NULL,view_revision_id uuid NOT NULL,
 PRIMARY KEY(actor_id,request_id),FOREIGN KEY(actor_id,request_id) REFERENCES request_key(actor_id,request_id),
 FOREIGN KEY(view_id,overlay_id,overlay_revision_id,view_revision_id) REFERENCES reading_view_revision(view_id,overlay_id,overlay_revision_id,id)
);
CREATE TABLE reading_receipt_block (
 actor_id uuid NOT NULL,request_id uuid NOT NULL,position integer NOT NULL CHECK(position BETWEEN 0 AND 31),
 block_space_id uuid NOT NULL,block_id uuid NOT NULL,revision_id uuid NOT NULL,
 PRIMARY KEY(actor_id,request_id,position),FOREIGN KEY(actor_id,request_id) REFERENCES reading_receipt(actor_id,request_id),
 FOREIGN KEY(block_space_id,block_id,revision_id) REFERENCES block_revision(space_id,block_id,id)
);
CREATE TABLE placement_migration (
 id uuid PRIMARY KEY,overlay_id uuid NOT NULL,old_overlay_revision_id uuid NOT NULL,
 target_space_id uuid NOT NULL,target_root uuid NOT NULL,target_revision_id uuid NOT NULL,
 author_id uuid NOT NULL REFERENCES app_user(id),reason text NOT NULL CHECK(char_length(reason) BETWEEN 1 AND 1000),created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(overlay_id,id),UNIQUE(id,old_overlay_revision_id),
 FOREIGN KEY(overlay_id,old_overlay_revision_id) REFERENCES overlay_revision(overlay_id,id),
 FOREIGN KEY(overlay_id,target_root) REFERENCES overlay(id,root_composition_id),
 FOREIGN KEY(target_space_id,target_root,target_revision_id) REFERENCES composition_revision(space_id,composition_id,id)
);
CREATE TABLE placement_migration_group (
 proposal_id uuid NOT NULL,old_overlay_revision_id uuid NOT NULL,group_id uuid NOT NULL,
 classification text NOT NULL CHECK(classification IN ('exact','candidate','unresolved')),
 candidate jsonb,reason_code text NOT NULL,
 PRIMARY KEY(proposal_id,group_id),FOREIGN KEY(proposal_id,old_overlay_revision_id) REFERENCES placement_migration(id,old_overlay_revision_id),
 FOREIGN KEY(old_overlay_revision_id,group_id) REFERENCES overlay_group(overlay_revision_id,group_id),
 CHECK((classification='unresolved' AND candidate IS NULL) OR (classification<>'unresolved' AND candidate IS NOT NULL))
);
CREATE TABLE placement_migration_decision (
 id uuid PRIMARY KEY,overlay_id uuid NOT NULL,proposal_id uuid NOT NULL UNIQUE,adopted boolean NOT NULL,
 result_overlay_revision_id uuid,result_view_revision_id uuid,
 author_id uuid NOT NULL REFERENCES app_user(id),reason text NOT NULL CHECK(char_length(reason) BETWEEN 1 AND 1000),created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(id,overlay_id),FOREIGN KEY(overlay_id,proposal_id) REFERENCES placement_migration(overlay_id,id),
 FOREIGN KEY(overlay_id,result_overlay_revision_id,result_view_revision_id) REFERENCES reading_view_revision(overlay_id,overlay_revision_id,id),
 CHECK((adopted AND result_overlay_revision_id IS NOT NULL AND result_view_revision_id IS NOT NULL) OR (NOT adopted AND result_overlay_revision_id IS NULL AND result_view_revision_id IS NULL))
);
CREATE TABLE placement_migration_mapping (
 decision_id uuid NOT NULL,overlay_id uuid NOT NULL,source_group_id uuid NOT NULL,result_group_id uuid NOT NULL,
 disposition text NOT NULL CHECK(disposition IN ('exact','candidate','manual','unplaced')),placement_order uuid[] NOT NULL,
 PRIMARY KEY(decision_id,source_group_id),FOREIGN KEY(decision_id,overlay_id) REFERENCES placement_migration_decision(id,overlay_id),
 FOREIGN KEY(overlay_id,source_group_id) REFERENCES overlay_group_identity(overlay_id,group_id),
 FOREIGN KEY(overlay_id,result_group_id) REFERENCES overlay_group_identity(overlay_id,group_id)
);
CREATE TABLE migration_receipt (
 actor_id uuid NOT NULL,request_id uuid NOT NULL,proposal_id uuid REFERENCES placement_migration(id),decision_id uuid REFERENCES placement_migration_decision(id),
 PRIMARY KEY(actor_id,request_id),FOREIGN KEY(actor_id,request_id) REFERENCES request_key(actor_id,request_id),
 CHECK((proposal_id IS NULL)<>(decision_id IS NULL))
);
-- Explicit records for manual placement without a new original-content migration.
CREATE TABLE placement_manual_decision (
 overlay_id uuid NOT NULL,overlay_revision_id uuid NOT NULL,source_group_id uuid NOT NULL,result_group_id uuid NOT NULL,
 PRIMARY KEY(overlay_revision_id,source_group_id),FOREIGN KEY(overlay_id,overlay_revision_id) REFERENCES overlay_revision(overlay_id,id),
 FOREIGN KEY(overlay_id,source_group_id) REFERENCES overlay_group_identity(overlay_id,group_id),
 FOREIGN KEY(overlay_id,result_group_id) REFERENCES overlay_group_identity(overlay_id,group_id)
);
GRANT SELECT,INSERT ON overlay,overlay_revision,overlay_group_identity,overlay_placement_identity,overlay_group,overlay_placement,
 reading_view,reading_view_revision,reading_receipt,reading_receipt_block,placement_migration,placement_migration_group,
 placement_migration_decision,placement_migration_mapping,migration_receipt,placement_manual_decision TO learning_runtime;
GRANT UPDATE(head_revision_id) ON overlay,reading_view TO learning_runtime;
CREATE INDEX overlay_owner ON overlay(owner_id,space_id,id);
CREATE INDEX overlay_revision_history ON overlay_revision(overlay_id,created_at,id);
CREATE INDEX migration_overlay ON placement_migration(overlay_id,created_at,id);
