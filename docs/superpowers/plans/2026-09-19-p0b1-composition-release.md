# P0-B1 固定组合与原子发布 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans 实施本计划，逐任务运行测试并记录结果；完成后执行一次独立代码审查。步骤采用 `- [x]` 跟踪。

**Goal:** 在不复制正文的前提下，让多份文档固定复用块、保持出现位置身份，并通过一个事务发布选定组合。

**Architecture:** 在现有 learning-core / learning-db 中新增组合与发布模块。不可变修订、出现项和发布根保存在关系表；所有引用明确包含稳定对象及修订 ID。沿用当前授权与幂等规则，增加跨操作请求登记及事务内 outbox 事件。

**Tech Stack:** Rust 1.97.0、Serde、Tokio、SQLx 0.8.6、PostgreSQL 18.6；沿用 Cargo.lock 与已固定的 Linux/amd64 测试镜像，不为本关新增 Web 或图数据库依赖。

**Spec:** [P0-B 总单](2026-09-19-p0b-content-assembly-roadmap.md)、[存储规范第 8–10 节](../../内容分块与存储规范.md)、[批准路线](../../设计批准与实施路线.md)、[P0-A 内容边界](../../content-boundary.md)。

**Status:** `P0_B1_VERIFIED` / `NOT_PRODUCTION`，2026-09-19。用户在计划交付后明确要求“继续”，本关已执行并验收；代码基线 `e799d6f`，核心提交 `b890798`、`6563a3d`。宿主与空库 Compose 各 68/68，独立审查问题已修复。计划片段保留为设计依据；实际命令与通过/失败结果见 [B1 验证记录](../../p0b1-verification.md)。

## Global Constraints

- “已发布组合只能固定版本；嵌套组合也固定，整个依赖闭包可复现。”
- “同一块可以在一个组合出现多次，每处有独立 occurrence ID；重排不改变其身份。”
- “同一幂等键与同一请求内容重试返回原结果；同键不同内容拒绝。”
- “发布前固定本变更集依赖；提交时再校验编辑头和预期组合发布指针”。
- “数据库发布与搜索构建不是一个跨系统事务。”
- P0-A 的 `TextDraft`、content-v1 摘要与原有 29 项测试保持兼容。
- 不改 `0001_content_core.sql`；新增 `0002_composition_release.sql`。
- 组合引用可跨空间，但每个所需对象分别检查授权；发布不创建分享权限。
- 本关只接受当前已支持的 text 块；其它 kind、自由引用载荷、个人层和语义关系不提前接入。
- 运行数据库角色继续不能更新/删除修订、收据、出现项或发布历史；管理员只运行迁移与夹具。

## Review Focus

1. 同一子章节复用两次不能被全局 visited 集误判成环；Task 3 加菱形与双路径断言。
2. 合法 UUID 配上其它对象的 revision_id 必须拒绝；Task 2/3 验证组合键与应用授权。
3. 已发布组合引用跨空间块，撤权后重放旧请求仍须拒绝；Task 5 验证原始结构、正文及发布清单。
4. 多文档发布最后一条 outbox 写入失败，不能留下部分活动指针；Task 4 注入真实数据库失败。
5. 已有 P0-A 请求键不能在组合操作中再次使用；Task 2/5 验证升级后的全局幂等及并发跨操作冲突。

---

## 1. 范围与运行结果

本关完成：新建/修订组合、固定嵌套、稳定出现项、完整授权读取、原子发布/回退、历史发布读取、outbox 原子落库。工作头与发布头分离：编辑组合不会立即改变已发布文档。

本关没有自动迁移笔记、动态 latest 引用、关系查询、资产、HTTP、UI 或 outbox 消费者。发布事件存在不表示搜索或预览已经生成；后续 P0-C 才建立消费状态。

本关的“回退”是新增 Release 将发布指针指向指定历史组合修订，工作头保持原值，历史修订不删除。一次 Release 中所有根组合属于发布空间；其只读依赖允许来自其它可读空间。

## 2. 文件及职责

以下路径相对 `仓库根目录`。

| 文件 | 行为 |
|---|---|
| `crates/learning-core/src/composition.rs` | 精确引用、出现项、保存命令和组合修订 |
| `crates/learning-core/src/release.rs` | 发布命令、根、发布记录与 outbox 事件类型 |
| `crates/learning-core/src/lib.rs`、`src/error.rs` | 导出新类型，增加发布指针冲突错误 |
| `crates/learning-db/src/request.rs` | 统一请求锁、登记和摘要比较，兼容旧内容请求 |
| `crates/learning-db/src/authorization.rs` | 多空间授权锁与读取判断 |
| `crates/learning-db/src/composition/{mod,write,read,closure}.rs` | 组合保存、完整闭包读取、路径校验 |
| `crates/learning-db/src/release/{mod,write,read}.rs` | 多根发布、历史读取 |
| `crates/learning-db/src/lib.rs`、`src/write.rs` | 暴露新 store，并接入统一请求登记 |
| `migrations/0002_composition_release.sql` | 新表、约束、索引、旧收据登记回填和 runtime ACL |
| `crates/learning-core/tests/assembly_contracts.rs` | 纯契约测试 |
| `crates/learning-db/tests/{assembly_schema,composition,release,assembly_authorization,assembly_concurrency}.rs` | PostgreSQL 行为测试 |
| `crates/learning-db/tests/support/assembly.rs`、`support/mod.rs` | 新夹具；保留原 TestRig 接口 |
| `crates/learning-db/tests/migration_upgrade.rs`、`tests/publication_state.rs` | 独立库升级，以及发布状态获取、冲突刷新和根级授权 |
| `contracts/composition-save.example.json`、`contracts/release.example.json` | 严格命令示例 |
| `docs/p0b1-boundary.md`、`docs/p0b1-verification.md`、`README.md` | 正式边界、证据与运行说明 |

新增模块留在两个现有 crate，避免为了本关拆出没有独立职责的 crate。只抽取本关实际共用的请求和授权代码，不顺手重构其余内容模块。

## 3. 统一接口合同

以下为必须实现的公开类型轮廓。实现时为命令和载荷增加严格 Serde 校验，所有公共入口同时验证直接由 Rust 构造的值。

```rust
use learning_core::{ContentError, Principal, Revision};
use chrono::{DateTime, Utc};
use uuid::Uuid;

pub struct BlockRef { pub block_id: Uuid, pub revision_id: Uuid }
pub struct CompositionRef { pub composition_id: Uuid, pub revision_id: Uuid }
pub enum NodeTarget { Block(BlockRef), Composition(CompositionRef) }
pub enum CompositionKind { Document, Section }
pub struct NodeDraft {
    pub occurrence_id: Option<Uuid>,
    pub target: NodeTarget,
}
pub struct Occurrence {
    pub occurrence_id: Uuid,
    pub target: NodeTarget,
}
pub struct SaveComposition {
    pub request_id: Uuid,
    pub composition_id: Option<Uuid>,
    pub base_revision_id: Option<Uuid>,
    pub kind: CompositionKind,
    pub title: String,
    pub nodes: Vec<NodeDraft>,
    pub reason: String,
}
pub struct CompositionRevision {
    pub reference: CompositionRef,
    pub parent_revision_id: Option<Uuid>,
    pub kind: CompositionKind,
    pub title: String,
    pub nodes: Vec<Occurrence>,
    pub content_sha256: String,
    pub author_id: Uuid,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}
pub struct CompositionSnapshot {
    pub root: CompositionRef,
    pub compositions: Vec<CompositionRevision>,
    pub blocks: Vec<Revision>,
}
pub struct PublishRoot {
    pub composition_id: Uuid,
    pub revision_id: Uuid,
    pub expected_head_revision_id: Uuid,
    pub expected_publication_token: String,
}
pub struct PublishCommand {
    pub request_id: Uuid,
    pub roots: Vec<PublishRoot>,
    pub reason: String,
}
pub struct Release {
    pub release_id: Uuid,
    pub space_id: Uuid,
    pub roots: Vec<CompositionRef>,
    pub author_id: Uuid,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}
```

方法约定（`CompositionStore::new(PgPool)` 与 `ReleaseStore::new(PgPool)` 同现有 ContentStore 模式）：

```rust
// CompositionStore
async fn save(&self, actor: Principal, space: Uuid, command: SaveComposition)
    -> Result<CompositionRevision, ContentError>;
async fn read(&self, actor: Principal, root: CompositionRef)
    -> Result<Option<CompositionSnapshot>, ContentError>;
// ReleaseStore
async fn publish(&self, actor: Principal, space: Uuid, command: PublishCommand)
    -> Result<Release, ContentError>;
async fn read(&self, actor: Principal, release_id: Uuid)
    -> Result<Option<Release>, ContentError>;
async fn active(&self, actor: Principal, composition_id: Uuid)
    -> Result<Option<CompositionRef>, ContentError>;
```

**审查修正：** 原计划的 expected_release_id 缺少 fresh caller 获取入口；本次改为组合专属 publication_token，并增加 `ReleaseStore.state(actor, composition_id) -> Result<Option<PublicationState>, ContentError>`。PublicationState 含 composition_id、head_revision_id、published: Option<CompositionRef>、publication_token: String。该接口分别检查工作头与发布根的完整闭包，不检查多根 release 的兄弟文档；发布在已有根行锁下比较令牌。token 是 SHA-256(domain=publication-basis-v1, composition_id, last_release_id) 的小写十六进制值，命令严格要求64字符；不返回内部 release ID。新增 publication_state 集成测试覆盖新调用方、冲突刷新、兄弟根撤权和跨根令牌。

`active` 只在该活动根的整个内容闭包可读时返回引用；`read(release)` 校验其全部根的闭包。`save` 的结果只返回本次组合修订，但返回前也必须拥有完整依赖授权。

### 3.1 JSON、校验与身份

引用枚举用 `#[serde(tag="type", rename_all="snake_case")]`；block 变体字段为 block_id/revision_id，composition 变体为 composition_id/revision_id；所有层拒绝未知字段。

- 新建：composition_id/base_revision_id 均为 null；修订：二者均非 null，必须匹配当前头及目标空间。
- composition kind 创建后固定；title 最多 300 Unicode 标量，reason 1–1,000 标量，拒绝 U+0000，不裁剪空白。
- 每层最多 512 nodes；空组合合法。已存在 occurrence_id 必须来自本组合的基础修订，且指向同一稳定目标及同一 target kind；允许更新目标修订。删除后的旧 ID 不通过客户端复活；恢复旧结构可发布旧修订或创建新 occurrence。
- 新 occurrence_id 为 null，由服务分配。复制一处必须传 null；同一命令两次使用同一既有 occurrence_id 为 Invalid。
- 发布 roots 为 1–16，composition_id 不重复；发布摘要前按 composition_id 排序，表示选择集合；nodes 有顺序，不能排序。
- 命令结构或超限返回 Invalid；隐藏与不存在目标都返回 NotFound；基础头冲突继续使用 Conflict，仅在已获完整授权后返回当前修订 ID。
- 发布指针冲突新增 `ContentError::PublicationConflict { composition_id: Uuid }`，只带已授权组合身份，不暴露可能包含其它空间根的 release ID。

### 3.2 固定闭包与结构环

闭包只走精确 revision，不追随 head。每条路径最多 16 层组合（根为第 1 层），累计最多 4,096 个展开 occurrence；不同修订对象最多 2,048 个；读取正文总序列化 UTF-8 字节不超过 8 MiB。超限返回固定 Invalid code，不返回未经授权的实际计数。

检查环使用**当前路径上的 composition_id**，而非全局 visited：A@2→B@1→A@1 仍拒绝；同一 B@1 出现在两个兄弟分支合法。块重复出现不算环。全局缓存可以减少数据库读取，但展开计数与路径检查必须对每条出现路径执行。

并发 A@2→B@1、B@2→A@1（A@1/B@1 都为空）可以同时成功，因为两个固定闭包都无环。随后 A@3→B@2 必须拒绝。不要将全库所有历史边合并检查，也不要按当前 head 推导本不存在的环。引用只能指向查验时已提交的固定组合修订（READ COMMITTED 不保证早于整个事务起点）；本关不接收带临时 ID 的相互引用新组合批次。

### 3.3 摘要、幂等与权限

组合内容摘要域 `composition-v1` 包含 kind/title/有序 occurrence 及精确 targets；不含作者、时间、原因和自身修订 ID。命令摘要域 `assembly-request-v1` 包含操作、目标空间、整个命令（含基础版本和 reason）。复用现有 canonical_json/hex_digest，保留 P0-A 内容请求摘要原算法。

请求唯一范围仍为 `(actor_id,request_id)`，跨块保存、组合保存与发布操作共享。继续使用原 `learning/content/request/{actor}:{request}` advisory lock 命名空间，避免新旧操作各自上锁导致撞键。

新增 request_key 登记旧/新请求，旧 mutation_receipt 保留具体 block_revision FK；组合和发布各用类型化 receipt。登记、业务写入、receipt 同一事务，失败不留占位请求键。先验证本次操作完整权限，再比摘要和查原结果；精确重放在基础头检查之前。

写事务 READ COMMITTED，依次锁请求 → 全部授权空间（UUID 升序）→ 根组合（UUID 升序）。闭包版本不可变：可先内部发现需要锁的空间，再重验空间及对象存在；内部发现结果不能进错误消息。目标空间需要 can_write，依赖空间仅需要 grant 存在；沿用 lock_space_grant 的共享锁，Some(false) 对只读依赖合法，NULL 不合法。

闭包发现的每一步查询都 JOIN 当前主体的 grant，不能先遍历隐藏子树再校验权限。引用 ID 已知但授权目标查不到时立刻使用统一 NotFound/None，不报告隐藏子树的大小或深度。遍历预算只对可见部分累计；随后按排序锁定已经发现的 grants，并重新确认权限与闭包，处理发现与锁定之间的撤权竞争。

读闭包使用 READ ONLY REPEATABLE READ 事务，同一快照读取结构、正文与当前 grants。权限快照以该次读取开始为界；请求开始前已提交撤权必须生效，已经开始/返回的内容不能被追溯收回。任何必要对象不可读，整个本关读取返回 None；不提供隐藏占位符、隐藏计数或原始 JSON 回退。

## 4. 数据库约束

迁移建立以下表；所有 ID 使用 uuid，时间使用 timestamptz。下列键是强制要求，不依赖应用校验替代。

| 表 | 主键及主要约束 |
|---|---|
| request_key | PK(actor_id,request_id)，request_sha256、operation 非空；operation 为 content_v1/composition_save/release_publish |
| composition | PK(id)，UNIQUE(space_id,id)，kind 创建后不可变，created_at 非空，head 非空；published_revision_id/last_release_id 同空或同非空 |
| composition_revision | PK(id)，UNIQUE(space_id,composition_id,id)，同对象 parent FK、非自身 parent；kind/title/hash/author/reason/time |
| composition_occurrence | PK(composition_revision_id,occurrence_id)，UNIQUE(composition_revision_id,position)，position>=0；owner space/composition 与修订三元 FK |
| composition_receipt | PK(actor_id,request_id)，FK request_key，结果 FK composition_revision(id) |
| release | PK(id)，space/author/reason/time，不可变 |
| release_root | PK(release_id,composition_id)，精确组合三元 FK；额外 UNIQUE(release_id,composition_id,revision_id) |
| release_receipt | PK(actor_id,request_id)，FK request_key，结果 FK release(id) |
| outbox_event | PK(id)，UNIQUE(event_type,aggregate_id)，aggregate_id FK release(id)，event_type=composition_released，payload_version=1，created_at |

出现项采用互斥的两组 FK，不用无约束 target JSON：block_space_id/block_id/block_revision_id 或 child_space_id/child_composition_id/child_revision_id；恰好一组全部非空，另一组全部为空。分别引用 block_revision 和 composition_revision 的三元 UNIQUE；position 保存顺序，target 身份不靠 title 判断。

核心迁移片段：

```sql
CREATE TABLE request_key (
  actor_id uuid NOT NULL REFERENCES app_user(id),
  request_id uuid NOT NULL,
  request_sha256 text NOT NULL CHECK (request_sha256 ~ '^[0-9a-f]{64}$'),
  operation text NOT NULL CHECK (operation IN
    ('content_v1','composition_save','release_publish')),
  PRIMARY KEY(actor_id, request_id)
);
INSERT INTO request_key(actor_id,request_id,request_sha256,operation)
  SELECT actor_id,request_id,request_sha256,'content_v1' FROM mutation_receipt;
ALTER TABLE mutation_receipt ADD CONSTRAINT content_request_registered
  FOREIGN KEY(actor_id,request_id) REFERENCES request_key(actor_id,request_id);
-- 新表建完后补延迟约束，允许一次事务插入身份、首修订和发布根。
ALTER TABLE composition ADD CONSTRAINT composition_head_same_object
  FOREIGN KEY(space_id,id,head_revision_id)
  REFERENCES composition_revision(space_id,composition_id,id)
  DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE composition ADD CONSTRAINT published_root_matches_release
  FOREIGN KEY(last_release_id,id,published_revision_id)
  REFERENCES release_root(release_id,composition_id,revision_id)
  DEFERRABLE INITIALLY DEFERRED;
```

release_root 的 space_id 必须同时关联 release 的 `(space_id,id)` 唯一键和 composition_revision 三元键，防止把其它空间根塞进当前发布。composition 的 published_revision 另加同空间同组合 FK，并加成对 NULL CHECK。receipt 的结果永远不可被 runtime 修改。

索引：出现项两个目标修订索引；composition(space_id,created_at,id)；revision(composition_id,created_at,id)；release_root(composition_id,release_id)。除身份表 head/发布指针列外仅授予 SELECT/INSERT；新表均不授予 runtime DELETE/TRUNCATE，outbox 暂时只追加。权限辅助函数沿用 P0-A，禁止扩大函数所有者或 runtime 的空间授权写权限。

## 5. 执行任务

### Task 1：严格组合和发布契约

**Files:** 新增 core 的 composition.rs/release.rs、assembly_contracts.rs、两个 contracts JSON；修改 core/lib.rs 和 error.rs。

**Interfaces:** 实现第 3 节所有数据类型、`SaveComposition::validate()`、`PublishCommand::validate()`；新增 PublicationConflict；不引入数据库访问。

- [x] 写契约测试，先通过 JSON 反序列化制造输入，再验证直接 Rust 构造的相同错误。基础样例：

```rust
#[test]
fn creation_rejects_an_unpaired_base() {
    let value = serde_json::json!({
        "request_id": uuid::Uuid::new_v4(),
        "composition_id": null,
        "base_revision_id": uuid::Uuid::new_v4(),
        "kind": "document", "title": "Attention", "nodes": [], "reason": "新建"
    });
    assert!(serde_json::from_value::<learning_core::SaveComposition>(value).is_err());
}
```

- [x] 运行 `cargo test --locked -p learning-core --test assembly_contracts`。缺类型先建立最小可编译类型，再确认校验错误的行为 RED；不能把拼写/编译错误当成有效红阶段。
- [x] 使用 Raw 类型 + TryFrom 方式实现严格反序列化和 validate，沿用 TextDraft 的双入口校验模式。核心分支：

```rust
if command.composition_id.is_some() != command.base_revision_id.is_some() {
    return Err(ContentError::Invalid("composition_base_pair".into()));
}
if command.nodes.len() > 512 {
    return Err(ContentError::Invalid("composition_nodes_limit".into()));
}
```

- [x] 加边界断言：空文档；512/513 nodes；300/301 标量标题；1/1,000/1,001 标量 reason；NUL；重复 occurrence；未知嵌套 target 字段；1/16/17 发布根；重复根；roots 置换摘要一致而 nodes 置换摘要不同。
- [x] 运行新测试与 `cargo test --locked -p learning-core`；固定一份组合规范 JSON 和独立 Python hashlib 摘要为 golden，不用被测 digest 函数生成预期值。保留 P0-A golden 不变。
- [x] 提交：`git add crates/learning-core contracts`，`git commit -m "feat: define fixed composition and release contracts"`。

### Task 2：存储、权限与请求键升级

**Files:** 新增 0002 迁移、request.rs、authorization.rs、assembly_schema.rs、migration_upgrade.rs；修改 db/lib.rs、write.rs、support/mod.rs。

**Interfaces:** 以下内部函数全部 async，tx 为 `&mut sqlx::Transaction<'_, sqlx::Postgres>`，actor 为 Principal，request_id 为 Uuid，digest/operation 为 `&str`：`lock_request(tx,actor,request_id)->Result<(),ContentError>`、`check_request(tx,actor,request_id,digest,operation)->Result<bool,ContentError>`、`register_request(tx,actor,request_id,digest,operation)->Result<(),ContentError>`；`lock_grants(tx,actor,required:&[(Uuid,bool)])->Result<(),ContentError>`，bool 表示需写。check_request 的 true 只表示已有相同登记，随后必须读取对应类型 receipt；登记与结果不完整作为 Storage。

- [x] 写数据库失败测试：不属于同一对象的 head/target、空 head、目标两组字段同时存在、重复 position、runtime 修改修订/receipt/outbox 均失败。ACL 断言示例：

```rust
let allowed: bool = sqlx::query_scalar(
    "SELECT has_table_privilege(current_user,'public.composition_revision','UPDATE')"
).fetch_one(&rig.runtime_pool).await.unwrap();
assert!(!allowed);
```

- [x] 在明确的隔离数据库环境运行 `cargo test --locked -p learning-db --test assembly_schema -- --test-threads=1`，确认表/约束缺失造成预期失败；记录这一阶段属于 schema RED。
- [x] 按第 4 节实现 0002。把原 write.rs 请求锁及授权函数移入共用模块；P0-A 在授权后检查 request_key，成功路径在 receipt 前登记键，摘要算法不变。现有内容 create/revise 结果及 counts 断言不得变更来迁就失败。
- [x] 旧数据迁移测试使用**专用空库**和测试夹具 admin 连接：先用只含 0001 的 SQLx Migrator 正常登记迁移版本；插入合法 block/revision/receipt；再运行完整 MIGRATOR。禁止手填或删改 `_sqlx_migrations`。核心加载方式：

```rust
let old = sqlx::migrate::Migrator::new(std::path::Path::new(
    &std::env::var("TEST_P0A_MIGRATIONS_DIR").expect("0001-only fixture directory")
)).await.unwrap();
old.run(&upgrade_admin_pool).await.unwrap();
// 管理员插入 P0-A 夹具后执行正式新增迁移。
learning_db::MIGRATOR.run(&upgrade_admin_pool).await.unwrap();
```

  `upgrade_admin_pool`/`upgrade_runtime_pool` 分别来自 `TEST_UPGRADE_ADMIN_DATABASE_URL` 和 `TEST_UPGRADE_DATABASE_URL`；该测试不调用会提前迁移的 TestRig::from_env。测试夹具用原 command/request_digest 规范独立生成收据，记录旧正文、摘要、头和请求结果，升级后逐一比对且重放返回原结果。空环境变量必须失败，不能忽略测试。
- [x] 验证两个独立空库路径：完整迁移一次运行、0001 数据升级。重跑原 29 项测试及新 schema 测试；迁移校验和必须保持 0001 原值。
- [x] 提交：`git add migrations crates/learning-db`，`git commit -m "feat: persist compositions and register idempotent operations"`。

### Task 3：固定组合保存及闭包读取

**Files:** 新增 db/composition 四文件、tests/composition.rs、support/assembly.rs；修改 db/lib.rs。

**Interfaces:** 实现 CompositionStore 的 save/read；内部 `load_closure` 接受精确根，返回对象集合、展开路径和所需空间；内部结果不实现面向外部的直接序列化。

- [x] 在 support/assembly.rs 增加以下薄夹具，只调用真正公开接口，不模拟数据库结果：

```rust
pub fn doc(block: learning_core::BlockRef) -> learning_core::SaveComposition {
    learning_core::SaveComposition {
        request_id: uuid::Uuid::new_v4(), composition_id: None, base_revision_id: None,
        kind: learning_core::CompositionKind::Document, title: "Attention".into(),
        nodes: vec![learning_core::NodeDraft {
            occurrence_id: None, target: learning_core::NodeTarget::Block(block),
        }], reason: "创建固定文档".into(),
    }
}
```

- [x] 写 save/read 的行为 RED：同一 K@1 进入 A/B；K 更新为 K@2 后，读取 A/B 仍返回 K@1 原文，二者 occurrence 不同。对同一文档重复 K@1，出现项不同但块正文只存一份。关键断言：

```rust
let saved = compositions.save(actor, space, doc(old_ref.clone())).await.unwrap();
let new_block = rig.store.revise(actor, original.block_id,
    support::change(&original, "新版")).await.unwrap();
let snapshot = compositions.read(actor, saved.reference.clone()).await.unwrap().unwrap();
assert_eq!(snapshot.blocks[0].revision_id, old_ref.revision_id);
assert_ne!(snapshot.blocks[0].revision_id, new_block.revision_id);
```

  测试函数内用 TestRig 创建 actor/space，`rig.store.create(actor,space,support::command("旧版"))` 得 original，old_ref 从 original 两个 ID 构造；compositions 为 CompositionStore::new(rig.runtime_pool.clone())。
- [x] 运行 `cargo test --locked -p learning-db --test composition -- --test-threads=1`；最小 store 方法先返回 Storage，以业务断言失败建立 RED。
- [x] 实现事务保存：输入校验 → 锁请求 → 发现并锁完整权限范围 → 同键重放 → 锁身份行并验基础头 → 验既有 occurrence 归属 → 检查精确闭包与路径 → 新修订/出现项/头/登记/receipt 一次提交。既有组合加锁语句：

```sql
SELECT head_revision_id FROM public.composition
WHERE id=$1 AND space_id=$2 FOR UPDATE;
```

  内容闭包验证完成前不落部分出现项；但仍必须测试数据库后段失败回滚。正文按授权后的去重块累计 JSONB 序列化字节，读取超出预算的正文前拒绝；预算错误不得含隐藏对象细节。
- [x] 实现 read 的只读一致快照：

```sql
SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL statement_timeout='15s';
```

  若任一引用不存在/不可读返回 None；输出 compositions 按 `(composition_id,revision_id)` 排序、blocks 按 `(block_id,revision_id)` 排序、nodes 保持原 position。路径计算依据 occurrence 树，不使用这些输出列表顺序推断阅读顺序。
- [x] 补足独立断言：重排 ID 不变；复制生成新 ID；改稳定目标拒绝；移除一次出现不删块；自环与跨历史身份环拒绝；共享子章节两次/菱形合法；错误 revision 归属拒绝；第 16/17 层边界；4,096/4,097 展开项与 8 MiB 边界；旧请求在后续编辑后重放原修订。
- [x] 通过新测试与现有测试后提交：`git add crates/learning-db`，`git commit -m "feat: save and read immutable composition closures"`。

### Task 4：多根原子发布与追加回退

**Files:** 新增 db/release 三文件、tests/release.rs；修改 db/lib.rs、support/assembly.rs。

**Interfaces:** ReleaseStore publish/read/active/state；outbox 事件只存本次 release ID/version/time，从不可变 release_root 解析事实，不复制可独立编辑的根清单。

- [x] 写 RED：A/B 都已发布 K@1，保存采用 K@2 的 A 新修订后发布头仍指向 A 旧版；仅发布 A 后 B 不变。接着以一个命令更新 A/B，故障时两个发布头均保留旧值。
- [x] 运行 `cargo test --locked -p learning-db --test release -- --test-threads=1`，从未实现 publish 的行为失败开始。
- [x] 实现一次事务：规范化 roots → 请求锁 → 闭包空间授权锁 → 精确重放 → UUID 升序锁各根 → 比较每个 expected_head/expected_publication_token → 校验 selected revision 归属/闭包 → 插入 release/roots → 更新全部发布指针 → outbox → request_key/receipt → commit。

```sql
UPDATE public.composition
SET published_revision_id=$1,last_release_id=$2
WHERE id=$3 AND space_id=$4;
INSERT INTO public.outbox_event(id,event_type,aggregate_id,payload_version)
VALUES ($1,'composition_released',$2,1);
```

  不要求被引用块的 head 仍是所选版本，也不自动更新工作头；必须检查组合 expected_head，因为它代表用户发布预览的编辑基础。
- [x] 用夹具管理员创建只针对本测试 author 的 BEFORE INSERT outbox 触发器，执行 `RAISE EXCEPTION 'injected_release_failure'`；触发器通过 NEW.aggregate_id 对 release.author_id 判断作用域。提交失败后移除触发器，核对 release、roots、outbox、receipt、request_key 都未增加，活动指针不变，再用同键重试成功。
- [x] 断言回退新增 release 而非删除历史；旧发布读取仍固定；同键重放不新增事件；同键改 roots/reason/基础头拒绝；读 release 检查全部根，而 active 只检查所选根。
- [x] 通过后提交：`git add crates/learning-db`，`git commit -m "feat: publish composition roots atomically with outbox"`。

### Task 5：权限及并发联合验证

**Files:** 新增 assembly_authorization.rs、assembly_concurrency.rs；只在失败定位需要时修正所属模块。

**Interfaces:** 使用公开 store 与真实 runtime 连接；管理员仅建立 grants、观察阻塞及制造故障。

- [x] 先写失败用例：可写组合空间 + 只读依赖空间可以保存；撤掉依赖空间 grant 后，composition.read/release.read/active 都不可读，save/publish 重放返回 NotFound；无 grant 与随机不存在端点同样返回 NotFound。对错误字符串和序列化结果断言不包含隐藏 UUID/正文。
- [x] 运行 `cargo test --locked -p learning-db --test assembly_authorization -- --test-threads=1`；修正只做根权限检查等具体失败，不能降低预期。
- [x] 并发用 Tokio Barrier 和独立 PG backend PID，至少验证以下独立竞争：同基础组合保存一成一 Conflict；相同请求重复保存只产一份修订；相同发布键只产一份 release/outbox；重叠 A/B 与 B/C 发布一成一 PublicationConflict；互不重叠发布均成功；原内容命令与组合命令共用新请求键只能一个成功，另一个 IdempotencyConflict。

  实际并发形式不引入 futures 依赖：

```rust
let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
let left_barrier = barrier.clone();
let left = tokio::spawn(async move {
    left_barrier.wait().await;
    left_store.save(actor, space, left_command).await
});
let right = tokio::spawn(async move {
    barrier.wait().await;
    right_store.save(actor, space, right_command).await
});
let (a, b) = (left.await.unwrap(), right.await.unwrap());
assert!(matches!((&a, &b),
    (Ok(_), Err(ContentError::Conflict { .. })) |
    (Err(ContentError::Conflict { .. }), Ok(_))));
```

  两个 store 各用独立连接池 max_connections(1)，先记录 SELECT pg_backend_pid() 并断言不同。屏障只确保共同起跑；必须同时核验数据库实际修订/头/事件计数。
- [x] 复现第 3.2 节固定图竞争：A@2→B@1 与 B@2→A@1 均成功，随后 A@3→B@2 拒绝；禁止添加 latest 引用来制造伪测试。
- [x] 权限锁竞争用 pg_blocking_pids 确认撤权/保存确实互相等待；验证撤权先提交时拒绝、保存已锁 grant 时先完成保存再允许撤权。多空间输入乱序不改变 UUID 锁顺序，不靠固定 sleep 判断成功。
- [x] 运行两组测试及原并发/授权测试；提交：`git add crates/learning-db`，`git commit -m "test: verify composition authorization and concurrent publishing"`。

### Task 6：服务器验证、审查与交付文档

**Files:** 修改 README.md、deploy/initdb.sh、deploy/run-tests.sh；新增 docs/p0b1-boundary.md、docs/p0b1-verification.md；更新根文档的实际状态。

**Interfaces:** initdb 增加独立 `learning_upgrade_test` 库，与主业务测试库使用同类角色；升级测试专用 DSN 仅经 secret 在进程环境构造。run-tests 顺序运行普通集成测试及专用升级测试，后者只有空库可运行。

- [x] 在 initdb 中增加隔离升级数据库，用相同 admin/runtime/auth_lock 角色授予与主测试库一致的初始权限。run-tests 在临时目录只复制 0001 文件，设置 TEST_P0A_MIGRATIONS_DIR；不复用保留的 P0-A 数据库。
- [x] 最终全 workspace 测试从新批次的两个空库运行，migration_upgrade 包含在该轮中；若此前单跑过升级测试，最终批次必须另建专用空库，不能对已升级库伪称再次验证升级。避免普通 TestRig 连接升级库。Linux shell 命令为：

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace -- --test-threads=1
docker compose -p learning-system-p0b1 -f deploy/compose.test.yaml --profile test config --quiet
docker compose -p learning-system-p0b1 -f deploy/compose.test.yaml --profile test build test
docker compose -p learning-system-p0b1 -f deploy/compose.test.yaml --profile test up --abort-on-container-exit --exit-code-from test
docker compose -p learning-system-p0b1 -f deploy/compose.test.yaml --profile test stop
```

  宿主测试需预先准备同样两库与专用环境变量；不得在日志输出连接串。Compose 使用全新项目卷，测试结束保留数据卷。repeat 验证另取新项目名，不删除之前证据卷。
- [x] 将固定源码包及逐文件哈希交给「本地 Linux 服务器」任务执行，保留命令与真实退出码，核验运行前后源码相同。复用已有官方镜像缓存；若网络失败，单独记录环境失败及恢复路径，不伪装为业务 RED。
- [x] 验收记录至少包含 P0-A 回归、各新增测试真实数量、升级前后摘要、并发结果、故障注入回滚、角色权限、Compose 实测及局限。未执行的项保留未执行状态。
- [x] 按 requesting-code-review 技能对固定提交做一次只读独立审查，重点检查原始载荷/错误中的隐藏 ID、跨操作幂等、不可变 FK、固定图环语义、发布尾部失败。修正后只重跑受影响检查及必要整体回归。
- [x] 全部通过才写 `P0_B1_VERIFIED`；更新 README 说明“组合与发布内核”，不写成个人插入层或完整系统完成。提交：`git add README.md deploy docs`，`git commit -m "docs: record verified composition and release core"`。

## 6. 验收与下一关交接

| 验收项 | 对应任务 | 必须保留的证据 |
|---|---|---|
| 稳定身份、重复出现、顺序和只升级选定组合 | 1、3、4 | 固定输入与读取输出，不仅比较行数 |
| 多组合原子发布、回退、outbox | 4 | 失败前后数据库状态和同键重试结果 |
| 授权闭包、撤权重放、并发撤权 | 3、5 | 正文、结构、ID、错误与真实阻塞断言 |
| 无环固定结构、合法共享 DAG | 3、5 | 顺序与并发场景及具体版本路径 |
| P0-A 无回归及升级兼容 | 2、6 | 旧摘要、旧修订、旧收据及全部原测试 |
| 空库和非空升级、非所有者角色 | 2、6 | 两条迁移路径、ACL 与实际退出码 |

B1 通过后交给 B2 的只有：精确 CompositionRef、稳定 occurrence 路径、完整闭包读取、当前授权和统一请求事务基础。B2 需要增加个人层专用投影，不能把本关全有或全无读取直接当作“个人笔记不可找回”的理由。

执行复选框已按实际证据完成。Task 2–5 共享的 schema、请求登记与模块合并为集成提交 `6563a3d`，未逐条使用计划中的示例提交命令；Task 1 为 `b890798`，部署和文档另行提交。一次独立审查发现发布恢复缺口，修复后完成 RED→GREEN 和全量回归；具体取舍见 [执行记录](../../p0b1-execution.md)。B2 尚未开始。
