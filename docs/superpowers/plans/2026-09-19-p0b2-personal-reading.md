# P0-B2 个人插入层与固定阅读视图 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans 实施本计划，逐任务验证，最后一次独立代码审查。若用户明确选择逐任务子代理方式，则使用 superpowers:subagent-driven-development。执行步骤以 `- [ ]` 跟踪。

**Goal:** 在固定原文间隙保存独立个人内容，使正文、位置、阅读版本原子更新，并支持可追溯的原文升级迁移。

**Architecture:** 在现有两个 Rust crate 内增加个人层、阅读投影和迁移模块。PostgreSQL 保存不可变版本与精确引用；读输出单独进行当前授权投影，不能直接序列化数据库记录。沿用 B1 请求登记、固定闭包、授权锁和非所有者运行角色。

**Tech Stack:** Rust 1.97.0、Serde、Tokio、SQLx 0.8.6、PostgreSQL 18.6；沿用 Cargo.lock 和已验证的 Linux/amd64 镜像，无新运行依赖。

**Spec:** [B2 书面设计](../specs/2026-09-19-p0b2-personal-reading-design.md)、[P0-B 总单](2026-09-19-p0b-content-assembly-roadmap.md)、[B1 边界](../../p0b1-boundary.md)。用户在书面设计及施工单交付后要求“推进吧”，已完成实施和验证。

**Status:** `P0_B2_VERIFIED`，2026-09-19。用户要求“推进吧”后执行全部六项任务，宿主与全新 Compose 各 105/105，独立审查三项缺陷均 RED→GREEN 修复。[验收记录](../../p0b2-verification.md) · [执行取舍](../../p0b2-execution.md)。

## Global Constraints

- “一个个人层绑定一个稳定根文档 composition_id，可在该文档不同修订之间显式迁移；跨文档整理使用专题组合。”
- “从融合阅读保存个人正文时，在同一事务产生块新修订、个人层新修订及阅读视图新修订；旧视图继续引用旧版正文。”
- “没有正文变化的移动或重排不产生块修订。”
- “Rust 内核 + PostgreSQL，新增迁移，不修改 0001/0002；现有 content-v1、B1 接口和原测试保持兼容。”
- “同一基础层上的两个修改只能一个成功，即便修改不同间隙。”
- “一个迁移提案最多处理该层的 512 组，只比较指定两个固定基线，不全库搜索候选。”
- 最多 512 组、2,048 个人 placement、每次 32 个正文创建/修订、个人去重 JSONB 正文 8 MiB；原文闭包预算沿用 B1，两个正文预算独立。
- 当前只接受现有 TextDraft；图片资产、谱系关系、HTTP/前端不纳入本关。
- 每阶段保留行为 RED/GREEN；缺依赖、编译错误和环境连接失败不能作为业务 RED。
- 所有新命令拒绝未知字段；作者来自 Principal，时间由服务端产生；隐藏和不存在对象使用相同外部行为。

## Review Focus

1. 原文被撤权后，原始层摘要、锚点、迁移原因和旧序号可能泄露隐藏结构：Task 4/6 对序列化输出逐项验证，保留自己的可读正文。
2. 从隐藏个人块前后返回未经压缩的序号或邻居 ID：Task 4 只返回可见连续顺序；原始写入状态不可用时不伪造可提交的完整层。
3. 一条笔记在同层两处出现，修订时错误升级全部引用：Task 3 只更新显式 selected_placements，历史视图和其它层保持固定。
4. 两组迁移撞到同一间隙，合并后旧组身份丢失或顺序随机：Task 5 要求显式合并顺序、记录旧组→结果组映射、保留各旧组内部次序。
5. 当前层已编辑后重放旧迁移/保存：Task 3/5/6 先按当前权限检查原收据，再判断新请求的预期头；不可把合法重放误判成过期。

---

## 1. 文件责任与依赖顺序

以下路径相对 `learning-system/`。只抽取 B2 需要的事务内写入，不重构无关模块。

| 文件 | 责任 |
|---|---|
| `crates/learning-core/src/overlay.rs` | 位置、出现、组、编辑命令、结构校验 |
| `crates/learning-core/src/reading.rs` | 固定视图引用、安全读取 DTO、保存结果 |
| `crates/learning-core/src/placement_migration.rs` | 提案命令、组处置、合并与决定 DTO |
| `crates/learning-core/src/lib.rs`、`error.rs` | 导出及 ReadingConflict |
| `crates/learning-db/src/block_write.rs` | 调用方持有事务/权限/锁的正文插入原语 |
| `crates/learning-db/src/write.rs` | P0-A 调用同一正文原语，原摘要和收据保持不变 |
| `crates/learning-db/src/overlay/{mod,model,anchor,edit,persist}.rs` | 内部记录、位置验证、纯结构变换、落库 |
| `crates/learning-db/src/reading/{mod,write,read,projection}.rs` | 原子保存、写入基础恢复、当前授权投影 |
| `crates/learning-db/src/placement_migration/{mod,propose,decide,read}.rs` | 持久化提案、决定、带权限的历史读取 |
| `crates/learning-db/src/request.rs`、`lib.rs` | 复用请求锁/登记，导出新 store |
| `migrations/0003_personal_reading.sql` | 个人层、视图、提案/决定、收据和 ACL |
| `crates/learning-core/tests/reading_contracts.rs` | 严格 JSON、限制、摘要 |
| `crates/learning-db/tests/{overlay_schema,overlay,reading,placement_migration,reading_authorization,reading_concurrency,b1_upgrade}.rs` | 各任务的真实数据库回归 |
| `crates/learning-db/tests/support/reading.rs` | 公开接口夹具及限定作用域故障注入 |
| `contracts/{reading-create,reading-edit,migration-decide}.example.json` | 与最终命令一致的可反序列化样例 |
| `deploy/{initdb,run-tests}.sh` | 增加独立 B1 升级库，保留 P0-A 升级测试 |
| `docs/{p0b2-boundary,p0b2-verification,p0b2-execution}.md`、`README.md` | 实际边界、证据、取舍与运行说明 |

顺序为契约 → schema/正文原语 → 原子编辑 → 权限读取 → 迁移 → 联合验收。Task 3 可先用安全且严格的全闭包读取验证保存；Task 4 再开放个人内容的部分可见投影，不能先暴露内部结构。

## 2. 公共合同

所有 UUID 使用 Uuid，时间使用 DateTime<Utc>，错误使用 ContentError。命令类型用 Raw + TryFrom 和 validate 保持 JSON/直接 Rust 构造双入口一致。以下类型的字段均为 pub；内部权威记录不实现面向外部的 Serialize。

```rust
pub struct OverlayRef { pub overlay_id: Uuid, pub revision_id: Uuid }
pub struct ReadingRef { pub view_id: Uuid, pub revision_id: Uuid }
pub enum Affinity { AfterLeft, BeforeRight }
pub struct GapAnchor {
    pub base: CompositionRef,
    pub parent_occurrence_path: Vec<Uuid>,
    pub left_occurrence_id: Option<Uuid>,
    pub right_occurrence_id: Option<Uuid>,
    pub affinity: Affinity,
}
pub struct Placement { pub placement_id: Uuid, pub block: BlockRef }
pub struct PlacementOrder {
    pub left_placement_id: Option<Uuid>,
    pub right_placement_id: Option<Uuid>,
}
pub enum InsertTarget {
    ExistingGroup { group_id: Uuid, order: PlacementOrder },
    NewGroup { anchor: GapAnchor },
}
pub struct CreateReading {
    pub request_id: Uuid, pub base: CompositionRef,
    pub title: String, pub reason: String,
}
pub struct TextChange {
    pub block_id: Uuid, pub base_revision_id: Uuid,
    pub draft: TextDraft, pub selected_placements: Vec<Uuid>,
}
pub enum ReadingEdit {
    InsertExisting { block: BlockRef, target: InsertTarget },
    InsertNew { drafts: Vec<TextDraft>, target: InsertTarget },
    ReviseSelected { changes: Vec<TextChange> },
    AdoptExisting { block: BlockRef, selected_placements: Vec<Uuid> },
    Move { placement_id: Uuid, target: InsertTarget },
    Remove { placement_id: Uuid },
    PlaceUnplaced { group_id: Uuid, anchor: GapAnchor,
        merge_into: Option<Uuid>, merged_order: Vec<Uuid> },
}
pub struct EditReading {
    pub request_id: Uuid,
    pub expected_overlay_revision: Uuid,
    pub expected_reading_view_revision: Uuid,
    pub edit: ReadingEdit, pub reason: String,
}
pub struct ReadingSaved {
    pub overlay: OverlayRef, pub view: ReadingRef,
    pub changed_blocks: Vec<BlockRef>,
}
```

Store 接口固定如下；三个 store 均通过 `new(PgPool)` 建立，actor 为 Principal。async 方法省略 &self，仅为排版简洁。

```rust
// ReadingStore
async fn create(actor: Principal, personal_space: Uuid, command: CreateReading)
    -> Result<ReadingSaved, ContentError>;
async fn edit(actor: Principal, overlay_id: Uuid, command: EditReading)
    -> Result<ReadingSaved, ContentError>;
async fn state(actor: Principal, overlay_id: Uuid)
    -> Result<Option<ReadingState>, ContentError>;
async fn read(actor: Principal, reference: ReadingRef, mode: ReadingMode)
    -> Result<Option<ReadingProjection>, ContentError>;
// PlacementMigrationStore
async fn propose(actor: Principal, overlay_id: Uuid, command: ProposeMigration)
    -> Result<MigrationProjection, ContentError>;
async fn decide(actor: Principal, overlay_id: Uuid, command: DecideMigration)
    -> Result<MigrationDecided, ContentError>;
async fn read(actor: Principal, proposal_id: Uuid)
    -> Result<Option<MigrationProjection>, ContentError>;
```

### 2.1 可公开状态与投影

- ReadingMode：Original / Fused / Personal；Original 也需个人层所有者授权，因为入口是私人视图。
- ReadingState：overlay:OverlayRef、view:ReadingRef、editable:Option<EditableReading>。所有者和个人空间可读即可返回头，所需原文与全部个人引用均可读才有 editable；读权限不承诺具备写权限。
- EditableReading：base:CompositionRef、title:String、groups:Vec<EditableGroup>。EditableGroup 含 group_id、location:GroupLocation、placements:Vec<Placement>；GroupLocation 为 Placed{anchor} 或 Unplaced{origin_anchor}。这是完整结构编辑恢复入口，不直接复用过滤后的阅读列表构造写命令。
- ReadingProjection：overlay:OverlayRef、view:ReadingRef、source:SourceProjection、items:Vec<ReadingItem>、unplaced:Vec<PersonalItem>。不含全层摘要或未经授权的总数。
- SourceProjection：Available{base:CompositionRef} / Unavailable。Unavailable 不附 ID、隐藏原因或结构信息。
- ReadingItem：SectionStart{path:Vec<Uuid>,title:String} / SectionEnd{path:Vec<Uuid>} / Original{path:Vec<Uuid>,revision:Revision} / Personal{item:PersonalItem}。
- PersonalItem：placement_id、revision:Revision、location:Option<GapAnchor>。不返回内部 position，items 顺序就是可见顺序。原文不可用或显式待放置时 location=null。
- ReadingConflict 新错误仅含已经授权的 current_overlay_revision、current_reading_view_revision；正文基础头冲突继续用已有 Conflict。不得在授权失败时返回这些信息。

成功写入返回精简 ReadingSaved；读取内容另经投影。重放写收据前需要原结果全部依赖仍可读，不能从原收据绕过投影。仅需要找回个人正文时使用 read。

### 2.2 迁移合同

```rust
pub struct ProposeMigration {
    pub request_id: Uuid, pub expected_overlay_revision: Uuid,
    pub expected_reading_view_revision: Uuid,
    pub target: CompositionRef, pub reason: String,
}
pub enum GroupDecision {
    Exact { group_id: Uuid },
    AcceptCandidate { group_id: Uuid },
    Manual { group_id: Uuid, anchor: GapAnchor },
    KeepUnplaced { group_id: Uuid },
}
pub struct MergeOrder {
    pub source_group_ids: Vec<Uuid>, pub placement_ids: Vec<Uuid>,
}
pub enum MigrationAction {
    Reject,
    Adopt { groups: Vec<GroupDecision>, merges: Vec<MergeOrder> },
}
pub struct DecideMigration {
    pub request_id: Uuid, pub proposal_id: Uuid,
    pub expected_overlay_revision: Uuid,
    pub expected_reading_view_revision: Uuid,
    pub action: MigrationAction, pub reason: String,
}
pub struct MigrationDecided {
    pub proposal_id: Uuid, pub decision_id: Uuid,
    pub adopted: Option<ReadingSaved>,
}
```

MigrationProjection 含 proposal_id、overlay:OverlayRef（提案基线）、target:Option<CompositionRef>、groups:Vec<MigrationGroupProjection>、decisions:Vec<MigrationDecisionSummary>。目标不可读时 target=null。每个组仅在含有可读个人 placement 时出现，含 group_id、可读 placement_ids、old_anchor:Option<GapAnchor>、suggested_anchor:Option<GapAnchor>、classification:Option<MigrationClass>；classification 的 Exact/Candidate/Unresolved 及原因仅在两侧原文闭包均可读时返回。

MigrationDecisionSummary 只含 decision_id、Adopted/Rejected、作者、时间；完整采用映射是内部权威记录，原始决定 JSON 不直出。每个提案至多一次最终决定；拒绝后重试同请求返回原拒绝，换新请求再决定返回 Invalid(proposal_decided)。要重新考虑则生成新提案。

原先已经待放置的组在新提案中仍为 Unresolved，只有 Manual 或 KeepUnplaced 可处置。正文未丢失，旧锚点仍记录其原始基线，不能假装来自当前层基线。

### 2.3 身份、限额和摘要

标题最多 300 Unicode 标量，理由 1–1,000，无 U+0000；继承 TextDraft 校验。每个命令只执行一个 ReadingEdit，InsertNew 或 ReviseSelected 可批量处理 1–32 个正文；一个块每批最多修订一次，selected_placements 非空且不重复。路径最多 15 个子组合 occurrence，与根算第 1 层一致。

AdoptExisting 将已保存的同一稳定块修订应用于显式选中的出现处，不创建正文修订；所选 placement 必须都属于该稳定块。需要个人层写权限和被采用块的读权限，不能以采用动作修改块头。新建层要求 root kind=document；嵌套父组合仍允许 section。

InsertTarget::NewGroup 不接受客户端 group_id；同一锚点/affinity 已有组时拒绝，要求用 ExistingGroup。新 placement 由服务端生成。Move 的目标邻居相对于移除该 placement 后的列表验证，不能指向自身；原位置完全相同视为 no_change。PlaceUnplaced 不合并时 merge_into=None 且 merged_order 为空；合并时必须列出两个组全部 placement，既不重复也不丢失，保留原组内部相对次序。

正文直接构造和 JSON 均验证。请求摘要域 `personal-reading-request-v1`，包括 operation、actor、target、完整命令；数组默认有序，明确为集合的 selected_placements、changes 按 placement_id/block_id 规范化。迁移 groups 按 group_id 规范化，MergeOrder 内 placement_ids 顺序不得排序。同组相对次序影响摘要。

层摘要域 `overlay-v1`：固定基线、title、按稳定 group_id 排序的组（含 location）、每组有序 placement 引用。读取时不公开这个全结构摘要。view 固定引用即可，不另存可以独立编辑的正文。

## 3. 存储约束和事务约定

新建 `0003_personal_reading.sql`。扩展 request_key 的 operation CHECK，保留 content_v1/composition_save/release_publish，增加 reading_create/reading_edit/migration_propose/migration_decide。只更改 CHECK，不改旧登记和旧摘要。

| 表 | 必需键与约束 |
|---|---|
| overlay | id PK；space_id、owner_id、root_composition_id、head_revision_id NOT NULL；UNIQUE(space_id,id)、UNIQUE(id,root_composition_id) |
| overlay_revision | id PK；overlay_id、space_id、root_composition_id、base_revision_id、parent_revision_id、title、content_sha256、author/reason/time；同层父引用和同根 base FK |
| overlay_group | (overlay_revision_id,group_id) PK；placed/unplaced；锚点原始基线、path UUID[]、left/right、affinity；组归属同层；placed 基线必须等于层基线 |
| overlay_placement | (overlay_revision_id,placement_id) PK；group_id、position、block_space_id/block_id/block_revision_id；同组 FK、块三元 FK；UNIQUE(overlay_revision_id,group_id,position) |
| reading_view | id PK；overlay_id UNIQUE NOT NULL；head_revision_id NOT NULL |
| reading_view_revision | id PK；view_id、overlay_id、overlay_revision_id、parent_revision_id、author/time；精确层 FK、同 view 父 FK |
| placement_migration | id PK；overlay_id、old_overlay_revision_id、target_root/target_revision、author/reason/time；固定旧层与同根目标 |
| placement_migration_group | (proposal_id,group_id) PK；旧锚点、候选锚点、classification/reason_code；内部明细，不输出原始行 |
| placement_migration_decision | id PK；proposal_id UNIQUE；adopted bool、result_overlay_revision_id/result_view_revision_id；拒绝结果两字段 null，采用两字段非 null |
| placement_migration_mapping | decision_id、source_group_id、result_group_id、处置类型、有序 placement 映射；记录合并后的新组 ID |
| reading_receipt / migration_receipt | (actor_id,request_id) PK/FK 到 request_key；前者固定层/视图及 changed_blocks 子表，后者互斥固定 proposal 或 decision |

组稳定身份增加 `overlay_group_identity(overlay_id,group_id)`；placement 稳定身份增加 `overlay_placement_identity(overlay_id,placement_id,block_id)`，稳定目标不可替换。修订明细复合 FK 绑定这些身份；从基础修订移除的 ID 不允许客户端复活，由保存逻辑验证。迁移合并创建新 group_id，原 placement_id 保留，并记录映射。

路径相邻性需要应用层固定闭包检查；SQL 的 UUID[] 长度、位置非负和对象归属约束不替代它。引用原文基线的表另存 source_space_id，和 root/base 组成对 composition_revision 的三元 FK；它不必等于个人层 space_id。为 NULL 邻居与空 path 使用 `UNIQUE NULLS NOT DISTINCT` 的 placed 间隙键，包含 layer revision、root/base、path、left/right、affinity；unplaced 不受该间隙唯一约束，可使用带条件的唯一索引。

循环的身份头 FK 用 DEFERRABLE INITIALLY DEFERRED；view 头对应的层修订必须属于该 view 的 overlay。运行账号仅 SELECT/INSERT，不可变表无 UPDATE/DELETE/TRUNCATE；身份表只授指定 head 列 UPDATE，授权锁函数沿用。

事务锁序为请求 → UUID 升序 grants → overlay → reading_view → UUID 升序待修订 blocks。READ COMMITTED 写先发现可读固定依赖，锁 grants，再重新检查；不能先遍历隐藏闭包。读用 request::begin_read 的一致快照。纯迁移提案不推进头，但也校验预期头，提案持久化和收据一起提交。

内部正文原语不处理请求收据、不提交事务、不再拿其它空间 grant；调用方已持全部权限锁及旧块头锁。接口：

```rust
pub(crate) struct PendingBlock<'a> {
    pub space_id: Uuid, pub block_id: Uuid, pub revision_id: Uuid,
    pub parent: Option<Uuid>, pub actor: Principal,
    pub draft: &'a TextDraft, pub reason: &'a str,
}
pub(crate) async fn insert_block_revision(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, pending: PendingBlock<'_>,
) -> Result<Revision, ContentError>;
```

该原语只插入 block_revision；新建 block 身份及推进 head 仍由调用方事务完成。P0-A 提取现有 insert_revision 后原请求格式、SQL 字段与错误语义保持一致。

## Task 1：严格契约与纯位置运算

**Files:** 新 core 三模块、reading_contracts.rs、三个 contracts 样例；修改 core/lib.rs、error.rs；新增 db/overlay/anchor.rs 的纯逻辑。

**Interfaces:** 实现第 2 节所有 public 类型及 validate；`validate_gap(snapshot:&CompositionSnapshot,anchor:&GapAnchor)->Result<(),ContentError>` 定义在 anchor.rs；`insertion_index(ids:&[Uuid],order:&PlacementOrder)->Result<usize,ContentError>` 返回合法间隙索引。

- [x] 写 JSON/直接构造边界和顺序 RED。先建立可编译签名，以下测试覆盖两侧都 null 不能插入非空组：

```rust
#[test]
fn null_neighbors_require_an_empty_group() {
    let ids = vec![uuid::Uuid::new_v4()];
    let order = learning_core::PlacementOrder {
        left_placement_id: None, right_placement_id: None,
    };
    assert!(super::insertion_index(&ids, &order).is_err());
}
```

- [x] 运行 core reading_contracts 与 db anchor 单元测试；记录可编译后的行为失败。
- [x] 实现位置检查：根引用必须相等，按 path 在固定节点表中逐级选择 Composition target；左右必须在最后父组合 nodes 相邻，null 必须对应真实边界。插入索引为 left 的下一位或 0，右侧必须正好位于该索引；尾部则为 len。
- [x] 增加空根/空章节、重复子章节两路径、错误对象修订、边界逆序、15/16 路径、512/513 组、2,048/2,049 placements、32/33 正文、未知字段、NUL、重复选择的断言。组/placement 总预算需在结构变换后再验，不能仅依输入数量。
- [x] 对 canonical JSON 使用独立 hashlib 计算一份 golden，正文沿用旧 golden；确认选择集合换序摘要相同、组内顺序变化摘要不同。运行全部 core 测试并提交契约及单元测试。

## Task 2：不可变 schema、正文原语与升级兼容

**Files:** 0003、block_write.rs、write.rs、lib.rs、overlay_schema.rs、b1_upgrade.rs、support/reading.rs。

**Interfaces:** 提供第 3 节 PendingBlock/insert_block_revision。B1 升级使用新增 `TEST_B1_UPGRADE_ADMIN_DATABASE_URL`、`TEST_B1_UPGRADE_DATABASE_URL`、`TEST_B1_MIGRATIONS_DIR`，指向独立空库与仅含原 0001/0002 的目录。

- [x] 写 schema RED；验证跨层 group/placement、跨根 base、跨 view 头、不同块/修订配对均拒绝。角色断言使用实际 runtime 连接：

```rust
let allowed: bool = sqlx::query_scalar(
    "SELECT has_table_privilege(current_user,'public.overlay_revision','UPDATE')"
).fetch_one(&rig.runtime_pool).await.unwrap();
assert!(!allowed);
```

- [x] 隔离库运行 `cargo test --locked -p learning-db --test overlay_schema -- --test-threads=1`，记录缺表 RED；按第 3 节建表、FK、CHECK、索引和最小列权限。
- [x] 提取现有正文 insert_revision 为第 3 节原语。P0-A 原测试不改预期；运行原文本、schema、权限、并发与原子性测试，比较原 digest 字节。
- [x] 写真实 B1 升级夹具：SQLx Migrator 正常应用 0001/0002，使用 B1 store 创建块、组合、发布和请求收据，然后应用完整 MIGRATOR，读取和重放逐项相同。不能提前由 TestRig 迁移升级库，不能手填迁移登记。

```rust
let old = sqlx::migrate::Migrator::new(std::path::Path::new(
    &std::env::var("TEST_B1_MIGRATIONS_DIR").expect("B1 migrations directory")
)).await.unwrap();
old.run(&upgrade_admin_pool).await.unwrap();
// 此处经 B1 store 创建历史夹具，再执行完整迁移。
learning_db::MIGRATOR.run(&upgrade_admin_pool).await.unwrap();
```

- [x] 新 schema、B1 升级及原 68 项全部通过后提交。保留 0001/0002 原校验和，原 P0-A 升级测试仍运行至最新 schema。

## Task 3：原子个人编辑和固定视图

**Files:** overlay/model.rs、edit.rs、persist.rs；reading/mod.rs、write.rs、read.rs；overlay.rs、reading.rs 测试及 reading 夹具；lib.rs。

**Interfaces:** 实现 ReadingStore create/edit/state/read；本任务 read 可只支持完整授权，Task 4 扩展部分可见投影。内部 `apply_edit` 接受已解析的当前层、命令和新正文 BlockRef 映射，返回新层结构，不自行调用数据库。

- [x] 夹具经真实 ContentStore/CompositionStore 创建 A、B 与重复章节，ReadingStore::create 建空层。写编辑 RED：K/N/I 中间插入 H、同块两次只选一处修订、移动不改正文、删除一次不删块，历史 view 读取仍旧值。每步保存独立 ReadingRef，再按 Fused 输出实际顺序断言。
- [x] 用实际 store stub 返回 Storage 运行 `--test overlay --test reading`，确认业务失败；然后按以下顺序实施：

```text
validate → begin(request lock) → owner/grant discovery → sorted grant locks
→ exact receipt authorization/replay → lock overlay/view → compare both heads
→ lock changed block heads in UUID order → compare text bases
→ insert new block revisions → validate and apply explicit placement edits
→ validate resulting budgets and no_change
→ insert overlay revision/groups/placements → insert reading revision
→ advance overlay/view heads → register request/receipt → commit
```

- [x] 根据第 2 节实现新组、新出现、批量正文、显式 selected_placements；相同 TextDraft 且没有结构变化时 no_change，避免仅以新 revision UUID 使空操作变成有效保存。块稿相同但当前 placement 指向更旧版时，用 AdoptExisting 采用已经保存的版本；ReviseSelected 的批次若任一 change 与块当前稿相同则整批返回 Invalid(no_change)，不产生部分修订。AdoptExisting 全部所选位置已是目标修订时也返回 no_change。
- [x] 在支持模块增加受 actor 限定的 BEFORE INSERT reading_receipt 故障，正文已插入后失败仍全部回滚；统计 block、block_revision、overlay_revision、group identity/detail、placement identity/detail、view_revision、request_key 和 receipt。移除故障后用原键重试成功。

```sql
RAISE EXCEPTION 'injected_reading_receipt_failure';
```

  触发器仅在 NEW.actor_id 等于该测试 actor 时抛错，管理员建/删夹具；运行操作始终用 runtime。
- [x] 补 state 恢复：新调用方只知道 overlay_id 也能获得两个当前修订；冲突后刷新再提交成功；已成功命令在后续编辑后重放原 ReadingSaved。验证 changed_blocks 收据固定，不读当前块头。
- [x] 运行新增测试、原 68 项及 Clippy 后提交；正文原语、保存和收据不留分离提交事务。

## Task 4：安全连续阅读及撤权恢复

**Files:** reading/read.rs、projection.rs、overlay/model.rs；reading_authorization.rs、reading.rs。

**Interfaces:** ReadingState/ReadingProjection 按第 2.1 节实现；内部层记录只在 db crate 内可见。三个 mode 共用当前权限解析，Original 不返回个人项，Personal 不返回原文正文，Fused 插入组内容。

- [x] 写 RED：原文根或某个子依赖撤权，source=Unavailable、个人正文仍返回且所有 location=null；另一个 actor 即使有原文和个人空间 grant 也不能读该层。再撤掉一个个人块 grant，输出不能留下该 ID、序号空洞或隐藏数量。

```rust
let value = serde_json::to_value(projection).unwrap().to_string();
assert!(!value.contains(&hidden_block_id.to_string()));
assert!(!value.contains(&hidden_occurrence_id.to_string()));
assert!(!value.contains("secret source text"));
```

  hidden_block_id/hidden_occurrence_id 来自夹具实际被撤权对象，不能用根本未引用的随机 UUID 代替。
- [x] 运行 `--test reading_authorization` 建立行为 RED；实现 READ ONLY REPEATABLE READ 单一快照。先检 owner/个人空间，原文用固定闭包查询；必要对象不可读转 source_unavailable，预算 Invalid 和数据库 Storage 继续传播，不能吞成权限状态。
- [x] 个人块读取 JOIN 当前 grant 后再计算去重正文预算和输出；过滤后生成顺序。原文不可用时所有可见 personal 项按内部既定顺序进入 unplaced；此时不返回 editable，恢复 grant 后位置自动恢复且数据库修订计数不变。
- [x] 检查摘要旁路：完整层 content_sha256、旧锚点、原始组位置不可出现在部分可见 DTO。已有 Revision.content_sha256 仅对应当前可读的个人正文，可以保留。
- [x] 覆盖可见个人正文恰好 8 MiB/+1、重复引用不重复计正文、原文预算与个人预算分别生效。过滤顺序不得让隐藏正文触发可见输出字节计数；超限不返回部分数据。
- [x] 三种模式、历史视图、state、撤权收据重放均通过后提交；原文不可用时编辑原层拒绝，但公开 ContentStore 对自己的可写块仍可独立修订。

## Task 5：持久化迁移提案、决定与组碰撞

**Files:** placement_migration 四模块；overlay/persist.rs、reading/write.rs 的事务内复用；placement_migration.rs 集成测试。

**Interfaces:** 第 2.2 节接口；内部纯函数 `classify_group(old:&CompositionSnapshot,new:&CompositionSnapshot,anchor:&GapAnchor)->Result<MigrationSuggestion,ContentError>`；MigrationSuggestion 仅内部使用，含 class、candidate:Option<GapAnchor>、reason_code。

- [x] 写 RED：相邻保持 Exact；中间新原文/单侧存在 Candidate；逆序、两侧消失、路径失效 Unresolved；原空章节变非空不 Exact；重复子章节只匹配原 occurrence 路径。用旧/新固定快照和显式期望 candidate 锚点，不调用被测函数生成期望。
- [x] 实现分类：先验证同根和路径稳定父身份，再按直接邻居判断。affinity 首选侧存在时紧靠该侧提出有效新 gap；仅另一侧存活则候选理由为 preferred_side_missing；反序先于 affinity 判断，直接 unresolved。旧待放置组不自动重新匹配。
- [x] propose 锁权限并校验预期头，保存所有组结果和收据，当前头不变；decide 对每个旧组恰好一次处理，Exact/AcceptCandidate 必须对应提案分类，Manual 必须是目标基线有效锚点。不同根/同基线目标拒绝；所有根与依赖当前可读。
- [x] 为组碰撞写真实失败断言：两个组映射同 gap/affinity，未提供 MergeOrder 返回 Invalid；提供全部无重复 placement 且保留各原组相对顺序后成功；生成一个新组并保存两个旧组的映射。映射到不同 affinity 无须合并。
- [x] Reject 只追加唯一决定和收据；Adopt 在同一事务创建层/视图/映射/决定/收据，原 view 可读。故障注入 decision receipt 末尾，验证上述表及两头全回滚；同请求重试原结果，新请求不能再次决定同一提案。
- [x] 增加 Manual/KeepUnplaced、旧待放置跨多次升级仍保留真正 origin_anchor、人工放置合并、过时提案拒绝。旧层已变化的新请求拒绝，已成功旧请求重放先走收据。
- [x] 迁移读输出分别授权两侧原文；任一侧不可读时 classification=null，候选/旧锚点按各自依赖过滤，整组无可读 placement 时省略。读取决定摘要不能暴露结果层隐含原文引用；新增测试后提交。

## Task 6：并发、升级环境与交付

**Files:** reading_concurrency.rs、reading_authorization.rs、b1_upgrade.rs、deploy/initdb.sh、run-tests.sh；README 和 B2 三份记录；更新外部批准路线和总单。

**Interfaces:** 沿用 request::begin/check/register、authorization::lock_grants；新增 B1 升级库不替代现有升级库。三库为普通全迁移测试库、P0-A 升级库、B1 升级库。

- [x] 并发测试用两个独立 backend PID、Tokio Barrier 同时起跑，断言同层同/不同 gap 一成一 ReadingConflict；失败批次无新正文。相同 request 两次只有一套层/视图；跨 P0-A/B1/B2 重用 request 仅一个操作成功，另一 IdempotencyConflict。

```rust
let (left, right) = tokio::join!(left_task, right_task);
let results = [left.unwrap(), right.unwrap()];
assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
assert_eq!(results.iter().filter(|r| matches!(r,
    Err(learning_core::ContentError::ReadingConflict { .. })
)).count(), 1);
```

  left_task/right_task 是独立池 max_connections(1) 上的真实 ReadingStore::edit；测试同时核对持久化行数和最终头，不以 join 返回顺序判断胜者。
- [x] 用 pg_blocking_pids 验证撤权先持锁则编辑拒绝、编辑先持 grant 锁则撤权等待；乱序多个正文块按 UUID 加锁；P0-A 与 B2 同块竞争一成一正文 Conflict，不产生死锁。避免用固定 sleep 当作锁证据。
- [x] 增加迁移和普通编辑同层竞争、同提案两个决定、合并与另一个移动竞争；只允许一个实际推进头，所有失败路径完整回滚。拒绝决定也要求预期头，不能对过时提案静默落一条新的拒绝。
- [x] Compose 初始化增加 `learning_b1_upgrade_test`，普通角色权限与既有两库相同。run-tests 构造两个新增 DSN；mktemp 目录只复制 0001/0002 为 TEST_B1_MIGRATIONS_DIR；TEST_P0A_MIGRATIONS_DIR 仍只含 0001。凭据值不得进入日志。
- [x] 本地执行格式、Clippy、core；服务器新批次三库运行全 workspace，固定源文件清单与哈希。升级库拒绝已有迁移登记；每轮全量重跑新建三库/新 Compose 项目，保留原证据卷。

```sh
cargo fmt --all -- --check
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo test --offline --locked --workspace -- --test-threads=1
docker compose -p learning-system-p0b2-final -f deploy/compose.test.yaml --profile test config --quiet
docker compose -p learning-system-p0b2-final -f deploy/compose.test.yaml --profile test build test
docker compose -p learning-system-p0b2-final -f deploy/compose.test.yaml --profile test up --abort-on-container-exit --exit-code-from test
docker compose -p learning-system-p0b2-final -f deploy/compose.test.yaml stop
```

- [x] 使用「本地 Linux 服务器」既有协作方式验证，保持专用网络、资源限制和无发布端口；环境失败与业务 RED 分别记录。对固定代码做一次只读独立审查，问题按根因修复并执行相应回归及最终全量。
- [x] 核验源码前后哈希、日志摘要、命令退出码、全部旧测试和新增真实数量。证据完整才标记 P0_B2_VERIFIED；更新边界、执行取舍、路线及 README，停止专用容器，保留数据卷。提交代码和交付文档，不声明 UI 或 P0-B 全部完成。

## 4. 设计覆盖与执行交接

| 设计章节 | 落实任务 |
|---|---|
| 1–3：范围、方案、权威对象 | Task 1、2 |
| 4：精确位置与顺序 | Task 1、3 |
| 5：保存和复用范围 | Task 3、6 |
| 6：升级与迁移 | Task 5、6 |
| 7：权限与可找回 | Task 4、5、6 |
| 8：存储事务 | Task 2、3、5、6 |
| 9：预算与失败 | Task 1、3、4、5 |
| 10：验收 | Task 1–6，原 68 项持续回归 |

本单选择单编辑命令加最多 32 个正文批处理，避免临时对象互相引用和通用脚本式操作；迁移提案终结后重新考虑需新提案。这些是既有设计下的实现约束，不扩展产品阶段。

执行完成：全部六项任务、一次独立审查、修复回归、两条升级路径与全新 Compose 均已验证。实施提交 b213d6e，审查修复 de71766；执行取舍、测试批次及范围限制以链接的验收记录和账本为准。B3/B4 和前端尚未实施。
