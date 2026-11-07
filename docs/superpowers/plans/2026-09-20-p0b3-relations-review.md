# P0-B3 关系与认识审查 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans or superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在知织中保存可复现、受当前权限约束的猜想—证据—反例—结论链，同时保留既有内容、阅读、发布和迁移历史。

**Architecture:** PostgreSQL 保存不可变正文、关系修订及两类人工审查。严格类型引用在写事务中建立依赖索引，统一授权闭包用于读取和发布；系统谱系由专用内容操作产生。旧 v1 摘要不变，v2 通过显式版本分派接入。

**Tech Stack:** Rust 1.97 / edition 2024、SQLx 0.8.6、PostgreSQL 18.6；沿用 workspace 现有依赖，不引入 Neo4j、HTTP 或前端依赖。

**Spec:** [已批准设计](../specs/2026-09-20-p0b3-relations-review-design.md)

**Status:** P0_B3_VERIFIED / NOT_PRODUCTION · 2026-09-21。8 项任务已完成逐任务子代理实施及独立审查；专项 12/12，宿主与全新 Compose 各 222/222，四条升级路径通过。工作树 `隔离工作树`，分支 `feat/p0b3-relations`，最终提交 `4252cc812b6c3bb7e7fabbcc309522b005f9df44`。B4 尚未实施。见[验收记录](../../p0b3-verification.md)。

## Global Constraints

- PostgreSQL 为权威库、首版不常驻 Neo4j。
- 正文修订、关系修订和人工审查分别留史，互不覆盖。
- 同端点同范围只保留一个关系身份。
- 隐藏必要依赖时整体正文不可用。
- B3 同时覆盖派生/拆分/合并。
- v1 仍拒绝 basis_refs、target 等未知字段；保留现有规范化字节与摘要。
- 作者、reviewer、时间和新身份由服务端生成；Principal 由受信任调用边界提供。
- 当前权限约束历史读取；不可访问与不存在行为一致。
- 历史数据库实验 NOT_PASSED 不因新实现通过而改写。
- B4 影响遍历、P0-C 资产、P1-A 网页与登录不进入本次提交。

## Review Focus

1. 单块可读但其引用卡三跳以后的证据撤权：不能利用两层展示上限绕过完整授权。Task 3/8。
2. 关系-only 视图变更之后移动个人块或迁移原文：不能丢失已选关系，也不能复制正文。Task 6。
3. 隐藏端点对应重复关系或过期头：错误对象不能携带隐藏身份、摘要或数量。Task 4/8。
4. 同一证据被多次引用且来源运行相同：引用顺序、去重和预算不能改变人工判断或声称独立性。Task 3/5。
5. 新程序在旧库上造升级夹具：可能误把新行为当兼容；必须以冻结旧实现生成证据。Task 8。

## 0. 基线与执行约束

应用仓库为 `仓库根目录`；本施工单和设计在其外层 docs 中。代码路径下文相对应用仓库。基线提交为 `b60056b`，分支 `feat/rust-learning-core`；进入执行时重新确认提交、干净状态及适用 AGENTS.md。如果存在新的用户改动，保留并调整隔离位置。

使用工作树技能选择隔离方式，不能把外层 docs 误当成应用仓库文件。没有 remote，不创建虚构 PR、不推送、不合并。按任务提交本地代码；外层设计/施工单单独保留。

旧验证基线为宿主和 Compose 各 105/105。新增测试数以实际日志为准；禁止提前声明总数。原迁移 `0001`、`0002`、`0003` 原字节不变。

推荐 Native：在本任务内顺序实现，末尾独立审查。各关共享严格引用、授权投影和版本适配接口，顺序执行便于及时发现接口冲突。若用户选择 Subagent-driven，仍按同一依赖次序逐关实施与审查。

本地无 PostgreSQL 配置时只运行核心、纯函数、编译及格式检查，不以忽略数据库测试充当通过。真实数据库测试交给既有任务「本地 Linux 服务器」，任务 ID `01a07ffb-b815-74a0-b832-e0d808ba8f6e`，使用新隔离目录与 Compose 项目；保留 B2 证据和卷。

## 1. 文件责任与依赖顺序

| 任务 | 新增文件 | 修改入口 | 责任 |
|---|---|---|---|
| 1 | core/src/references.rs、content_version.rs；core/tests/versioned_contracts.rs | core/src/lib.rs | 版本化正文与严格引用 |
| 2 | migrations/0004_relations_review.sql；db/tests/relations_schema.rs | db/src/lib.rs | 新对象、依赖、约束和权限 |
| 3 | db/src/references/{mod,closure,projection,index}.rs、versioned_content/{mod,read,write}.rs；db/tests/reference_authorization.rs | db/src/read.rs、block_write.rs、composition/closure.rs、overlay/model.rs | 内容存储和统一授权 |
| 4 | core/src/relation.rs；db/src/relations/{mod,read,write,review}.rs；db/tests/relations.rs、relation_concurrency.rs | core/db 的 lib.rs | 关系及关系审查 |
| 5 | core/src/epistemic.rs；db/src/reviews/{mod,read,write}.rs；db/tests/epistemic.rs | core/db 的 lib.rs | 条件化人工判断 |
| 6 | core/src/reading_selection.rs；db/src/reading/selection.rs；migrations/0005_reading_evidence.sql；db/tests/reading_evidence.rs | reading、overlay/persist、placement_migration、release 模块 | 固定关系选择与发布 |
| 7 | core/src/lineage.rs；db/src/lineage/{mod,read,write}.rs；migrations/0006_lineage.sql；db/tests/lineage.rs | core/db 的 lib.rs | 派生、拆分、合并 |
| 8 | db/tests/b2_upgrade.rs、b3_scenario.rs；docs/p0b3-{boundary,execution,verification}.md | deploy/initdb.sh、run-tests.sh、Dockerfile.test、compose.test.yaml | 冻结版本升级和最终验收 |

表中 core = `crates/learning-core`，db = `crates/learning-db`。新文件按责任拆分，不合并成单个大型 store 文件。

## 2. 跨任务契约

### 2.1 类型和内容版本

使用既有 `BlockRef { block_id, revision_id }`。新增精确引用：

```rust
pub struct RelationRef { pub relation_id: Uuid, pub revision_id: Uuid }
pub struct RelationReviewRef { pub relation: RelationRef, pub review_id: Uuid }
pub struct EpistemicReviewRef { pub stream_id: Uuid, pub review_id: Uuid }
pub enum ExactRef {
    Block(BlockRef),
    Relation(RelationRef),
    RelationReview(RelationReviewRef),
    EpistemicReview(EpistemicReviewRef),
}
pub enum DependencyRole { Basis, Target, RequiresContext, SourceRun, SelectedRelation, SelectedReview }
pub struct Dependency { pub role: DependencyRole, pub target: ExactRef }
pub enum ContentDraft { V1(TextDraft), V2(ContentV2) }
pub struct ContentV2 {
    pub intent: Intent, pub language: String, pub title: String,
    pub body: BodyV2,
    pub basis_refs: Vec<ExactRef>,
    pub requires_context: Vec<BlockRef>,
    pub source_run: Option<BlockRef>,
}
pub enum BodyV2 {
    Text(TextPayload),
    Reference { target: BlockRef },
    RelationView { selections: Vec<RelationSelection> },
}
pub struct RelationSelection {
    pub relation: RelationRef,
    pub review: Option<RelationReviewRef>,
}
```

为严格嵌套对象使用 deny_unknown_fields；版本入口通过显式 Raw DTO 解码，不使用会吞字段的 untagged 回退。新命令 JSON 格式为 `{contract_version: 2, draft: {...}}`；存储列 contract_version 与正文形态严格匹配。旧 content JSON 和摘要完全保持。

`ContentDraft::decode(version: u32, value: serde_json::Value) -> Result<Self, ContentError>`；`validate()`、`digest()`、`dependencies() -> Vec<Dependency>` 是所有消费者共同入口。v2 摘要覆盖 `{domain:"content-v2",contract_version:2,draft:...}` 的 canonical_json 字节，v1 仍调用现有 content_digest。

`ContentRevision` 复制现有 Revision 元数据字段，draft 改用 ContentDraft。保留现有 Revision/TextDraft 和 v1 create/revise 入口；新统一内部模型显式转换，不向旧 v1 客户端伪造文本形式的引用卡。旧返回类型遇可读 v2 时返回 Invalid(unsupported_content_version)，遇不可读 v2 先返回与不存在相同的结果。

新 `VersionedContentStore`：`create(actor, space, CreateContent) -> ContentRevision`、`revise(actor, block, ReviseContent) -> ContentRevision`、`read(actor, BlockRef) -> Option<ContentRevision>`、`read_many(actor, Vec<BlockRef>) -> Vec<ContentRevision>`。CreateContent 包含 request_id/draft/reason；ReviseContent 另含 base_revision_id。两类 store 共享内部事务写入原语和全局请求键。

### 2.2 限额和错误

v2 保留标题 300 Unicode 标量、language 1–35 ASCII、文本 200,000 UTF-8 字节、理由 1–1,000 Unicode 标量；均拒绝 U+0000。条件与审查说明各最多 10,000 UTF-8 字节；非空要求按 trim 后检查但原字节保存。

每个 v2 块的直接依赖最多 256 项；依据、关系选择均拒绝重复精确引用。批量正文读取仍最多 200 输入；一次关系选择最多 256 项、认识审查最多 256 项关系及 256 项证据。来源运行只能是精确 BlockRef。

依赖闭包最多 2,048 个去重对象、4,096 条去重边、最长简单路径 32 层、8 MiB 去重载荷；载荷字节沿用 PostgreSQL `octet_length(jsonb::text)`，关系/审查也计入。授权先于取正文和计数；预算针对一次操作的合并闭包，不能拆成 200 次独立限额放大。

内嵌显示上限独立设为两层。B1 的组合 2,048 对象、4,096 occurrence、16 层及 8 MiB，B2 的 512 组、2,048 placement、32 正文批次和个人 8 MiB 原样保留。新的依赖闭包作为额外预算，不扩大旧限额。

新超限返回 Invalid(reference_budget_exceeded)，不返回对象身份、实际计数或部分成功结果。错误合同：无权/缺失 NotFound；请求键冲突 IdempotencyConflict；普通关系重复 Invalid(relation_exists)；头过期使用现有 Conflict，仅在当前头完整可读时带其 ID，否则 NotFound。SQL 原文、约束名和隐藏端点不能进入对外错误。

### 2.3 存储和锁

新增不可变对象 registry：`reference_object(kind, object_id, revision_id, space_id)`，主键前三列。kind 仅 block/relation/relation_review/epistemic_review。每种实际表在相同事务内登记；延迟约束触发器校验 registry 行确实对应正确空间和身份的真实修订，禁止伪造登记。

registry 身份映射固定为：block 使用 block_id/revision_id；relation 使用 relation_id/revision_id；relation_review 使用其目标 relation revision ID/review_id（另由真实审查行核对 relation_id）；epistemic_review 使用 stream_id/review_id。不能用 review_id 同时冒充稳定身份和记录身份。

`reference_dependency` 保存源对象、role、position、目标对象；两端复合外键均指向 registry。正文、关系、审查的结构化载荷必须与索引完全一致；延迟约束检查类型与依赖清单，故意遗漏或追加索引的原生 SQL 测试必须提交失败。索引是读取加速结构，不能成为可独立篡改的另一份事实。

旧 block_revision 回填 registry，无依赖；新增 contract_version 默认 1。runtime 对新不可变表只有 SELECT/INSERT，不能 UPDATE/DELETE；只对身份表的 head 列开放 UPDATE。保持原 trusted runtime 边界，不宣称已经实现 RLS。

关系身份唯一键 `(space_id, overlay_id, type, from_block_id, to_block_id) NULLS NOT DISTINCT`；overlay 非空时复合 FK 验证层归属空间。related_to 的 from_block_id < to_block_id，所有关系两端身份不同。关系修订以复合 FK 固定两端具体版本，并与身份表固定端点身份匹配。审查前序必须处于同一审查流，不允许跨目标拼接。

| 操作 | 请求和授权锁之后的锁顺序 | 事务写入 |
|---|---|---|
| 新/改正文 | 排序 block ID（修订时） | 修订、registry、依赖、head、回执 |
| 新/改关系 | 规范化关系键 advisory lock → relation head | 关系修订、registry、依赖、head、回执 |
| 关系审查 | relation identity → 对应 revision 的审查头 | 审查、registry、依赖、审查头、回执 |
| 认识审查 | 排序 relation identity → 排序所选关系审查头 → scope/target/actor 流键 advisory lock → review stream head | 审查、registry、依赖、head、回执 |
| 视图关系选择 | overlay → reading_view | 新视图修订、固定选择、回执 |
| 个人层编辑/迁移 | 沿用 overlay → reading_view → 排序 block ID | 保留选择后写新层/视图及既有事务内容 |
| 发布 | 排序 composition ID → 排序 reading_view ID | release、完整依赖清单、发布指针、outbox、回执 |
| 谱系操作 | 新生成输出 block ID 排序 | 输出身份和正文、谱系、依赖、回执 |

发现依赖 → 合并所需空间（输出空间 write，依赖空间 read）→ 排序 lock_grants → 重新解析确认 → 身份头锁。固定引用不可变，锁后不得临时发现新空间再反向补锁；出现集合不一致时失败重试。

读事务使用 READ ONLY REPEATABLE READ，其授权判断以同一事务快照为准；撤权提交后的新读取必须立即遵守新权限，不承诺取消已开始的合法读取。写事务 READ COMMITTED + 授权行锁，继续沿用 10s 锁超时、15s statement_timeout。

## Task 1: 严格内容与关系引用契约

执行备注：已完成，提交 `b4701d7`、`58711bb`。补齐模块拆分和精确上限断言后复审通过；核心 35/35、workspace check、Clippy 与格式检查通过。v1 内容/摘要实现、旧迁移及 Cargo.lock 与基线无差异。

**Files:** 按文件表 Task 1；另新增 core/tests/reference_contracts.rs。

**Interfaces:** 实现 2.1 全部类型、decode/validate/digest/dependencies；本任务只产生纯 Rust 契约，数据库入口仍不接入 v2。

- [x] 新建失败测试，验证旧摘要、未知版本、未知字段、类型错配、引用缺修订、重复引用及预算边界。核心固定断言：

```rust
let v1: TextDraft = serde_json::from_value(serde_json::json!({
    "kind":"text","intent":"note","language":"zh-CN","title":"注记",
    "payload":{"format":"markdown","text":"保留空格  与换行\n"}
})).unwrap();
assert_eq!(v1.digest(), "35db28db30d4e6d1b4f81c32a365ba9d2b34757bbb6694e09ab308baab1037ec");
assert_eq!(ContentDraft::V1(v1.clone()).digest(), v1.digest());
assert!(ContentDraft::decode(99, serde_json::json!({})).is_err());
assert!(serde_json::from_value::<BlockRef>(serde_json::json!({"block_id":Uuid::new_v4()})).is_err());
```

- [x] 运行 `cargo test --offline --locked -p learning-core --test versioned_contracts --test reference_contracts`，保存失败日志；新增类型未定义属于本关预期失败，不能把构建环境错误称为 RED。
- [x] 实现版本分派、严格 DTO、校验、稳定摘要和引用枚举。依赖只从结构字段抽取，不扫描自由文本；数组语义顺序保留，集合型关系选择按完整引用排序后进入命令摘要。
- [x] 运行 core 全部测试及 `cargo check --offline --locked --workspace --all-targets`。旧 API 仍能编译；新契约测试与旧黄金摘要都通过后提交 `feat: define versioned content and exact evidence references`。

## Task 2: 不可变关系、审查和依赖 schema

**Files:** 文件表 Task 2；core/src/relation.rs 与 epistemic.rs 在本关先定义 enum 和存储 DTO，Task 4/5 添加命令行为。

**Interfaces:** 0004 提供 registry/dependency、relation/relation_revision、relation_review_head/relation_review、epistemic_stream/epistemic_review 及其收据。所有固定引用可通过 registry 找到所属空间，不能只凭 UUID 类型猜测对象。

- [x] 写 schema 测试，使用非 owner runtime 直接尝试 UPDATE/DELETE、错空间 FK、伪造 registry、关系反向重复、篡改依赖索引与跨流前序。断言 SQLSTATE 权限拒绝或完整性错误，正常事务能提交。

```rust
let result = sqlx::query("DELETE FROM public.reference_object")
    .execute(&rig.runtime_pool).await;
assert_eq!(sqlstate(&result.unwrap_err()).as_deref(), Some("42501"));
```

- [x] 在隔离 PostgreSQL 上运行 `cargo test --offline --locked -p learning-db --test relations_schema -- --test-threads=1`，保存新表缺失或约束缺失的失败证据。
- [x] 编写 0004；只追加新迁移。扩展 request_key 操作枚举，保留所有旧操作并新增 content_v2/relation_save/relation_review/epistemic_review。建立 deferrable FK 与提交时引用一致性校验，不通过扩大 runtime 权限实现触发器。
- [x] 测试迁移前后 0001–0003 checksum 不变、v1 无依赖可读、runtime 无迁移权限。上述 schema 测试全部通过后提交 `feat: persist immutable relation and review identities`。

## Task 3: 版本化正文与统一授权闭包

**Files:** 文件表 Task 3；新增 db/tests/versioned_content.rs、reference_budget.rs；修改 composition/read.rs、reading/read.rs/projection.rs 的内部正文类型。

**Interfaces:** `references::load(tx, actor, roots: &[ExactRef]) -> Result<AuthorizedClosure, ContentError>`；闭包提供已授权对象与所需空间。`references::project(tx, actor, root) -> Result<Option<AuthorizedObject>, ContentError>` 在对象不存在或必要依赖不可读时返回 None。这些类型仅 crate 可见，不能把原始 registry/载荷泄露给调用方。

AuthorizedObject 包含完整可读的 Block、Relation、RelationReview、EpistemicReview 类型化 DTO。闭包从起点按服务端预算展开，只沿明确依赖，不扫语义邻居；visited 精确对象去重，path 检测展示环。一个不可读必要对象令根整体不可用，不返回已收集的部分正文。

- [x] 写 basis_refs 和 target 两条回归：创建有权限的 E 与 C/引用卡，撤销 E 所在空间，分别调用单项、批量、历史、组合和融合读取；输出不得包含 E 的 ID、正文、摘要或隐藏计数。

```rust
let hidden = store.read(actor, conclusion_ref.clone()).await.unwrap();
assert!(hidden.is_none());
let batch = store.read_many(actor, vec![conclusion_ref]).await.unwrap();
assert!(batch.is_empty());
```

- [x] 运行 `cargo test --offline --locked -p learning-db --test versioned_content --test reference_authorization --test reference_budget -- --test-threads=1`，保存失败证据。
- [x] 将内部 RevisionRow 改为先读版本和 Json<Value> 再显式 decode；新增 VersionedContentStore 与共享 transaction-only insert 原语。v1 写入同事务登记 registry，v2 同事务保存依赖。旧 v1 read/list/history/many 必须先走授权投影，再进行 v1 类型适配。
- [x] 新统一读取分页只以可见对象生成游标和 has_more；候选扫描设上限，超限返回通用预算错误，不返回扫描进度或隐藏数量。读闭包过滤后取正文；保持列表顺序，不因中间隐藏项暴露原序号。零可见结果与空集合一致。
- [x] 将组合和个人阅读内部 block 载荷改为 ContentRevision；新增 versioned 读取入口供新客户端使用，既有 v1 返回 DTO 通过显式适配。可见 v2 必须返回 unsupported_content_version，不能省略它伪装成完整 v1 页面；不可见个人项仍按 B2 策略省略。
- [x] 补多层必要引用、重复菱形引用、所有预算边界与 200 批量合并预算测试；隐藏与不存在结果相同，恢复 grant 后原精确版本恢复。只读依赖不包括 relation 的其他语义邻居，允许语义图环而不无限展开。
- [x] 新增 `VersionedContentStore::preview(actor, BlockRef) -> Option<ReferencePreview>`；ReferencePreview 由完整授权闭包生成，包含当前块及零或一个引用卡目标子节点。第二层之后生成 Link(BlockRef)，重复访问路径中的对象生成 Link，不再内嵌；Link 仅可指向已授权对象。测试第三层隐藏时整个根仍不可用，第三层可读时有链接但无第三层内嵌正文。自由文本 Markdown 不参与此自动预览。
- [x] 运行本关与旧 authorization/assembly_authorization/reading_authorization、核心全量；通过后提交 `feat: authorize versioned content through fixed dependency closures`。

## Task 4: 关系保存、去重和关系审查

**Files:** 文件表 Task 4；新增 core/tests/relation_contracts.rs。

**Interfaces:** `RelationStore::new(pool)`；`save(actor, SaveRelation) -> RelationRevision`；`read(actor, RelationRef) -> Option<RelationRevision>`；`read_selected(actor, Vec<RelationRef>) -> Vec<RelationRevision>`；`review(actor, ReviewRelation) -> RelationReview`；`read_review(actor, RelationReviewRef) -> ReviewProjection<RelationReview>`。

SaveRelation 字段：request_id、scope、relation_id: Option<Uuid>、expected_revision: Option<Uuid>、type、from/to: BlockRef、rationale、conditions。两项 Option 必须同时空或同时非空。scope 为 Space{space_id} 或 PersonalOverlay{space_id,overlay_id}。

ReviewRelation 字段：request_id、relation: RelationRef、expected_previous: Option<Uuid>、state、explanation。每个精确 relation revision 有独立审查流；第一条 expected_previous=None。ReviewProjection 为 Available(T)、Incomplete、Unavailable；Incomplete 不携带审查载荷或依据标识。

- [x] 写关系固定 H@1、显式接纳 H@2、修改关系不改正文、撤回保留旧审查的测试；列出关系摘要及端点摘要分别断言。
- [x] 加并发屏障测试：两个请求同时创建 related_to(A,B) 和 related_to(B,A)，只有一条身份和初始修订；另一方收到 relation_exists，不返回获胜方隐藏 ID。

```rust
let results = tokio::join!(store.save(actor, forward), store.save(actor, backward));
assert_eq!(usize::from(results.0.is_ok()) + usize::from(results.1.is_ok()), 1);
let count: i64 = sqlx::query_scalar("SELECT count(*) FROM relation WHERE space_id=$1")
    .bind(space).fetch_one(&rig.admin_pool).await.unwrap();
assert_eq!(count, 1);
```

- [x] 执行 `cargo test --offline --locked -p learning-db --test relations --test relation_concurrency -- --test-threads=1` 确认 RED。
- [x] 实现规范化键、关系 head CAS、request_key 重放与独立审查流。先核对当前可见性，再返回重复/冲突；端点 identity、scope、type 不允许普通修订改变。expected_previous 精确比较，不以最后写入获胜代替并发控制。
- [x] 对 reviewed 的 supports/opposes 要求非空理由、条件及审查说明；空理由可保存关系且视为 unreviewed。禁止客户端字段 reviewer/created_at/origin=system；八种用户关系以 enum 封闭，系统谱系不在此 enum。
- [x] 测试同键不同操作、旧请求跨头重放、撤权后重放、审查流竞争和事务故障回滚；通过后提交 `feat: save fixed semantic relations and independent reviews`。

## Task 5: 认识审查、固定依据和来源运行

**Files:** 文件表 Task 5；新增 core/tests/epistemic_contracts.rs。

**Interfaces:** `ReviewStore::new(pool)`；`append(actor, AppendEpistemicReview) -> EpistemicReview`；`read(actor, EpistemicReviewRef) -> ReviewProjection<EpistemicReview>`。AppendEpistemicReview 包含 request_id、scope、target: BlockRef、expected_previous: Option<EpistemicReviewRef>、state、relations: Vec<RelationSelection>、evidence: Vec<BlockRef>、conditions、explanation。

流键为 scope + target 精确修订 + actor；允许不同作者的判断并存。前序只用于审计和并发控制，不作为推理依据自动无限追溯；公共输出不返回未授权前序引用。认识状态不写入块真值字段。

- [x] 用真实 store 创建 H@1、H@2、E1、E2、X、来源运行 R；E1/E2 的 source_run 固定同一个 R。测试相同运行分组只返回可见组及成员，不生成 confidence 或 independent 字段。
- [x] 测试把指向 H@1 的关系用于 H@2 必须失败；支持/反驳状态缺对应方向依据、条件或理由必须失败。所选 review 若存在必须属于相同关系修订；withdrawn 不能作为当前新判断的有效支持依据，needs_recheck/unreviewed 不能包装成已完整复核的依据。

```rust
assert!(matches!(reviews.append(actor, mismatched_target).await,
    Err(ContentError::Invalid(_))));
let value = reviews.read(actor, saved.reference.clone()).await.unwrap();
assert!(matches!(value, ReviewProjection::Available(_)));
rig.revoke(actor, evidence_space).await;
let value = reviews.read(actor, saved.reference).await.unwrap();
assert!(matches!(value, ReviewProjection::Incomplete));
```

- [x] 运行 `cargo test --offline --locked -p learning-db --test epistemic -- --test-threads=1` 确认 RED，之后实现 append/read 与原子依赖登记。
- [x] 记录中证据列表必须包含所选支持/反对关系的证据端，目标修订精确相同；未选关系的额外证据允许，但仍须固定可读。supported/refuted 是人工选择的条件判断，不能由关系数量自动产生。
- [x] 新的 supported/refuted 判断要求对应方向至少一项所选关系审查为 reviewed，并在锁定关系审查头后核对该修订当前未处于 withdrawn/needs_recheck。旧 reviewed 记录不是绕过当前撤回的凭证。测试“撤回先提交则新判断拒绝”和“新判断先提交则撤回追加后保留历史判断”两种锁顺序；不用 sleep 猜并发结果，使用屏障及 pg_blocking_pids。历史审查仍固定旧选择，投影不按全库最新审查替换历史选择。
- [x] 创建 C@1，其 basis_refs 固定 H、审查与证据；撤权后 C 整体不可用，审查为 Incomplete；恢复权限后历史记录不变。通过后提交 `feat: record scoped epistemic judgments with fixed evidence`。

## Task 6: 关系选择、融合阅读及发布契约

**Files:** 文件表 Task 6；修改 core/src/reading.rs、release.rs，db/src/release/{read,write}.rs。

**Interfaces:** `ReadingStore::select_relations(actor, overlay_id, SelectRelations) -> ReadingSaved`；SelectRelations 包含 request_id、expected_overlay_revision、expected_reading_view_revision、selections: Vec<RelationSelection>、epistemic_reviews: Vec<EpistemicReviewRef>、reason。新视图快照返回 `ReadingEvidence { selections, epistemic_reviews }` 的当前授权投影。

`PublishEvidence` 包含 request_id、roots: Vec<PublishRoot>、readings: Vec<ReadingRef>、reason。`ReleaseStore::publish_evidence(actor, space, command) -> EvidenceRelease`；EvidenceRelease 包含旧 Release 基本字段、固定 readings、manifest_sha256，读取必须先验证完整闭包。

- [x] 测试只改关系选择：view 修订改变、overlay 修订不变、块/正文数量不变；随后移动个人块、迁移原文、切换 Original/Fused/Personal，选择均不丢失。

```rust
let selected = store.select_relations(actor, before.overlay.overlay_id, command).await.unwrap();
assert_eq!(selected.overlay, before.overlay);
assert_ne!(selected.view, before.view);
assert!(selected.changed_blocks.is_empty());
```

- [x] 运行 `cargo test --offline --locked -p learning-db --test reading_evidence -- --test-threads=1` 确认 RED。
- [x] 0005 给 reading_view_revision 增加 contract_version default 1 与固定选择子表；旧行不回填新摘要。新增 view-only 保存原语；既有 overlay/persist 与 migration/decide 显式复制所选精确版本，不跟随关系头。相同选择提交返回 no_change。
- [x] 审查/关系选择不属于当前原文仍可作为可读历史关联保留；PersonalOverlay 关系只允许该层选择，Space 关系可由拥有读权限的层选择。关系必要端点不可读时省略整个关系投影，不输出原序号或隐藏数；审查降级按 Task 5。
- [x] 旧 publish 处理 v1 组合仍保留旧协议；涉及 v2 内容时要求新 publish_evidence，不能产生没有依赖清单的“成功发布”。新发布把正文、依赖、关系、审查、reading 的精确对象清单排序摘要，并以复合 FK 固定。
- [x] 新发布根最多 16、reading 最多 16；层属 actor 且发布空间有写权限，所有端点依赖当前可读。依赖缺失、预算超限、发布 token 竞争、收据故障均整体回滚。旧 release 原始记录和 outbox payload v1 不改写；新事件显式 payload_version=2。
- [x] 验证旧请求重放与空库/升级读取；通过后提交 `feat: freeze relation selections in reading and release snapshots`。

## Task 7: 原子派生、拆分和合并

**Files:** 文件表 Task 7。

**Interfaces:** `LineageStore::new(pool)`；`apply(actor, output_space, LineageCommand) -> LineageSaved`；`read(actor, operation_id) -> Option<LineageSaved>`。LineageCommand 为 request_id、operation、inputs: Vec<BlockRef>、outputs: Vec<ContentDraft>、reason。LineageSaved 为 operation_id、operation、inputs、outputs: Vec<BlockRef>、author_id、created_at。

operation 只含 Derive、Split、Merge。Derive 为 1 输入/1 输出；Split 为 1 输入/2–32 输出；Merge 为 2–32 个不同块身份输入/1 输出。输出全部新生成稳定身份，顺序与 outputs 命令一致；原始输入顺序进入命令摘要，禁止重复输入。

- [x] 写失败测试：拆分输出是两个不同 block_id，均不同于原块，原版正文和所有旧位置不变；合并/派生限制正确；普通关系创建系统类型反序列化失败。

```rust
let result = store.apply(actor, space, split).await.unwrap();
assert_eq!(result.outputs.len(), 2);
assert_ne!(result.outputs[0].block_id, result.outputs[1].block_id);
assert!(result.outputs.iter().all(|r| r.block_id != original.block_id));
```

- [x] 运行 `cargo test --offline --locked -p learning-db --test lineage -- --test-threads=1` 确认 RED。
- [x] 编写 0006 的 operation/input/output/receipt 表和只读系统谱系投影；输出修订与谱系同事务。谱系不进入用户关系唯一键空间，不产生可编辑 relation head。没有相应内容操作记录的谱系不能提交。
- [x] 输入谱系不自动加入新块的必要语义依据；读谱系自身必须授权全部输入输出，输出正文按自己声明的 basis/context 授权。由此避免“查看改编正文”隐式泄露已隐藏来源身份。
- [x] 使用数据库故障注入在第二输出、谱系、收据处逐一失败，比较操作前后块、修订、依赖和回执数量无变化；测试撤权重放、幂等和输出预算。通过后提交 `feat: create atomic derive split and merge lineage`。

## Task 8: 全链验收、升级与独立审查

**Files:** 文件表 Task 8；外层 docs/设计批准与实施路线.md 与 P0-B roadmap 仅在真实验收通过后更新状态。

**Interfaces:** B3 使用四个独立测试数据库：普通、P0-A 升级、B1 升级、B2 升级。新增 TEST_B2_UPGRADE_ADMIN_DATABASE_URL、TEST_B2_UPGRADE_DATABASE_URL、TEST_B2_FIXTURE_MANIFEST；无凭据测试直接失败，不能 skip。

- [x] 先写 b3_scenario 集成测试：固定 Attention 原文→个人 H→同源 E1/E2 与反例 X→人工审查→C→阅读与发布→H@2→撤回/撤权→恢复→拆分。断言逐步精确引用及历史摘要，不用保存结果再读出的自比较代替预期值。
- [x] 写 b2_upgrade：升级前由冻结 b60056b 工作副本实际运行旧 ContentStore/CompositionStore/ReadingStore/MigrationStore/ReleaseStore，导出精确 IDs、旧返回 JSON、摘要和请求命令为夹具清单。不得用新 store 在旧 schema 上造数据。新程序只负责应用新增迁移并校验/重放旧数据。
- [x] P0-A/B1 现有升级测试同样保留真实旧 schema 的造数路径；若新内部 registry 改造使旧测试程序无法运行旧 schema，改为冻结对应旧实现运行器造数，而不是要求旧库预装新表。冻结代码只用于测试，保存源码提交和包摘要。
- [x] 执行格式、Clippy、纯函数与核心测试，生成待测源码包白名单和 SHA-256 清单。禁止包含 DSN、密码、私钥、整个 .runtime 或无关原课件。

```powershell
$env:CARGO_HOME = Join-Path (Get-Location) '.runtime/cargo'
$env:RUSTUP_TOOLCHAIN = 'stable'
cargo fmt --all -- --check
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo test --offline --locked -p learning-core
```

- [x] 委托既有 Linux 任务在新目录执行宿主全量和空库 Compose；数据库 2 CPU/4 GiB、测试 4 CPU/4 GiB，内部网络、无宿主端口。新测试项目与卷，不删除 B2 证据卷；测试完成停止新容器并保留验收证据。

```sh
cargo test --offline --locked --workspace -- --test-threads=1
docker compose -p learning-system-p0b3-test -f deploy/compose.test.yaml --profile test build
docker compose -p learning-system-p0b3-test -f deploy/compose.test.yaml --profile test up --abort-on-container-exit --exit-code-from test
docker compose -p learning-system-p0b3-test -f deploy/compose.test.yaml stop
```

- [x] 完成独立审查，覆盖 Review Focus 五项、统一授权入口、并发锁序、全部新迁移、旧 API 适配、失败回滚与升级协议。真实缺陷补失败测试再修复；仅缺覆盖的测试如初次 GREEN，明确记录为补覆盖，不伪称 RED→GREEN。
- [x] 必要修复后重跑受影响检查和最终宿主/Compose 全量，归档源码摘要、命令、退出码、实际测试数、审查发现及处置。旧 105 个测试若重构，逐项列出等效覆盖，不靠改名或删除隐藏回归。
- [x] 写 p0b3-boundary、execution、verification 并更新外层路线，提交 `docs: record verified relation and epistemic review milestone`。仅在真实验收与审查都通过时标记 P0_B3_VERIFIED / NOT_PRODUCTION；B4 仍未实施。

## 3. 完成定义与计划自检

设计 §3 → Task 4/5；§4 → Task 1/2/3；§5 → Task 3/4/5/8；§6 → Task 6；§7 → Task 7；§8 → Task 2–8；§9 → Task 8。所有新持久化事实都有固定引用、当前授权、幂等回执和不可变历史规则。

五项 Review Focus 已分别进入对应测试任务。闭包预算与两层展示预算分离；历史审查不自动成为当前审查；正文来源谱系不冒充必要证据；旧发布读取不重算为新协议。

实施者遇到设计范围变化或无法同时满足旧兼容和新授权时，先记录具体冲突并修订方案，不能降低权限合同、改写旧摘要或把不完整读取当验收通过。

执行收尾：首次 Compose 外部 secret 权限准备失败，后续以相同源码在全新 `learning-system-p0b3-test2` 成功重验；失败证据保留。全部 B3 专用容器已停止，五个数据/证据卷保留。最终可执行文件与测试包 e560181f41ccbc0fb5f18f0dcd8a1b0950dfa2dfd6494ccda20a6b3093c80ead 一致；仅验收文档在封包后归档。
