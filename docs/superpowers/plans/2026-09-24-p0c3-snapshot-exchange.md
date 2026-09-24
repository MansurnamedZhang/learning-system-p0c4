# P0-C3 融合快照交换 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把一个精确 Reading 修订及其已授权闭包导出为可验证的版本化目录包，在同一身份的另一隔离实例中幂等、原样、原子地导入；被裁剪的阅读副本始终不可导入。

**Architecture:** `learning-core` 定义独立于存储的 v1 包契约与规范摘要；`learning-db` 在一个只读一致事务内求授权闭包，受限 Worker 复制并交付原件，目标端先验证目录包再以一个写事务导入。所有现有普通写接口、C1 原件文件存储和 C2 租约继续保留；只追加迁移和 C3 模块。

**Tech Stack:** Rust 1.97、edition 2024、serde/serde_json、SHA-256、sqlx 0.8.6/PostgreSQL、现有 `FsAssetStore`、Tokio、Docker Compose 隔离验收。首版交换格式是**普通目录**；不接受 ZIP/TAR 输入，压缩输入一律 `InvalidPackage`，所以不存在解压路径或压缩比绕过。后续压缩传输须单独版本化设计。

**Spec:** [已批准的 C3 详细规格](../specs/2026-09-24-p0c3-snapshot-exchange-design.md)；上位 [P0-C 总设计](../../../../../docs/superpowers/specs/2026-09-23-p0c-assets-jobs-portability-design.md) C3 节。

## Global Constraints

- 基线 `085c37724ae291559fe684ded8e7e29e97e3f93e`，C2 状态 `P0_C2_VERIFIED / NOT_PRODUCTION`。不能改写 0001–0013、冻结旧程序、C2 租约与原件摘要；新 SQL 从 0014 起追加。
- 首版只支持相同 `app_user.id`、`space.id`、预置目标授权和预置其他作者/审查者身份；包不能创建或更新用户、空间、grant。不同身份的模板/克隆不属于原样导入。
- 原样包必须包含完整已授权父/前序链、位置身份与锚、精确关系/审查、必要资产元数据；缺授权或缺原件直接拒绝 `exact_import_v1`，不拼造缺边。`reading_copy_v1` 永不能导入。
- 共享预算：精确引用最多 2048 对象/4096 边/深度 32/正文 8 MiB；组合最多深度 16/4096 occurrences/2048 对象/正文 8 MiB。目录包最多 2048 文件、单个 JSON 8 MiB、JSON 合计 64 MiB、单原件 128 MiB、原件合计 512 MiB；超限显式失败。所有数字作为代码常量与格式文档固定，并在单包范围累计。
- 权威格式 v1 使用规范 UTF-8 JSON：对象键按字节序排序、无多余空白、数值/时间用已定义序列化；每行完整记录单独 SHA-256，manifest 按路径排序记录 `path,size,sha256`，不把作业时间和随机 ID 放入权威摘要。目录只允许程序生成的 ASCII 类型/UUID/SHA 路径、普通文件和固定名单；拒绝符号链接/重解析点、未知文件、碰撞和路径穿越。
- 固定目录路径：`manifest.json`、`objects/<table>/<uuid>[__<uuid>...].json`、`assets/sha256/<前两位>/<64位摘要>`、`validation.json`，可选 `reading.html`/`reading.md`。manifest 的 `files` 收录除自身以外的每个文件。数据库 `timestamptz` 用 UTC RFC3339 微秒精度；所有 UUID 小写连字符格式；JSON 不使用浮点 NaN/Infinity。包文件名和目录结构只由固定 table enum 与已验证身份生成。
- 目标导入新容器的 head 指向包内所选精确修订；已有容器 head/published/last_release 永不改动。完整规范记录相同复用，任一不可变字段不同则全包拒绝；任何失败不得出现半可见阅读。
- C3 不搬运 release、lineage operation、迁移决策、收据、job/outbox/result、用户/空间/grant；C4 处理整库恢复。P1 HTTP/UI 与生产部署均不在本计划。

## Review Focus

1. 可读根背后有隐藏的 `basis`、关系审查或旧父修订：Task 2 必须证明整个原样包拒绝，阅读副本不暴露隐藏 ID/路径/数量。
2. `include_originals=false` 且目标只有相同 SHA 的另一空间原件：Task 5/6 必须拒绝，不创建虚假的 `ready` 资产。
3. 已有相同 revision ID、正文摘要相同但作者/原因/父修订不同：Task 5/6 必须报 `IdentityConflict`，全事务回滚。
4. 源端撤权发生在 DB 清单读取后、复制期间或交付前：Task 4 必须阻断成功结果/交付，不留下可直取的静态路径。
5. 同一包重试、两个导入者并发及目标已有更新 head：Task 6 必须幂等、无半可见状态且不倒退 head。

## 文件与接口地图

| 文件 | 单一责任 |
|---|---|
| `crates/learning-core/src/snapshot.rs` | `SnapshotRequest`、互斥的 Exact/Copy manifest、row/asset-use 类型、固定预算和规范摘要 |
| `crates/learning-core/src/job.rs` | C2 原资产作业不变，增加 `SnapshotExport` 输入的严格版本化解析 |
| `crates/learning-db/src/snapshot/{mod,closure,rows}.rs` | 同一只读事务求精确可见闭包，读取完整行与附属行 |
| `crates/learning-db/src/snapshot/{copy,import}.rs` | 安全阅读副本；目标身份/闭包验证及单事务精确导入 |
| `crates/learning-assets/src/snapshot_package.rs` | 目录包文件/资产复制、路径约束、流式哈希、staging 与原子发布 |
| `migrations/0014_snapshot_jobs.sql` | C3 事件与作业载荷白名单、交付记录/最小 runtime 权限；不改旧迁移 |
| `migrations/0015_snapshot_import.sql` | 专用导入批次收据及最小 runtime 权限；不修改 0014 |
| `crates/learning-db/src/snapshot/job.rs` | 导出请求、租约处理、当前授权复核、交付复核 |
| `crates/learning-worker/src/main.rs` | 根据 job kind 分发 C2 与 C3，保持受限角色/租约 |
| `crates/learning-db/tests/snapshot_*.rs` | 真实 PG 权限、闭包、作业、导入和往返测试 |
| `crates/learning-assets/tests/snapshot_package.rs` | 真文件系统路径/哈希/中断测试 |
| `docs/p0c3-{format,boundary,execution,verification}.md` | v1 格式、非备份边界、隔离验收与原始证据 |

## Task 1：v1 类型、预算与规范包契约

**Interfaces:**

```rust
pub struct SnapshotRequest {
    pub reading: ReadingRef,
    pub mode: ReadingMode,
    pub include_personal: bool,
    pub include_originals: bool,
    pub resource_versions: Vec<ResourceVersionRef>,
    pub source_segments: Vec<SourceSegmentRef>,
}
#[serde(tag = "capability", rename_all = "snake_case")]
pub enum SnapshotManifest {
    ExactImportV1(ExactSnapshotManifest),
    ReadingCopyV1(ReadingCopyManifest),
}
pub struct ExactSnapshotManifest {
    pub format_version: u32,
    pub root: ReadingRef,
    pub files: Vec<SnapshotFile>,
    pub requires_destination_assets: bool,
}
pub struct SnapshotFile { pub path: String, pub size: u64, pub sha256: String }
pub struct ReadingCopyManifest {
    pub format_version: u32,
    pub copy_id: Uuid,
    pub files: Vec<SnapshotFile>,
}
pub enum SnapshotTable { Asset, Resource, ResourceVersion, SourceSegment,
    Block, BlockRevision, BlockAssetUse, Composition, CompositionRevision,
    CompositionOccurrence, Overlay, OverlayRevision, OverlayGroupIdentity,
    OverlayPlacementIdentity, OverlayGroup, OverlayPlacement,
    PlacementManualDecision, ReferenceObject, ReferenceDependency, Relation,
    RelationRevision, RelationReviewHead, RelationReview, EpistemicStream,
    EpistemicReview, ReadingView, ReadingViewRevision,
    ReadingRelationSelection, ReadingEpistemicSelection }
pub struct SnapshotRow {
    pub table: SnapshotTable,
    pub identity: Vec<Uuid>,
    pub immutable_values: serde_json::Value,
    pub sha256: String,
}
pub struct SnapshotAssetUse {
    pub use_ref: AssetUseRef,
    pub asset: AssetRef,
    pub sha256: String,
    pub byte_size: u64,
    pub storage_key: String,
}
pub struct SnapshotBudget {
    objects: usize, edges: usize, depth: usize,
    body_bytes: usize, occurrences: usize,
}
pub struct ReadingCopy {
    pub items: Vec<CopyItem>,
    pub evidence: Vec<String>,
}
pub enum CopyItem {
    Heading { title: String },
    Content { intent: String, title: String, display_text: String },
    Omitted,
}
pub fn canonical_record_hash(value: &serde_json::Value) -> String;
```

`ResourceVersionRef` 已在 `learning-core::asset`，`SourceSegmentRef` 目前在 `learning-db::assets`；此任务把后者的纯身份类型移到 core 并从 db 重导出，保持现有路径兼容。`ReadingCopyManifest` 只有新生成的本地 `copy_id`，没有精确 root 字段。`ReadingCopy` 只有文本和固定 `Omitted` 标记，没有原身份；同一不可见组件只给一个标记，不能按隐藏对象逐个输出。`SnapshotRow.immutable_values` 对修订/附属行包含全部列；对 block/composition/overlay/reading/relation/stream 等容器按规格中的逐表身份字段编码，明确排除可变 head/published/last_release。现有容器只比这些不变字段，不因后续 head 不同误判冲突。`reference_object`/`reference_dependency` 是精确派生索引，包中逐行记录期望值；导入时 block/relation/review 触发器生成 registry，显式插入依赖行，在事务末核对两者。`SnapshotRow` 和 `SnapshotAssetUse` 定义在 core，供 DB 与 assets crate 共用；`learning-assets/Cargo.toml` 追加对 `learning-core`、`serde_json` 的依赖，不形成循环。

- [ ] **Step 1: RED。** 在 `crates/learning-core/tests/snapshot_contract.rs` 写 `canonical_hash_changes_for_author_parent_reason`、`unknown_version_or_capability_rejected`、`copy_has_no_exact_root`、`budget_is_package_wide`；固定两份只差 `author_id` 的完整行，断言摘要不同，错误格式无法反序列化。运行 `cargo test -p learning-core --test snapshot_contract`，保存缺少接口的编译 RED。
- [ ] **Step 2: GREEN。** 增 `snapshot.rs` 与 `lib.rs` 导出，复用 `digest::canonical_json`/`hex_digest`，用 `#[serde(deny_unknown_fields)]` 与 tagged enum 解析；预算常量定义上述数值，`SnapshotBudget::charge(kind, bytes, depth)` 用 `checked_add` 防溢出并跨所有根共享实例。运行 Task 1 专项和 `cargo fmt --all -- --check`。
- [ ] **Step 3: 审查与提交。** 独立 reviewer 核对 copy 无精确 root、规范 JSON 与现有内容摘要互不替代；提交 `feat(snapshot): define versioned package contract`。

## Task 2：同事务授权闭包与完整行提取

**Interfaces:**

```rust
pub struct SnapshotStore { pool: sqlx::PgPool }
impl SnapshotStore {
    pub async fn plan_exact(&self, actor: Principal, request: &SnapshotRequest)
        -> Result<SnapshotPlan, ContentError>;
}
pub struct SnapshotPlan {
    pub manifest: ExactSnapshotManifest,
    pub rows: Vec<SnapshotRow>,
    pub assets: Vec<SnapshotAssetUse>,
}
```

`SnapshotRow` 是 `{table, identity, complete_values, sha256}` 的严格类型化记录，不接受任意表名；`SnapshotAssetUse` 保留精确 `AssetUseRef` 与元数据。`plan_exact` 的所有 SQL 从 `request::begin_read` 创建的**一个** `&mut Transaction` 进入，不能调用另开事务的 public `read_versioned`、`read_for_use`。

- [ ] **Step 1: RED。** 在 `snapshot_closure.rs` 用 `TestRig` 建固定 Reading（含 unplaced group、v2/v3 basis、v3 figure/attachment、选择的关系及审查、前序修订），断言按精确版本收齐 identity/anchor/parent/evidence/use 和 `reference_object`/`reference_dependency`/`relation_review_head`。另测隐藏 basis/审查/祖先、跨空间资源根、按同 SHA 的未选资源、两个根合计超预算；失败不得返回 `SnapshotPlan`。运行 `cargo test -p learning-db --test snapshot_closure -- --test-threads=1`，记录缺接口 RED。
- [ ] **Step 2: GREEN。** 从 `reading/read.rs`、`overlay/model.rs`、`assets/read.rs` 提取仅 crate 内可见的 `*_in_tx(&mut Transaction,...)` 查询；重用 `references::Session` 和 `composition::closure` 的授权，但由单一 `SnapshotBudget` 计总量。显式查 `block_revision.parent_revision_id`、`composition_revision.parent_revision_id`、`overlay_revision.parent_revision_id`、`reading_view_revision.parent_revision_id`、`relation_revision.parent_revision_id`、两个 review 的 `previous_review_id` 直到根；每步用原 actor 当前授权复核。按主外键把 identity、group/placement、选择索引、`block_asset_use`、`reference_object`、`reference_dependency` 与 `relation_review_head` 精确读出，`ORDER BY` 固定。任何隐藏/缺行统一 `NotFound` 或包级不可导出错误，不带私有 ID。
- [ ] **Step 3: 验证与审查。** 跑 `snapshot_closure`、`reading_authorization`、`reference_authorization`、`reading_evidence_closure` 和严格 Clippy；独立 reviewer 检查没有第二个事务、没有 raw JSON 泄露、预算不按根重置；提交 `feat(snapshot): plan authorized exact closure`。

## Task 3：目录包、原字节与阅读副本

**Interfaces:**

```rust
pub struct SnapshotDirectory { path: std::path::PathBuf, manifest_sha256: String }
impl SnapshotDirectory {
    pub fn manifest_sha256(&self) -> &str;
    pub fn open_verified_files(&self)
        -> Result<Vec<(String, std::fs::File)>, SnapshotIoError>;
}
pub enum SnapshotIoError { InvalidPackage, MissingAsset, CorruptAsset, LimitExceeded, Io(std::io::Error) }
pub fn stage_snapshot(root: &std::path::Path, job_id: Uuid,
    manifest: &ExactSnapshotManifest, rows: &[SnapshotRow],
    assets: &[SnapshotAssetUse], files: &FsAssetStore)
    -> Result<SnapshotDirectory, SnapshotIoError>;
pub fn stage_reading_copy(root: &std::path::Path, copy_id: Uuid,
    manifest: &ReadingCopyManifest, copy: &ReadingCopy)
    -> Result<SnapshotDirectory, SnapshotIoError>;
pub fn stage_incoming(root: &std::path::Path,
    files: impl Iterator<Item = (String, Box<dyn std::io::Read>)>)
    -> Result<SnapshotDirectory, SnapshotIoError>;
pub fn verify_snapshot(stage: &SnapshotDirectory)
    -> Result<VerifiedSnapshot, SnapshotIoError>;
pub fn sanitize_reading_copy(projection: &VersionedReadingProjection,
    mode: ReadingMode, include_personal: bool) -> Result<ReadingCopy, ContentError>;
```

`stage_snapshot` 接受 core 类型，DB 层传拆开的字段；assets crate 不引用 `SnapshotPlan`。`SnapshotDirectory` 不公开路径或构造器，只能由导出器或受控文件流写入仅服务身份可写的 0700 私有 staging 并封存后得到；`verify_snapshot` 不接受调用者提供的路径。它提供只读 `open_verified_files()` 给 DB 交付接口。Linux 操作外部目录的适配器逐项使用目录句柄相对、no-follow 打开，Windows 只接受已经由服务安全写入的私有 staging，不直接递归读取外部目录。`FsAssetStore` 增仅供已授权调用者使用的 `copy_verified(storage_key, sha256, size, target)` 内部方法，二次流式核哈希。

- [ ] **Step 1: RED。** 在 `learning-assets/tests/snapshot_package.rs` 验合法确定性目录、缺失/损坏资产、同名额外文件、symlink/Windows reparse、`../`/绝对路径/大小写碰撞、超预算、写一半进程中断后的非交付状态；并发把输入普通文件替换成 symlink 时，不得读到链接目标。在 `learning-db/tests/snapshot_copy.rs` 验 Original/Fused/Personal 显示与泛化 omitted marker，无隐藏 ID、嵌套位置 ID、路径、计数、原 revision ID；Original/no-personal 不出现个人 evidence。运行两套专项，记录缺接口 RED。
- [ ] **Step 2: GREEN。** `snapshot_package.rs` 从固定种类+UUID+SHA 构造路径，不拼接包内任意 path 到磁盘；输入先从调用者已打开的普通文件句柄流入 0700 私有 staging，绝不在 `symlink_metadata` 后再凭外部路径打开同一文件，封存期间禁止其他进程写。Linux 外部目录适配器使用目录句柄相对与 `O_NOFOLLOW` 打开每项；Windows 对外部目录路径直接拒绝，只接受流输入。使用 create-new 暂存、`sync_all`、目录同步和原子 rename。对权威 JSON 使用 Task 1 规范字节，逐文件计算 SHA-256/长度后再写 manifest 和 `validation.json`；`verify_snapshot` 重算并比对，拒绝 ZIP/TAR 魔数与未知条目。`stage_reading_copy` 写独立 copy manifest。`sanitize_reading_copy` 从 projection 逐字段白名单映射到无原身份的 DTO，按 mode/include_personal 剔除证据；HTML escaping 与 Markdown 安全链接处理用户正文，不序列化原 projection。
- [ ] **Step 3: 验证与审查。** 跑两套专项、C1 资产读取/损坏回归和格式/Clippy；独立 reviewer 尤其检查 TOCTOU、符号链接、短读和 copy 泄露；提交 `feat(snapshot): stage and validate directory packages`。

## Task 4：追加 C3 作业与撤权交付

**Interfaces:**

```rust
impl JobInput {
    pub fn snapshot_export(actor_id: Uuid, space_id: Uuid,
        request: SnapshotRequest) -> Result<Self, ContentError>;
}
impl SnapshotStore {
    pub async fn enqueue_export(&self, actor: Principal, request_id: Uuid,
        request: SnapshotRequest) -> Result<Uuid, ContentError>;
    pub async fn deliver_export(&self, actor: Principal, job_id: Uuid)
        -> Result<SnapshotDelivery, ContentError>;
}
pub struct SnapshotDelivery {
    pub manifest_sha256: String,
    pub files: Vec<(String, std::fs::File)>,
}
```

`SnapshotDelivery` 是受控读取对象，包含 manifest SHA 与已打开的普通文件句柄清单，不将暂存目录路径当授权令牌。`enqueue_export` 同事务写 C3 `job_outbox` 与请求幂等记录；`process_snapshot_export` 取 C2 当前租约，执行 Task 2/3。完成前在同一写事务内重新授权完整清单、按稳定顺序锁住全部 grant，然后以当前租约 token 插入结果/提交成功，沿用 C2 `AssetIntegrityProcessor::publish` 的撤权排序。交付前再用新事务重验当前授权。暂存目录路径不在对外可读静态根。`JobInput` 改为 `AssetIntegrity { actor_id, space_id, block: BlockRef } | SnapshotExport { actor_id, space_id, request: SnapshotRequest }`；原 asset JSON 和 business key 不变，`actor_id()`/`space_id()` 仍适用于两类，`block()` 改为 `asset_block() -> Option<&BlockRef>` 并更新 C2 处理器、现有调用测试和验收夹具以显式拒绝错误种类。0014 用 `CREATE OR REPLACE FUNCTION` 让旧 `p0c2_claim_job` 对 C3 种类始终返回空，新 Worker 使用限定 C3 种类的新领取函数；旧/新 Worker 并存也不能让旧 Worker 消耗 C3 attempt。

- [ ] **Step 1: RED。** `snapshot_jobs.rs` 测 0014 之前无 C3 种类、0014 后旧 asset 事件仍合法、C3 非法 payload/不同选项同 request key 被拒；旧 C2 Worker 扫描并尝试领取 C3 job 得到空且 attempt 不增加，新 Worker 可领取。注入清单后撤权、复制时撤权、grant 锁前/锁后的并发撤权、成功后交付前撤权、租约易主，断言无可交付路径/旧 token 不能完成。先运行 `cargo test -p learning-db --test snapshot_jobs -- --test-threads=1` 取得 RED。
- [ ] **Step 2: GREEN。** 在 `migrations/0014_snapshot_jobs.sql` 增严格 C3 payload 验证与交付结果表/最小 grant；扩展 `job_outbox`/payload CHECK、`request_key.operation` 白名单，保留旧校验分支。替换 `p0c2_claim_job` 函数体时限定旧事件种类，另增 `p0c3_claim_snapshot_job`，两者复用数据库时钟与 fencing 规则。修改 `job.rs`、`jobs.rs`、`worker/main.rs` 的类型分发。完成结果与 job 成功状态在同事务并受 token 约束，且同事务持有全部必需 grant 锁；交付时通过 `SnapshotStore::plan_exact` 复核，验证包哈希后返回受控文件句柄，不暴露目录路径作为令牌。撤权后失效/过期包只可按非公开 staging 的 job ID 保守回收。
- [ ] **Step 3: 验证与审查。** 跑 `snapshot_jobs`、`jobs_schema`、`jobs_dispatch`、`jobs_lease`、`jobs_processor`、`cargo test -p learning-worker` 和严格 Clippy；独立 reviewer 核对 SQL CHECK 与 Rust decoder 同意、租约 fencing、交付时的当前授权；提交 `feat(snapshot): run fenced authorized export jobs`。

## Task 5：导入预检、身份与资产存在性

**Interfaces:**

```rust
pub struct SnapshotImportStore { pool: sqlx::PgPool, files: FsAssetStore }
pub struct PreparedSnapshotImport {
    manifest: ExactSnapshotManifest,
    rows: Vec<SnapshotRow>,
    assets: Vec<VerifiedImportAsset>,
}
struct VerifiedImportAsset { reference: AssetRef, sha256: String, byte_size: u64,
    source: Option<std::path::PathBuf> }
impl SnapshotImportStore {
    pub async fn validate_exact(&self, actor: Principal, stage: &SnapshotDirectory)
        -> Result<PreparedSnapshotImport, ContentError>;
}
```

`PreparedSnapshotImport` 包含已验证 manifest、完整行与资产文件句柄/元数据；不向外暴露可伪造的 constructor。校验只读目标数据库与目录包，不改权威数据。`reading_copy_v1` 在入口就拒绝。

- [ ] **Step 1: RED。** `snapshot_import_validation.rs` 测未知版本/种类、篡改完整行但保持业务 content hash、缺父链/引用、缺 `reference_dependency`/`relation_review_head`、同 ID 其他作者/原因、目标未预置 actor/space/作者/审查者、跨空间私有目标、缺原件或只有另一空间同 SHA、坏路径和 symlink；每种错误核对目标 DB 权威行计数不变。运行专项得到 RED。
- [ ] **Step 2: GREEN。** 在 `snapshot/import.rs` 严格调用 `verify_snapshot`，验证每行身份/完整记录哈希/业务摘要、FK 与依赖闭包、`reference_object`/`reference_dependency`/`relation_review_head` 和目标授权；`include_originals=false` 时用**同 space+asset ID** 的现有 ready 行和 `FsAssetStore::open_record` 验字节，不以 SHA 跨空间搜索。枚举所有被引用 author/reviewer，目标 `app_user` 必须已存在；actor 必须持目标写 grant 且为 overlay owner/epistemic actor。使用固定 SQL 白名单读目标已有行并规范字节比对，不接受包自带 grant。在 `learning-core/src/error.rs` 加 `IdentityConflict`，不在对外错误文本中带出隐藏对象身份。
- [ ] **Step 3: 验证与审查。** 跑专项、C1 `asset_authorization`、`reference_authorization` 和严格 Clippy；独立 reviewer 检查预检不会修改 DB、错误不泄私有身份；提交 `feat(snapshot): validate exact imports before publication`。

## Task 6：单事务精确导入与并发幂等

**Interfaces:**

```rust
pub struct SnapshotImportReceipt { pub manifest_sha256: String, pub reused: bool }
impl SnapshotImportStore {
    pub async fn import_exact(&self, actor: Principal, request_id: Uuid,
        prepared: PreparedSnapshotImport) -> Result<SnapshotImportReceipt, ContentError>;
}
```

`request_id` 与 manifest SHA 做幂等键；同 request_id 不同 manifest 拒绝。目标不写普通业务 receipt；Task 6 只追加 `migrations/0015_snapshot_import.sql`，建专用 `snapshot_import_batch`（actor、request、manifest SHA、status），不修改已应用 0014。包内不可变记录导入顺序：资产→块与 revision/asset use→组合 revision/occurrence→overlay identity/revision/group/placement→关系及 revision→`relation_review_head` 身份→review/epistemic→reading view/revision/selection→`reference_dependency`。`reference_object` 由现有 block/relation/review trigger 生成；在事务末对照包内期望 registry 行和依赖行。循环 FK 在一个事务内延迟检查。任何隐式触发器要求不能通过跳过或关闭触发器规避。

- [ ] **Step 1: RED。** `snapshot_import_atomic.rs` 在两套空库上做 Attention 精确往返、二次导入、两个并发导入、已有目标 head 较新、同 revision ID 不同完整字节、导入到一半注入 SQL/FK 失败；检索所有相关表确认失败后无半可见 reading、无额外 head 更新。同一包重试返回同一 manifest receipt。运行专项取得 RED。
- [ ] **Step 2: GREEN。** 先把包内原件经现有 `FsAssetStore` 内容寻址写法持久化并复核，失败 orphan 只报告。DB 开事务并使用 actor+manifest 的 advisory lock；事务内重新核对授权并锁住所有必需 grant，元数据包在提交前再核目标真实资产字节。针对固定表名单 `INSERT ... ON CONFLICT DO NOTHING` 后 `SELECT` 全部不可变字段比对；插入子行与 `reference_dependency`，确认触发器生成的 `reference_object` 和包期望一致。在提交前 `SET CONSTRAINTS ALL IMMEDIATE`，任何不一致回滚。新容器设置选中 revision head；已有容器只验证身份不 UPDATE head/published。写导入 batch receipt 与数据同提交，返回 `reused`。
- [ ] **Step 3: 验证与审查。** 跑专项、`reading_evidence_replay`、`relations_schema`、`overlay_schema`、C1 asset tests 与严格 Clippy；独立 reviewer 核对所有表白名单、每项冲突全字段比较和失败回滚；提交 `feat(snapshot): import exact revisions atomically`。

## Task 7：隔离端到端验收与交付文档

- [ ] **Step 1: 固定源码。** 记录精确 commit、clean status 和逐文件 SHA；Task 1–6 中任何 RED/GREEN 中间包或最终源码包送往 Linux 之前，都按用户既定“每份新包单独确认”的边界给出完整路径、摘要、文件数与隔离目标。未获该包授权前仅运行本地不需要服务器的检查。绝不借用 C2 包授权。
- [ ] **Step 2: 本地静态验证。** 运行 `cargo fmt --all -- --check`、`cargo clippy --offline --locked --workspace --all-targets -- -D warnings`；只有环境具备显式隔离 `TEST_ADMIN_DATABASE_URL` 和 `TEST_DATABASE_URL` 时才运行 PG 专项，不把缺少 DSN 记为产品失败。冻结 0001–0013 文件 hash 与 C2 基线比对。
- [ ] **Step 3: 经授权的 Linux 隔离验收。** 新 Compose project/网络/卷/数据库，执行 `cargo test --offline --locked --workspace -- --test-threads=1`、C3 四种失败注入、两个独立 Worker 进程 SIGKILL/租约易主、撤权后交付、同身份第二实例往返、四套旧程序升级；保存命令、stdout/stderr、退出码、SQL 状态和关键原字节/包哈希。验证容器只有 runtime secret，无管理员 secret。
- [ ] **Step 4: 文档与终审。** 写 `docs/p0c3-format.md`（精确字段和目录上限）、`docs/p0c3-boundary.md`、`docs/p0c3-execution.md`、`docs/p0c3-verification.md`；逐条比对上方 Spec 与 Review Focus。独立 reviewer 做全分支审查；只有全部真实证据通过才标记 `P0_C3_VERIFIED / NOT_PRODUCTION`，否则逐项列出未验证项。C4/P1/生产保持开放。

## 执行交接

用户已选择逐任务由子代理实现并审查。书面计划获用户审阅后，按 Task 1→7 顺序：新 implementer 执行 RED/GREEN，独立 reviewer 对照规格和测试审查，修复后才进入下一任务；最后全分支复审。每次只提交已通过本地可运行检查的独立任务。Linux 新源码包按精确文件和目标单独确认，不把 C2 的历史授权扩展到 C3。
