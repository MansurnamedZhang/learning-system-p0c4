# P0-C1 原件、附件与图片块 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 B4 已验收内核上保存并授权读取原字节原件，使图片和附件成为可固定到块修订、组合与个人位置的真实资产。

**Architecture:** PostgreSQL 保存空间内资产身份、精确使用关系、来源版本及幂等收据；独立本地文件适配器将原字节按 SHA-256 不可变定稿。`ContentDraft` v3 增加图片/附件体，所有 v3 写入经共享 `block_write::insert_content` 验证资产已就绪并登记精确使用关系。文件先就绪、数据库后提交，失败孤儿由保守对账处理。

**Tech Stack:** Rust 1.97 / edition 2024、SQLx/PostgreSQL、Tokio、本地文件系统、Docker Compose；沿用 B4 的普通非 owner runtime 角色和冻结旧版本升级夹具。

**Spec:** [P0-C 总设计](../specs/2026-09-23-p0c-assets-jobs-portability-design.md)，以及[批准路线](../../设计批准与实施路线.md)、[存储规范](../../内容分块与存储规范.md)。

## Global Constraints

- 精确基线 `fc25e91954419bd0c6c272d00577db1b74ce6ce8`；工作树 `隔离工作树`，分支 `feat/p0c-assets`。不改写迁移 0001–0008 或 v1/v2 的序列化与摘要。
- 资产 ID、资源版本 ID、块修订 ID 与 SHA-256 各有不同语义。首关资产和引用它的块/资源版本同属一个空间；物理同字节可复用，授权不因哈希复用而共享。
- `VersionedContentStore`、`LineageStore` 等公开写入路径都可到达共享 `block_write::insert_content`。v3 的资产校验和链接必须放在共同事务边界，不能只在一个外围服务检查。
- 当前 `outbox_event` 仅支持 `composition_released` 且外键指向 `release`。C1 不改它的历史；C2 另加通用 `job_outbox/job`。
- P0-C1 无 HTTP/登录、OCR、S3、整库导入或生产部署。测试文件、DB、卷和 staging 只用新隔离环境，保留旧 B4 证据。
- 所有成功声明以精确源码包、真实 PostgreSQL/Compose 原始日志、迁移升级、格式/Clippy 和独立审查为准；本计划的用例名与预期不能充作通过结果。

## Review Focus

1. 相同 SHA 但不同空间/主体：字节可去重，未授权者无法观察资产存在或读取内容（Task 3、5）。
2. 文件定稿成功而数据库提交失败：不产生可见资产或图片，延迟对账前字节保留（Task 2、4）。
3. 相同请求键带不同文件：明确冲突，不能复用第一次收据或覆盖旧字节（Task 4）。
4. 已就绪资产后来丢失/损坏：读取失败，不返回空文件或假图片；导出预检由 C3 再验证（Task 2、5）。
5. 历史 v1/v2 与新 v3 混合在同一固定阅读：旧摘要和精确位置不变，移动图片不重复写字节（Task 1、5）。

## File Map

| 文件 | 责任 |
|---|---|
| `crates/learning-core/src/asset.rs` | 精确资产/来源引用与图片、附件字段验证 |
| `crates/learning-core/src/content_version.rs` | 仅追加 v3 编解码、摘要与资产引用提取；保留 v1/v2 |
| `migrations/0009_assets.sql` | 空间内资产、来源版本、上传收据、块资产精确使用关系与最小权限 |
| `crates/learning-db/src/assets/{mod,write,read}.rs` | 元数据登记、幂等、当前授权下的精确引用读取 |
| `crates/learning-db/src/block_write.rs` | 所有 v3 写入共同校验并插入 `block_asset_use` |
| `crates/learning-db/src/{reviews,release,versioned_content}/` | v3 对审查、旧发布拒绝规则与历史投影的兼容处理 |
| `crates/learning-assets/src/{lib,fs}.rs` | staging、SHA-256、不可变定稿、核验与只读流 |
| `crates/learning-db/tests/{assets_schema,assets,asset_authorization,asset_reading}.rs` | 真实 PostgreSQL、权限与固定位置夹具 |
| `crates/learning-assets/tests/fs.rs` | 中断、重复字节、篡改与孤儿保护 |
| `docs/p0c1-{boundary,execution,verification}.md` | 已验证契约、决策、原始证据；实施时创建 |

## Task 1：v3 内容与资产契约

**Files:** Create `crates/learning-core/src/asset.rs`, `crates/learning-core/tests/asset_contracts.rs`; modify `crates/learning-core/src/{lib,content_version}.rs`.

**Interfaces:** Produce `AssetRef { space_id: Uuid, asset_id: Uuid }`, `ResourceVersionRef { space_id, resource_id, version_id }`, `AssetUseRef::{Block(BlockRef), Resource(ResourceVersionRef)}`, `BodyV3::{Text,Reference,RelationView,Figure,Attachment}`, and `ContentV3::{validate,digest,dependencies,asset_ref}`. `Figure` carries `asset`, `usage`, `caption`, and either nonempty `alt` or `decorative=true`; `Attachment` carries `asset` and `display_name`. V3 retains V2's `intent/language/title/basis_refs/requires_context/source_run` validation and dependencies. Task 4 connects this tested type to `ContentDraft::V3`; Task 1 leaves the existing public enum unchanged so the whole workspace still compiles.

- [x] **Step 1: Write failing contract tests.** Create literal v1/v2 fixtures with previously frozen digest strings; assert their encode/decode/digest remain byte-identical. Add `ContentV3` figure and attachment JSON round trips, required alt/decorative rule, invalid UUID/unknown field rejection, and exact `ContentV3::asset_ref()` extraction. This test must use literals, not only serialize-then-deserialize of the same struct.
- [x] **Step 2: Run RED.** `cargo test --offline --locked -p learning-core --test asset_contracts` must fail because `ContentV3` and `AssetRef` do not exist; record the actual stderr and exit code.
- [x] **Step 3: Add minimal types and dispatch.** Extend the existing version switch without editing v1/v2 branches. The expected public shape is:

  ```rust
  pub struct AssetRef { pub space_id: Uuid, pub asset_id: Uuid }
  pub struct ContentV3 {
      pub intent: Intent, pub language: String, pub title: String,
      pub body: BodyV3, pub basis_refs: Vec<ExactRef>,
      pub requires_context: Vec<BlockRef>, pub source_run: Option<BlockRef>,
  }
  impl ContentV3 {
      pub fn asset_ref(&self) -> Option<&AssetRef> {
          match &self.body {
              BodyV3::Figure { asset, .. } | BodyV3::Attachment { asset, .. } => Some(asset),
              _ => None,
          }
      }
  }
  ```

  V3 uses a distinct `content-v3` digest domain. Do not add the `ContentDraft::V3` variant or change existing command/revision serializers until Task 4.
- [x] **Step 4: Run GREEN and commit.** Rerun the named test, `cargo test --offline --locked -p learning-core`, and the existing workspace suite; commit only its code/tests after review of old digest fixtures.

## Task 2：本地不可变字节适配器

**Files:** Create `crates/learning-assets/{Cargo.toml,src/lib.rs,src/fs.rs,tests/fs.rs}`; modify workspace `Cargo.toml`/`Cargo.lock` and test Compose to mount separate `assets` and `staging` volumes. Do not put bytes in Git or the image.

**Interfaces:** `FsAssetStore::put_from_file(upload_id: Uuid, source: &Path, declaration: UploadDeclaration) -> Result<VerifiedBlob, AssetIoError>`; `UploadDeclaration { expected_size_bytes, max_size_bytes }` is mandatory, and the trusted service sets `max_size_bytes` from policy. Both staging and assets-local copy enforce the declaration before writing each chunk and require an exact final byte count. `verify(&VerifiedBlob) -> Result<(), AssetIoError>`; `open(&VerifiedBlob) -> Result<File, AssetIoError>`. `VerifiedBlob` contains generated storage key, SHA-256 and byte size, never an untrusted source path. A backend swap may implement the same interface after P0-C1. Ready registration checks PDF `%PDF-`, PNG 8-byte signature and Notebook JSON `cells`/`nbformat` shape when those specific media types are claimed. `application/octet-stream` is a permitted generic fallback, including for deliberately generic treatment of known bytes; other claimed types are unsupported. These checks do not constitute full parsing or content safety scanning.

- [x] **Step 1: Write failing file tests.** Use a test-owned temporary directory and at least PDF-like, PNG and Notebook byte fixtures. Assert original bytes and digest round trip; repeated same content retains immutable bytes; interrupted staging never returns `VerifiedBlob`; poisoned existing target is rejected; source basename is never used to form the server-generated target path.
- [x] **Step 2: Run RED.** `cargo test --offline --locked -p learning-assets --test fs` must fail before the crate exists; preserve stderr.
- [x] **Step 3: Add file adapter.** Stream to a unique `staging/<upload_id>.incoming-<nonce>` file, compute SHA-256 and byte count, flush/sync, then rename within staging to `staging/<upload_id>`; reject a source inside staging before writing. Because staging and assets are separate mounts, copy and verify into `assets/.finalizing/<upload_id>.pending-<nonce>`, flush/sync that file and its parent, then publish **within the assets volume** via atomic create-if-absent hard link to `assets/sha256/<prefix>/<digest>`, unlink the temporary name and sync destination and finalizing directories. Sync digest directory parent entries on both creation and retry/reuse, including the existing-final fast path. Plain rename could overwrite a concurrently published target, so do not use it for final publication. Neither temporary location is a visible `VerifiedBlob`; interrupted copies remain for later reconciliation. Verify an existing target before reuse. On filesystems without hard-link support, fail explicitly without returning a blob. `open` and `verify` recheck size/hash when a correctness decision requires it; a missing or corrupt file yields an explicit error, never empty bytes.
- [x] **Step 4: Run GREEN and commit.** Repeat the named tests including injected interrupted writes and mutation of a finalized test file. Commit adapter/tests/lockfile and mount configuration together.

## Task 3：资产与来源元数据迁移

**Files:** Create `migrations/0009_assets.sql`, `crates/learning-db/src/assets/{mod,write,read}.rs`, `crates/learning-db/tests/{assets_schema,assets}.rs`; modify `crates/learning-db/{Cargo.toml,src/lib.rs}` and migration-count expectations in existing upgrade tests, without changing frozen old fixtures.

**Interfaces:** `AssetStore` is constructed with its SQL pool and the Task 2 file adapter; `AssetStore::register_verified(actor, space, request_id, verified: VerifiedBlob, media: AssetMedia) -> Result<AssetRecord, ContentError>`; `AssetStore::link_resource_version(actor, resource, asset) -> Result<ResourceVersionRef, ContentError>`; `AssetStore::add_source_segment(actor, version, selector) -> Result<SourceSegmentRef, ContentError>`. `VerifiedBlob` is produced by Task 2's file adapter, but can become stale; registration re-verifies the finalized file before an asset row or receipt commits. Task 3 tests create real test-owned files through that adapter. `AssetMedia` and `AssetRecord` are defined in this task; Task 5 adds `read_for_use`.

- [x] **Step 1: Write schema/behavior RED.** On a new DB, require composite `(space_id,id)` keys for asset/resource/version and a precise `block_asset_use` FK to `(space_id,block_id,revision_id)`; no asset may link across spaces. Runtime role must lack schema/permission administration. Duplicate `request_id` with same digest returns one receipt; different digest conflicts. Corrupt a finalized file after `VerifiedBlob` creation but before registration and require no asset row or receipt. Source locator retains exact resource version and selector JSON.
- [x] **Step 2: Run RED.** `cargo test --offline --locked -p learning-db --test assets_schema --test assets -- --test-threads=1`; record actual failure against absent migration/store.
- [x] **Step 3: Add 0009 and stores.** Minimum SQL shape (add bounded text/size checks, FKs, indexes and grants in the real migration):

  ```sql
  CREATE TABLE asset (
    space_id uuid NOT NULL REFERENCES space(id), id uuid NOT NULL,
    sha256 text NOT NULL, byte_size bigint NOT NULL CHECK (byte_size >= 0),
    storage_key text NOT NULL, PRIMARY KEY (space_id,id));
  CREATE TABLE block_asset_use (
    space_id uuid NOT NULL, block_id uuid NOT NULL, revision_id uuid NOT NULL,
    asset_id uuid NOT NULL,
    PRIMARY KEY (space_id,block_id,revision_id),
    FOREIGN KEY (space_id,block_id,revision_id)
      REFERENCES block_revision(space_id,block_id,id),
    FOREIGN KEY (space_id,asset_id) REFERENCES asset(space_id,id));
  ```

  Add `resource`, `resource_version`, `source_segment`, `upload_receipt`; extend `block_revision.contract_version` check to include 3 and `request_key.operation` by appending `asset_register`/`content_v3`, preserving old values and global actor/request uniqueness. Migration 0004's `b3_content_dependencies` still rejects v3, so Task 3 must not claim committed v3 writes; Task 4 extends that function in 0010 together with the v3 write path. No direct lookup by hash is public. Resource/version/segment reads first require current grant; Task 5 supplies asset use reads under precise reference authorization. Validate display names as bounded metadata, never as storage paths.
- [x] **Step 4: Run GREEN, upgrade, commit.** Run both named tests and P0-A/B1/B3-schema/B2 frozen bootstrap scripts against new isolated databases; check old rows/receipts/digests unchanged. Commit migration/store/tests only after actual results and independent review.

## Task 4：资产登记和 v3 写入闭包

**Files:** Create `migrations/0010_asset_links.sql`; modify `crates/learning-core/src/content_version.rs`, `crates/learning-core/tests/asset_contracts.rs`, and `crates/learning-db/src/{block_write,versioned_content/write,versioned_content/read,assets/write,reviews/read,reviews/write,release/write}.rs`; add `crates/learning-db/tests/asset_authorization.rs` and v3 write scenarios. Existing `LineageStore` and Reading paths must pass via the same central guard. Do not rewrite 0009 after Task 3 has run it.

**Interfaces:** `AssetStore::register_verified` commits only after `FsAssetStore::verify`; `block_write::insert_content` checks `draft.asset_ref()` against same-space ready metadata and inserts `block_asset_use` in the block-revision transaction. Existing `ContentDraft::dependencies()` remains for B3 block/relation references; asset links are a separate exact closure.

- [x] **Step 1: Write behavior RED.** A figure referencing missing, wrong-space or uncommitted asset fails atomically; no block head, revision, use row or receipt survives. Same valid request replays same result; same request key/new asset conflicts. Direct `VersionedContentStore::create/revise` and derive/split using v3 cannot bypass the common guard. Reading `InsertNew/ReviseSelected` remain frozen `TextDraft`/v1 commands; v3 figure is created through the versioned content path and placed with Reading `InsertExisting`, while existing v1 Reading writes still traverse the shared primitive. Legacy `ReleaseStore::publish` rejects v3 and evidence-list publish accepts a visible v3 figure; source grouping and epistemic target intent use v3 fields correctly. Inject DB failure after file finalization and assert no visible asset or figure; file is retained for reconciliation. Raw SQL insertion of a v3 figure without its matching `block_asset_use` must fail at commit.
- [x] **Step 2: Run RED.** `cargo test --offline --locked -p learning-db --test asset_authorization -- --test-threads=1`; record actual failures.
- [x] **Step 3: Implement transactional link.** Add `ContentDraft::V3` decode/digest/dependencies/serializer arms in one integration change. In the shared write primitive, after grant locking and before commit, enforce `asset.space_id == block.space_id`, exact asset readiness and `block_asset_use` insertion. In 0010 extend 0004's `b3_content_dependencies` for v3 figure/attachment and preserved v1/v2 dependency semantics; add the deferred database integrity check so v3 JSON asset reference and link row match at commit. The insertion and revision/receipt must roll back together. Register v3 writes as `content_v3` but leave existing v1/v2 receipts unchanged. Preserve the v1-only legacy release rejection; update v3 projections and reviews explicitly.
- [x] **Step 4: Run GREEN, replay/upgrade checks, commit.** Run named test, `versioned_content`, `lineage`, `reading`, old bootstraps and strict Clippy. Review authorization order and all public write paths independently before committing.

## Task 5：授权读取、固定位置及孤儿对账

**Files:** Modify `crates/learning-db/src/assets/read.rs`; create `crates/learning-db/tests/asset_reading.rs` and `crates/learning-assets/src/reconcile.rs`; update C1 boundary/execution docs.

**Interfaces:** `AssetStore::read_for_use(actor, AssetUseRef) -> Result<Option<AssetRecord>, ContentError>` takes a precise resource version or block revision, returns no raw blob key for a hidden use, and returns `None` identically for hidden/absent uses. `AssetStore::open_for_use(actor, AssetUseRef)` authorizes that exact use before opening and verifying bytes; callers never reconstruct a file path from metadata. `FsAssetStore::reconcile(protected: &HashSet<String>, older_than: SystemTime)` uses an absolute age cutoff and reports candidates first; destructive deletion is a separately invoked operation after checking active upload and retained-backup protections. `SystemTime` avoids adding a time library to the file-only adapter while retaining the same cutoff semantics.

- [x] **Step 1: Write scenario RED.** Compose two fixed documents sharing H and one figure; insert the figure between personal placements. Move only its placement and assert one asset ID/one original byte object, new position, old fixed Reading unchanged. Revoke and restore actor grants; hidden and nonexistent asset use return indistinguishable external results. Tamper with or remove the file and assert byte access rejects it. Reconcile never proposes active, referenced, or explicitly protected bytes.
- [x] **Step 2: Run RED.** `cargo test --offline --locked -p learning-db --test asset_reading -- --test-threads=1` and `cargo test --offline --locked -p learning-assets --test fs`; record actual failures.
- [x] **Step 3: Implement read guard and dry-run reconciliation.** Resolve the exact use and its current authorization before exposing storage metadata; use the fixed revision/placement rather than current head. Reconcile only lists old unprotected candidates in C1; deletion policy needs separate review before enabling deletion.
- [x] **Step 4: Run GREEN and commit.** Repeat named tests, B4 Attention scenario, all old-version bootstraps and reviewer gate. Document precise C1 behavior and remaining C2–C4 work.

## Task 6：C1 总验收

**Files:** Create/update `docs/p0c1-{boundary,execution,verification}.md`; update top-level [批准记录](../../设计批准与实施路线.md) only after evidence is complete.

- [x] Freeze source manifest (274 files), exact commit `a962ab8faa6abc2506f2f47d3e23ae99e2ee781c` and ZIP SHA-256 `79a24f6f158f25f1edc470956e1232c9bc9b7a38f75b7abedd19986ce9ca55ec`；核对 Linux 主门及 Compose 探针源码前后一致。
- [x] 全新隔离 PostgreSQL 五库执行 `cargo fmt --all -- --check`、`cargo clippy --offline --locked --workspace --all-targets -- -D warnings`、`cargo test --offline --locked --workspace -- --test-threads=1`；工作区 328 通过/0 失败/3 既有忽略，综合夹具 1/1，原始证据见工作树 `docs/p0c1-verification.md`。
- [x] P0-A/B1/B3-schema/B2 四套冻结旧程序在追加迁移上退出 0；另一个全新 Compose 项目核对 PG/evidence/assets/staging 四卷，真实挂载中 A/B 原字节与只读 B 复读通过，PG 五空库/非 owner、健康及停机证据齐全，卷保留。两次控制脚本失败也保留在证据中。
- [x] 独立审查旧契约兼容、精确授权、文件/数据库失败窗口、固定 Reading、上传限额和媒体声明；RED/GREEN 后冻结提交复跑，Linux 主门 78/78、卷探针 39/39、同项目 PG 补证 60/60 原始证据哈希逐项核对。
- [x] 仅标记 `P0_C1_VERIFIED / NOT_PRODUCTION`；C2–C4、P1-A 与生产未完成。由已验证 C1 接口另立 C2 施工单，不实施 C2。

## Execution Handoff

用户选择逐任务子代理实现和独立审查；C1 Task 1–6 已依该方式实施与验收。实际通过结果与保留的控制脚本失败记录以隔离工作树 `docs/p0c1-verification.md` 为准。此施工单不授权实施 C2–C4、P1 或生产部署。
