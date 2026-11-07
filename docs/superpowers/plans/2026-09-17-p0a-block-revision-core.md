# P0-A Rust 块修订与权限内核 Implementation Plan

> **执行授权更新（2026-09-19）：** 用户已明确要求“开始实施落地”，现按本施工单执行 P0-A，包含必要的编码、构建和隔离 PostgreSQL 验证。P0-B 及后续前端、HTTP 和生产部署不自动纳入本轮。下文原计划的“获准后”条件已对本阶段满足。

**Goal:** 规划一个可独立验收的 Rust + PostgreSQL 内容内核，保证块修订不可变、历史可读、当前权限有效、重试幂等及并发更新不静默覆盖。

**Architecture:** Cargo workspace 将领域契约与数据库实现分开。P0-A 交付受信任应用内部调用的库及测试，不暴露 HTTP；后续 API 和前端调用同一业务内核。原件、组合、个人层、关系和学习事实保持已批准的独立边界。

**Tech Stack:** 用户指定 Rust 后端 + Web 前端；P0-A 已采用 Serde、Tokio、SQLx 和 PostgreSQL，Rust 1.97.0 与依赖锁文件经过实际验证。后续 HTTP 建议 Axum，前端建议 React + TypeScript + Vite；后续框架仍属于建议，不把环境中已安装的版本视为验收结果。

**Spec:** [批准记录与实施路线](../../设计批准与实施路线.md)、[总体设计](../../学习系统设计方案.md)、[存储规范](../../内容分块与存储规范.md)、[连续阅读与思考块设计](../../连续阅读与思考块设计.md)、[数据库实验审阅](../../数据库选型实验审阅与首版决策.md)。

**Status:** `P0_A_VERIFIED / NOT_A_PRODUCTION_RELEASE`。任务已按实际证据验收：宿主与两次空库 Compose 均为 29/29 通过，静态检查通过，独立审查无发现。已有试作保留 Git 基线，未直接折算为完成。详见[P0-A 验收记录](../../p0a-verification.md)。

## Global Constraints

获批设计中的共同约束：

- “内容块是最小编辑和版本单位。”
- “稳定身份与内容修订分开。”
- “组合只保存引用、结构和局部展示信息。”
- “个人事实记录与内容分开。”
- “原件独立且不可变。”
- “同一幂等键与同一请求内容重试返回原结果；同键不同内容拒绝。”
- “首版默认单人私有实例。”

本阶段范围进一步限定为：

1. 实现原创文本块 `kind=text`、`payload.format=markdown`；用途支持 knowledge/note/question/idea/conjecture/observation/evidence/conclusion。用途标签不自动赋予“已证明”状态。
2. 首阶段不接受其它块形式或跨块语义字段；对未知字段明确拒绝。完整 `basis_refs`、个人 `target`、关系与上下文依赖在 P0-B 引入时补充严格契约和权限回归。
3. 身份由未来服务端受信任入口或受控测试夹具提供；客户端正文不得声明作者或权限。当前主体模型不等于实现了登录。
4. 只推进当前编辑头，不自动发布文档、升级组合或更改学习事实。原子组合发布属于 P0-B。
5. 单条、批量、列表、历史及幂等重试都核对当前权威授权；历史存在不赋予永久访问权。
6. 数据库管理员仅用于迁移、夹具和内部核验；业务测试必须使用真实非超级用户、非表所有者的运行角色。
7. 不将 SQLite、内存返回或只编译成功视为 PostgreSQL 行为验证。未运行、失败与通过分别记录。
8. 本阶段无 HTTP 服务、访问密钥登录、前端工作台或公开预览；这些归入 P1-A。前后端字段和错误语义在本计划中先明确。

---

## 1. 交付边界

| 本阶段交付 | 后续阶段交付 |
|---|---|
| 严格文本块契约、稳定 ID、内容摘要 | 公式、代码、图片及原件等其它块形式 |
| 新块、不可变修订、当前编辑头 | 固定组合、发布闭包与选择升级 |
| 当前授权下的正文与历史读取 | 载荷内部引用、关系路径、计数与导出权限 |
| 请求幂等、真实并发冲突、数据库事务回滚 | 个人层位置冲突、结构环并发防护 |
| 机器可核对的 JSON 样例与 Rust 类型 | OpenAPI、TypeScript 客户端、登录和 Web 编辑器 |

P0-A 验收对象是内容内核，不是可部署的完整学习系统。本文替代原 Python 施工单，并恢复原先“内核先行”的分期；[旧 Python 稿](../../archive/2026-09-17-p0a-python-plan-superseded.md)只作历史参考。另一份 `2026-09-17-p0a-rust-block-core.md` 中提前加入 API/前端的计划不作为当前执行依据。

## 2. 文件结构与责任

源码根目录为 `仓库根目录`。2026-09-19 获准后按下列结构创建或修订；旧试作已保存基线并复核差异。

```text
Cargo.toml / Cargo.lock                        # workspace 与锁定依赖
rust-toolchain.toml                            # 经验证的固定工具链
.gitignore                                    # 排除凭据、资产、target 和运行产物
.gitattributes                                # 跨平台源码与脚本换行
.dockerignore                                 # 构建上下文只包含代码和测试所需文件
crates/learning-core/
  Cargo.toml
  src/lib.rs                                  # 公共领域边界
  src/content.rs                              # TextDraft、Intent、Revision
  src/identity.rs                             # Principal
  src/error.rs                                # ContentError
  src/digest.rs                               # 规范序列化与摘要
  tests/contracts.rs                          # 非法输入与内容保真
crates/learning-db/
  Cargo.toml
  src/lib.rs                                  # ContentStore 与迁移入口
  src/write.rs                                # 创建、修订、收据事务
  src/read.rs                                 # 当前授权下的读取
  tests/support/mod.rs                        # 测试数据库、主体与空间夹具
  tests/schema.rs                             # 外键与运行角色权限
  tests/revisions.rs                          # 历史与幂等
  tests/authorization.rs                      # 当前授权及正文输出
  tests/concurrency.rs                        # 多连接竞争
  tests/atomicity.rs                          # 实际写入故障与回滚
migrations/0001_content_core.sql
contracts/content-draft.example.json           # 内部契约示例，不是已发布交换标准
docs/content-boundary.md                       # 供后续 API / Web 使用的字段语义
docs/p0a-verification.md                       # 实际验收记录，开始实施后填写
deploy/compose.test.yaml                       # 隔离数据库与 Rust 测试运行器
deploy/Dockerfile.test
deploy/initdb.sh                               # 独立测试角色和空数据库初始化
deploy/run-tests.sh                            # 从密钥文件注入连接并运行测试
README.md
```

领域 crate 不依赖 SQLx 或 HTTP。数据库 crate 依赖领域 crate；未来 API 依赖两者。前端只依赖对外数据契约，不导入数据库行结构。P0-A 不创建 `apps/web` 或 API 空壳。

## 3. 统一契约

### 3.1 内容与身份

| 类型 | 字段和约束 |
|---|---|
| `Principal` | `actor_id: Uuid`，只由受信任内部入口构造 |
| `TextDraft` | kind=text；intent 为八种枚举；language/title 为字符串；payload 为严格 TextPayload |
| `TextPayload` | format=markdown；text 为原样保留的字符串；未知字段拒绝 |
| `Revision` | block_id、revision_id、parent_revision_id、draft、content_sha256；创建时间、作者、修改理由保留在修订审计字段中 |
| `CreateCommand` | request_id、draft、reason；目标 space_id 作为方法参数 |
| `ReviseCommand` | request_id、base_revision_id、draft、reason；目标 block_id 作为方法参数 |
| `PageCursor` | 查询内的排序时间与稳定 ID；只用于该类列表续读，不代表固定发布快照 |
| `RevisionPage` | items、next_cursor；只根据有权读取的对象计算 |

建议首阶段约束：标题最多 300 个 Unicode 字符，正文最多 200,000 个 UTF-8 字节，language 为 1–35 个 ASCII 字符，修改理由为 1–1,000 个 Unicode 字符。允许空正文草稿，不对标题、正文、代码样式文本做隐式 trim 或格式重写。这些是本阶段已采用并验证的实现参数。

block_id 与 revision_id 由服务生成；request_id 由调用方在首次提交前生成并保存，后续重试复用。actor_id 与 space_id 来自已有身份和空间，正文不得指定新块或修订的身份。内容哈希用于检测变化，不充当块身份；同内容的两个新块仍获得不同 block_id。摘要输入包含契约版本、kind、intent、language、title 和 payload，不含 ID、作者、时间和修改理由。对象键排序、数组顺序及字符串原值保留；用固定样例锁定字节和摘要规则，不依赖碰巧的字段声明顺序。

`TextDraft` 的反序列化与服务写入都需校验，避免 Rust 内部直接构造绕过长度限制。当前示例是 P0-A 内部契约，不能据此宣称完整交换格式 1.0 已发布。

### 3.2 服务方法

数据库实现提供以下异步方法；`ContentStore::new(pool: PgPool)` 绑定运行角色连接池。类型由领域 crate 定义，存储错误在边界转为 ContentError。

```rust
pub async fn create(&self, actor: Principal, space_id: Uuid,
    command: CreateCommand) -> Result<Revision, ContentError>;
pub async fn revise(&self, actor: Principal, block_id: Uuid,
    command: ReviseCommand) -> Result<Revision, ContentError>;
pub async fn read(&self, actor: Principal, revision_id: Uuid)
    -> Result<Option<Revision>, ContentError>;
pub async fn read_many(&self, actor: Principal, revision_ids: &[Uuid])
    -> Result<Vec<Revision>, ContentError>;
pub async fn list(&self, actor: Principal, space_id: Uuid, cursor: Option<PageCursor>)
    -> Result<RevisionPage, ContentError>;
pub async fn history(&self, actor: Principal, block_id: Uuid, cursor: Option<PageCursor>)
    -> Result<RevisionPage, ContentError>;
```

`read` 对无权限与不存在均返回 None。`read_many` 建议最多接受 200 个输入 ID，按首次出现顺序去重并返回可见对象，不输出隐藏数量。列表建议每页 100 项；先授权再排序和分页，历史按修订创建时间与 ID 排序，块列表按块创建时间与 ID 排序。当前头列表会随编辑变化，不称为不可变文档快照。

错误语义：Invalid 表示输入不符合契约；NotFound 表示写入目标不存在或无当前权限；Conflict 携带仍有权访问的 current_revision_id；IdempotencyConflict 表示同键不同请求；Storage 表示内部存储错误，详细 SQL 和连接信息不进入用户响应。P1-A 再将这些错误映射为 HTTP 状态和前端操作。

### 3.3 与前端的衔接

后续 JSON 中 UUID 使用字符串，时间使用 UTC RFC 3339，kind/intent 使用稳定的小写枚举。TypeScript 类型、API schema 与 Rust 序列化必须通过同一组固定样例核对，不能各自猜字段。

前端应区分“编辑头”“正在查看的旧修订”“未提交草稿”和“尚未确定结果的请求”。网络重试复用原 request_id 及完整载荷；冲突保留草稿，由用户明确合并或另存。历史预览不会直接写回当前头。这些是后续接口要求，本阶段不实现页面行为。

## 4. 任务与验收步骤

以下复选框在相应实际验收完成后更新。命令与代码片段是执行指引，实际版本、结果和差异另记入实施记录。

### Task 1：领域类型、严格校验与内容摘要

**Files:** 创建或修订 workspace 文件、`learning-core` 的 content/identity/error/digest 模块、`tests/contracts.rs`、`contracts/content-draft.example.json` 和 `docs/content-boundary.md`。

**Interfaces:** 消费严格 JSON，产出第 3 节领域类型；`TextDraft::validate() -> Result<(), ContentError>`；`canonical_content(&TextDraft) -> Vec<u8>`；`content_digest(&TextDraft) -> String`。

- [x] **写行为测试。** 覆盖伪造 author_id、未知 basis_refs、payload 内多余 target、非法 kind/intent/format、长度限制、空草稿，以及空格/换行保真。下面示例中的预期来自契约，不由实现计算。

```rust
#[test]
fn unknown_fields_cannot_bypass_the_content_contract() {
    let raw = r#"{"kind":"text","intent":"note","language":"zh-CN",
      "title":"笔记","payload":{"format":"markdown","text":"原样内容"},
      "author_id":"forged"}"#;
    assert!(serde_json::from_str::<TextDraft>(raw).is_err());
}
```

- [x] **观察缺失行为的失败。** 运行 `cargo test -p learning-core --test contracts`；缺工具链、下载失败或编译错误记为环境问题，不算业务失败。
- [x] **实现最小契约。** Serde 使用严格枚举和 deny_unknown_fields；显式校验处理大小和内部构造对象；规范序列化含版本并稳定排序后计算 SHA-256。
- [x] **核对摘要与类型映射。** 使用人工审阅的规范 JSON 字节和独立算出的固定摘要；键顺序改变不影响摘要，正文字符、用途或版本改变会影响摘要。记录字段对后续 JSON/TypeScript 的映射。
- [x] **验收。** 单元用例、`cargo fmt --check` 和 `cargo clippy -p learning-core --all-targets -- -D warnings` 通过；锁定实际依赖，不把所有领域形式标为已支持。

### Task 2：数据库结构、连接身份与不可变约束

**Files:** `migrations/0001_content_core.sql`、`learning-db/src/lib.rs`、`tests/support/mod.rs`、`tests/schema.rs`、`deploy/compose.test.yaml`、`deploy/Dockerfile.test`。

**Interfaces:** `ContentStore::new(PgPool)`、独立迁移入口；测试辅助 `TestRig::from_env()` 读取显式测试连接配置，提供 admin_pool、runtime_pool、`seed_actor_space(can_write)` 和 `revoke(actor_id, space_id)`。辅助函数只存在测试目录。

- [x] **先定义并测试结构行为。** 运行身份不是超级用户、不是表所有者；跨块父修订、自指父修订、错误工作头被数据库拒绝；运行角色无法 UPDATE/DELETE/TRUNCATE 修订。夹具通过管理连接插入字面基线，不依赖尚未实现的业务服务。
- [x] **准备隔离环境。** 仅在未来已授权范围内提供 TEST_ADMIN_DATABASE_URL 和 TEST_DATABASE_URL；凭据不写源码、命令输出或报告。前者迁移与布置数据，后者是真实非所有者登录角色。
- [x] **实施以下最小关系模型。** 独立角色由环境初始化配置建立，迁移不使用业务连接提升权限。

| 表 | 键与必须约束 |
|---|---|
| app_user | id 为 UUID 主键 |
| space | id 主键；owner_id 引用 app_user |
| space_grant | (actor_id, space_id) 主键且分别引用用户和空间；can_write 非空；授权存在表示可读，所有者也显式建立授权 |
| block | id 主键；space_id 外键；非空 head_revision_id；created_at；(space_id,id) 唯一 |
| block_revision | id 主键；(space_id,block_id) 引用 block；parent_revision_id 可空且不得等于自身 id；content、摘要、作者外键、理由、创建时间非空；(space_id,block_id,id) 唯一 |
| mutation_receipt | (actor_id,request_id) 主键；actor_id 外键；非空完整请求摘要与结果 revision_id 外键 |

父修订与工作头必须属于同一空间和块，核心约束形态为：

```sql
FOREIGN KEY (space_id, block_id, parent_revision_id)
  REFERENCES block_revision(space_id, block_id, id);
-- 在 block 表上定义，允许同一事务完成新块与其首修订：
FOREIGN KEY (space_id, id, head_revision_id)
  REFERENCES block_revision(space_id, block_id, id)
  DEFERRABLE INITIALLY DEFERRED;
```

- [x] **落实角色与索引。** 运行角色读取空间授权、插入块/修订/收据，只能更新 block 的 head_revision_id 列；不授予修订或收据更新、删除、TRUNCATE、授权表写入和结构变更权。按空间/创建时间/ID、块/创建时间/ID 建读取索引。业务用户级授权仍须由 Task 4 的查询执行，数据库角色不是其替代。
- [x] **封装授权行锁。** PostgreSQL 的 `FOR SHARE` 也要求目标表至少一列的 UPDATE 权限，因此不能让仅可读授权表的运行角色直接执行它。[SELECT 权限说明](https://www.postgresql.org/docs/18/sql-select.html)。规划由迁移建立 `lock_space_grant(actor_id, space_id)` 函数：只锁定指定授权行并返回 can_write，不新增或修改授权；无行返回空结果。函数采用权限受限的 NOLOGIN 所有者与 SECURITY DEFINER，固定可信 search_path 并将 pg_temp 放最后，表名完全限定、无动态 SQL；同一迁移事务内撤销 PUBLIC 执行权，只向运行角色授予 EXECUTE。[函数权限要求](https://www.postgresql.org/docs/18/sql-createfunction.html#SQL-CREATEFUNCTION-SECURITY)。schema 测试须证明运行角色能调用函数，但不能改权、替换函数或通过临时同名表改变结果。
- [x] **验收。** 运行 `cargo test -p learning-db --test schema -- --test-threads=1`，以实际数据库错误与普通运行角色结果核对所有结构断言。

### Task 3：创建、修订与幂等写入

**Files:** `learning-db/src/write.rs`、`tests/revisions.rs`；复用内容类型、错误和测试夹具。

**Interfaces:** 实现第 3 节 create/revise；每次调用独立取得连接并使用一个事务，管理连接不进入业务服务。

- [x] **写保存行为测试。** 创建两次相同内容但不同 request_id 得到不同块；相同请求重试得到同一结果；旧版内容不变；同键不同正文/目标/基础版本/修改理由均拒绝。初始断言通过管理连接检查持久化行和工作头，避免依赖未实现的读取方法。
- [x] **运行失败用例。** `cargo test -p learning-db --test revisions -- --test-threads=1`；记录业务失败后再实现，不把缺数据库配置标为预期失败。
- [x] **实现以下事务顺序。**

```text
校验命令与正文
BEGIN
  按 actor_id + request_id 获取事务级请求锁
  调用受限 lock_space_grant，锁住当前目标空间授权行并检查 can_write
  检查收据：同键同完整请求则返回原结果；不同请求则拒绝
  新块：生成稳定块 ID；修订：FOR UPDATE 锁块并读取锁后的工作头
  比较 base_revision_id，过期则冲突
  插入不可变修订 → 更新编辑头 → 插入结果收据
COMMIT
```

请求锁可使用 PostgreSQL 事务级 advisory lock，数据库对象锁保证多个进程竞争时仍正确：

```rust
sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
    .bind(format!("{}:{}", actor.actor_id, command.request_id))
    .execute(&mut *tx).await?;
```

收据摘要包含操作类别、目标空间/块、基础版本、规范正文和理由。返回旧收据前仍检查当前权限；新增修订继承稳定块身份，作者由 Principal 写入。锁超时或存储失败不能被伪装成保存成功或版本冲突。

- [x] **补核失败路径。** 只读空间拒绝写入，跨块基础版本拒绝，保存已提交但响应丢失后用同键找回原结果；授权撤销后不能借收据取回正文。
- [x] **验收。** 所有写入用例通过，确认草稿保存只推进编辑头，没有写入组合、发布或学习记录。

### Task 4：当前授权下的正文、列表与历史

**Files:** `learning-db/src/read.rs`、`tests/authorization.rs`。

**Interfaces:** 实现 read/read_many/list/history；复用 Revision 和 RevisionPage，不返回原始数据库行或连接错误。

- [x] **写当前权限测试。** 两个测试主体分别持有空间；另设只读授权。先能读旧/新修订，撤销授权后再发请求时，单条、批量、列表、历史和重试均不能读回隐藏内容。
- [x] **运行并确认缺失过滤会失败。** `cargo test -p learning-db --test authorization -- --test-threads=1`。无权主体也应与随机不存在对象得到一致的外部结果。
- [x] **实现授权与正文同次查询。** 单条与批量复用同一受控路径，SQL 形态为：

```sql
SELECT r.id, r.block_id, r.parent_revision_id, r.content, r.content_sha256
FROM block_revision r
JOIN block b ON (b.id, b.space_id) = (r.block_id, r.space_id)
JOIN space_grant g ON g.space_id = b.space_id
WHERE g.actor_id = $1 AND r.id = ANY($2);
```

这是未来数据库实现片段；所有绑定参数由受信任服务调用提供。调用者不能拼接 SQL 或传入任意“可见空间清单”绕过权威授权。

- [x] **覆盖批量与分页。** 混合可见、不可见、重复和不存在 ID，检查顺序与去重；空输入返回空集合，超限输入拒绝。列表分页只基于已过滤结果生成 next_cursor，不暴露隐藏总量。权限撤销测试明确在撤销事务提交后发起新请求，不声称能收回已经完成的历史响应。
- [x] **验收响应内容。** 断言隐藏块 ID、修订 ID、标题和正文不进入序列化结果；只读授权可读不可写。P0-B 的载荷内引用、路径和计数过滤另有验收，不能由本任务替代。

### Task 5：真实竞争、事务回滚与重试恢复

**Files:** `tests/concurrency.rs`、`tests/atomicity.rs`；根据失败修正 write.rs，不在生产类加入仅供测试使用的开关。

**Interfaces:** 测试辅助提供两个独立运行角色连接及管理断言；业务方法不接受“跳过权限”“注入错误”等测试参数。

- [x] **设置真实竞争。** 两个独立连接经 Barrier 从同一基础修订同时保存，不同 request_id；分别持久记录成功结果或 Conflict。命令未来为 `cargo test -p learning-db --test concurrency -- --test-threads=1`。

```rust
// left / right 是两个实际异步保存调用返回的结果。
let results = [left, right];
assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
assert_eq!(results.iter().filter(|r|
    matches!(r, Err(ContentError::Conflict { .. }))).count(), 1);
```

另检查成功结果是新工作头、冲突指向该工作头，并且只有一个新修订。连接 backend PID 不同的诊断可以记录，连接串不得记录。

- [x] **验证重复请求与独立块。** 同键同载荷并发只产生一份修订/收据；同键异载荷只能有一个成功；不同块并发均能正确提交。结果的权限判断不能被幂等缓存绕过。
- [x] **验证撤权与写入竞争。** 保存先取得授权行共享锁时，撤权或降为只读的事务等待保存结束；撤权先提交时，后续保存及重试拒绝。使用独立管理连接执行撤权，实际观察等待与提交顺序；不以线程睡眠猜测竞争已发生。
- [x] **注入真实数据库失败。** 仅在隔离测试库创建限定本测试 actor_id 的触发器，使插入 mutation_receipt 失败。通过正常 revise 触发，在 fixture 中移除触发器；不模拟一个 Rust 错误返回冒充数据库回滚。
- [x] **验证完整回滚。** 新修订不存在、head 保持旧值、无成功收据；移除故障后以同一请求重试可成功且只生成一次。运行 `cargo test -p learning-db --test atomicity -- --test-threads=1`。
- [x] **验收边界。** 行锁与事务结果通过多连接测试；应用层进程锁不作为正确性证据。组合结构环和个人层位置竞争属于 P0-B，不将本任务写成“所有并发安全已完成”。

### Task 6：获准后的隔离重跑与交付审阅

**Files:** README.md、docs/p0a-verification.md、锁文件及测试部署配置；日志与产物写入忽略版本管理的 artifacts/p0a/。

**Interfaces:** 输入固定源码、契约、迁移和测试，输出可重复命令、版本信息、源文件摘要、通过/失败/未运行清单及限制。

- [x] **确认执行前提。** 用户已明确授权该阶段及对应测试范围后，才决定原试作的复用范围和本地/服务器运行方式。未获授权时停留在计划状态，不发送服务器请求。
- [x] **验证 Rust 静态和领域检查。** 命令示例仅供获准后执行：

```text
cargo fmt --check
cargo test --locked -p learning-core --test contracts
cargo clippy --locked --workspace --all-targets -- -D warnings
```

- [x] **运行真实 PostgreSQL 检查。** 明确提供迁移与运行两种身份，逐个运行 schema、revisions、authorization、concurrency、atomicity 集成目标。不能通过自动跳过、忽略测试或禁用断言取得“全绿”。
- [x] **需要服务器协作时交付固定产物。** 在当次授权范围内通知既有「本地 Linux 服务器」任务，使用同一源码清单和精确命令；拟用独立 learning-system-p0 项目、2 CPU/4 GiB 数据库、无宿主数据库端口。是否复用已停止实例先纳入当次计划；不得修改原图对比归档或业务数据库。
- [x] **审阅后再标记完成。** 对照下面验收表，记录真正运行的版本、角色、连接竞争、故障注入与结果。仅完成文档、下载依赖或编译不能标记 P0-A 完成。后续是否进入 P0-B，由用户给出的实施范围决定，不自动扩展。

## 5. 阶段验收表

| 验收项 | 必须成立的结果 | 本轮状态 |
|---|---|---|
| 类型与保真 | 未知字段拒绝，正文原样，摘要规范明确 | 已验证通过 |
| 身份与修订 | 块 ID 稳定、修订 ID 唯一、父版本属于同块 | 已验证通过 |
| 历史 | 修改后旧修订不变，不能被运行角色覆盖或删除 | 已验证通过 |
| 当前授权 | 单条/批量/列表/历史/收据不绕过撤权 | 已验证通过 |
| 幂等 | 同键同请求同结果，同键异请求拒绝 | 已验证通过 |
| 并发 | 同基础版竞争一个成功一个冲突，无静默覆盖 | 已验证通过 |
| 原子性 | 收据阶段失败时正文、头和收据全部回滚 | 已验证通过 |
| 数据库角色 | 普通运行身份完成业务，缺少结构和历史篡改权限 | 已验证通过 |
| 契约边界 | 后续 API/前端可据字段与错误语义接入，无 HTTP 或 UI 已完成的宣称 | 字段契约已验证；HTTP/UI 留待后续 |

P0-A 的全部关键项通过后，才能把该内核作为 P0-B 的依赖。独立的实验失败项、完整生产授权、发布、位置、图片、恢复与学习流程继续按实施路线验证。

## 6. 本次修订结论

2026-09-17 已将本文件恢复为完整 Rust 首阶段计划；2026-09-19 用户明确要求开始实施，P0-A 已通过验收，本轮完成。分期保持“内核 → 组合/个人层/关系 → 原件与恢复 → API/前端 → 学习闭环”。本阶段采用 SQLx；React、Axum 仍是后续建议，不冒充用户已经逐项指定的选型。实际验证与差异见 [P0-A 验收记录](../../p0a-verification.md)。
