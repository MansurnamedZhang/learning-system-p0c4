# P0-A Rust 块修订与前端切片 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. 当前任务持有源码，既有「本地 Linux 服务器」任务承担隔离运行，不另写业务实现。

**Goal:** 用 Rust + PostgreSQL 实现不可变块修订与当前权限，通过 React 前端完成文本块创建、修订和历史查看。

**Architecture:** Cargo workspace 分为领域契约、数据库和 API 三个 crate；前端单独构建。API 用显式事务操作数据库，前端只经 HTTP 读写权威内容。图片、组合、个人位置、关系和学习功能按既有分期接入。

**Tech Stack:** Rust 2024 edition；Tokio、Axum 0.8、SQLx 0.8、Serde；PostgreSQL 18；React、TypeScript、Vite。依赖精确版本保存在 Cargo.lock 和 package-lock.json。

**Spec:** [Rust 技术方案](../../Rust与前端技术方案.md)、[批准与实施路线](../../设计批准与实施路线.md)、[存储规范](../../内容分块与存储规范.md)。原 Python 计划已归档。

## Global Constraints

- “内容块是最小编辑和版本单位。”
- “稳定身份与内容修订分开。”
- “同一幂等键与同一请求内容重试返回原结果；同键不同内容拒绝。”
- “个人事实记录与内容分开。”
- “首版默认单人私有实例。”
- 首阶段只实现 text/markdown 严格正文，用途使用获批八种枚举；其它形式和跨块载荷拒绝，不静默吞字段。
- 不复制实验的无授权正文 helper；单条、批量、列表与历史均查询当前空间授权。
- 真实 PostgreSQL 测试不能由 SQLite 或内存实现替代；未运行明确记录。
- 新测试项目为 learning-system-p0，不重置原图对比实验与既有业务服务。
- 作者来自服务端验证的身份，不接收请求中的 actor_id；首阶段访问密钥为开发入口，不等同完整生产会话系统。

## 文件结构与接口

所有路径以 `learning-system/` 为根。

```text
Cargo.toml / Cargo.lock
crates/learning-core/src/lib.rs         # TextDraft、Intent、Revision、校验、摘要
crates/learning-core/tests/contracts.rs
crates/learning-db/src/lib.rs           # ContentStore
crates/learning-db/src/read.rs          # 权威授权读取
crates/learning-db/src/write.rs         # 幂等、修订、事务
crates/learning-db/tests/postgres.rs     # 真实连接及故障注入
crates/learning-api/src/lib.rs          # Axum Router
crates/learning-api/src/main.rs         # 配置、迁移/初始化与启动
crates/learning-api/src/auth.rs         # 密钥绑定主体
crates/learning-api/tests/http.rs
apps/web/src/App.tsx                    # 块工作台与草稿状态
apps/web/src/api.ts                     # HTTP 客户端和错误
apps/web/src/types.ts                   # 与 Rust 对应的 JSON 类型
apps/web/src/styles.css
migrations/0001_core.sql
deploy/compose.test.yaml
deploy/Dockerfile.test
README.md
docs/p0a-verification.md
```

TextDraft 字段为 kind=text、title、intent、language、payload={format:markdown,text}。Revision 含 block_id、revision_id、parent_revision_id、draft、content_sha256。Principal 只由受信任边界创建。ContentStore 接口为异步 create、revise、read、read_many、list、history；写命令带 request_id、draft、reason，revise 另含 base_revision_id。业务错误为 Invalid、NotFound、Conflict(current_revision_id)、IdempotencyConflict、Storage。

## Task 1：严格领域契约

**Files:** workspace Cargo.toml、learning-core crate、contracts.rs、.gitignore。

**Interfaces:** 消费明确 JSON；产出上述领域类型及稳定内容摘要。

- [ ] 先写拒绝未知字段、错误用途、伪造作者、超长正文及稳定摘要测试；预期值用字面数据。
- [ ] 最小占位实现必须可编译；运行 `cargo test -p learning-core --test contracts`，确认行为断言失败。缺依赖和编译错误不算行为失败。
- [ ] 用 Serde deny_unknown_fields、枚举、显式 validate 实现契约，保留正文空格与换行；SHA-256 只覆盖规范内容。

```rust
#[test]
fn client_cannot_supply_an_author() {
    let raw = r#"{"kind":"text","intent":"note","language":"zh-CN",
      "title":"笔记","payload":{"format":"markdown","text":"内容"},
      "author_id":"forged"}"#;
    assert!(serde_json::from_str::<TextDraft>(raw).is_err());
}
```

- [ ] 运行测试、fmt 和 Clippy，保留实际生成的 Cargo.lock。

## Task 2：不可变数据库与真实事务

**Files:** learning-db crate、read.rs、write.rs、postgres.rs、migrations/0001_core.sql。

**Interfaces:** ContentStore::new(PgPool)；管理连接只用于迁移和 fixture，运行池使用非所有者登录角色。

- [ ] 先写历史保留、撤权、幂等、两连接冲突及运行角色禁止 UPDATE/DELETE 修订的测试。
- [ ] 独立 PG 上记录未实现行为的失败；不得未执行数据库测试就标通过。
- [ ] 建 app_user、space、space_grant、block、block_revision、mutation_receipt。父修订、头通过复合外键约束到同空间/块；运行角色只可读/插入修订、按列更新头，不可改空间归属或删除修订。
- [ ] 实现顺序：锁请求 ID → 当前授权 FOR SHARE → 校验收据 → 工作头 FOR UPDATE → 比基础版本 → 新修订/头/收据 → 一起提交。

```rust
let mut tx = pool.begin().await?;
sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
    .bind(format!("{}:{}", actor.id, request_id))
    .execute(&mut *tx).await?;
// 后续权限、收据和写入均使用同一 tx；出错返回时回滚。
tx.commit().await?;
```

收据的请求摘要包含操作、目标、基础版本、完整正文和理由；相同请求重试仍需当前权限。读取在 SQL 内连接授权；批量按输入首次出现顺序去重，不返回隐藏数量；列表与历史限制长度并稳定排序。

- [ ] 真正用 PG 触发器在 INSERT receipt 失败，断言新修订、头、收据全部回滚。
- [ ] 用两个连接和 Barrier 竞争同一基础版，一个成功另一个冲突；同请求并发返回同一修订。不要用进程内锁替代数据库正确性。

## Task 3：受保护 HTTP 接口

**Files:** learning-api crate、auth.rs、lib.rs、main.rs、tests/http.rs。

**Interfaces:** GET /api/health；受保护 GET/POST /api/blocks、GET /api/blocks/{id}/history、POST /api/blocks/{id}/revisions、GET /api/revisions/{id}。

- [ ] 先写无/错密钥拒绝测试，使用真实 Router 请求；认证失败不能进入数据库调用。
- [ ] 实现密钥摘要常量时间比较，绑定服务端 actor/space；密钥不写日志。不要开启第三方 CORS。
- [ ] 路由只调用 ContentStore；错误映射 404/409/422，数据库细节不返回浏览器。冲突只向有权限调用者返回当前修订 ID。
- [ ] 普通启动使用 runtime DSN；显式 migrate/bootstrap 子命令才读取管理 DSN。默认绑定 127.0.0.1；密钥缺失拒绝启动。
- [ ] HTTP 测试通过后，以真实 PG 跑创建→修订→读取历史流程；保留状态码及正文检查。

## Task 4：React 文本块工作台

**Files:** apps/web 的 package.json、锁文件、Vite/TS 配置、入口、App.tsx、api.ts、types.ts、styles.css 和行为测试。

**Interfaces:** JSON 字段和 Rust 序列化一致，UUID 使用 string；统一结构化 API 错误；密钥仅在内存，不进入 URL 或 localStorage。

- [ ] 先写失败响应不能当成功、失败保留草稿、冲突不能自动覆盖、历史查看不丢草稿的行为测试。
- [ ] 实现密钥入口、块列表、用途标签、编辑表单、保存状态及只读历史。只展示服务器实际结果；正文以文本渲染。
- [ ] 创建/修订产生 request_id；失败重试复用同 ID 和同载荷，编辑改变后另开请求。结果不确定时不盲目换 ID 重复写入。
- [ ] 409 保留草稿，展示当前版本和用户明确操作；不能静默改 base_revision_id 重提。历史查看与当前编辑分开。
- [ ] 运行前端行为测试、TypeScript 检查及生产构建；验证窄屏和键盘路径。

## Task 5：隔离重跑与交付

**Files:** deploy 配置、README.md、docs/p0a-verification.md；修复实测问题。

- [ ] 本地跑 `cargo fmt --check`、单元/HTTP 测试、`cargo clippy --workspace --all-targets -- -D warnings`；数据库测试单独明确运行。
- [ ] 将固定源码摘要和检查命令交付「本地 Linux 服务器」；独立 PG 2 CPU/4 GiB，无宿主数据库端口。
- [ ] 运行 Rust 编译、真实 PG 测试、前端构建与 HTTP 流程；记录版本、角色、失败及未运行项。
- [ ] 来源任务审核输出和断言，停止仅本测试服务，保留证据。
- [ ] 更新任务状态；只有实测成功才勾选完成。图片、个人位置、关系、原子发布、恢复和正式部署仍按后续里程碑推进。

## 自检

正式后端、数据库层、HTTP 与测试已转为 Rust；前端独立使用 React/TypeScript。PostgreSQL 权威性、历史、权限及原件边界不变。先接一个块工作台属于实施顺序调整，不删减长期能力。旧 Python 对比实验继续作为历史证据。
