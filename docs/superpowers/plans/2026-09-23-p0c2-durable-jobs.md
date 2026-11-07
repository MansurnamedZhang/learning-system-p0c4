# P0-C2 持久后台任务 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. This document began as the C2 construction plan; Tasks 1–6 were subsequently executed sequentially with an implementer and independent review. The resulting verification is recorded in the isolated worktree's `docs/p0c2-verification.md`.

**Goal:** 在已验收 C1 的精确内容、原字节与授权边界上增加持久任务，让业务事件能被可靠转投、受限 Worker 原子领取，在进程终止和租约易主后安全重试，并完成第一个资产完整性/轻量元信息处理器。

**Baseline:** C1 `feat/p0c-assets` 提交 `a962ab8faa6abc2506f2f47d3e23ae99e2ee781c`；状态 `P0_C1_VERIFIED / NOT_PRODUCTION`，证据见隔离工作树 `docs/p0c1-verification.md`。C2 从该提交及随后**纯文档收口**的已核对状态起步，正式开工时记录准确 HEAD 和工作树状态；不要改写 C1 的已验收源码包、0001–0010 迁移、v1/v2 摘要、旧收据或四套冻结程序。

**Spec:** [P0-C 总设计](../specs/2026-09-23-p0c-assets-jobs-portability-design.md)的 C2 节。C2 只实现通用 outbox/job、租约与首个处理器；C3 快照交换、C4 备份恢复、P1 HTTP/前端与生产部署不在本计划。

## 固定设计边界

- 旧 `outbox_event` 的 `aggregate_id` 外键指向 `release`，事件只允许 `composition_released`；保留旧表与行为。新业务事件进追加的 `job_outbox`，以可审计的 outbox→job 转投协议处理，不能把一次发布事件直接当作可重试作业。
- 首个事件在共享 `block_write::insert_content` 成功写入 v3 Figure/Attachment 的**精确** `block_asset_use` 时，于同一数据库事务写新 outbox；载荷固定 `space_id + block_id + revision_id`、发起 actor、处理器版本和稳定业务幂等键。旧请求键重放旧收据时不重写事件。此首个处理器只自动覆盖已有 v3 块使用关系的资产；仅登记、尚未使用或只关联资源版本的资产不因此自动入队，后续另设计资源版本事件。
- Worker 使用普通非 owner、非 BYPASSRLS 的 runtime 数据库角色。它以事件保存的发起 actor 经 `AssetStore::open_for_use(actor, AssetUseRef::Block(exact_ref))` 复核**当前**授权，不能按摘要、存储键或资产 ID 绕过使用关系。发起者撤权、跨空间或必要引用闭包不可见时，不发布派生结果。此身份模型须在真实 PostgreSQL 负面测试中先冻结；若证据表明现有 API 不足，先修订设计与测试，不暗增权限。
- job 至少记录精确输入、处理器版本、幂等键、状态 `queued/running/retry_wait/succeeded/failed/cancelled`、尝试次数、租约 token/期限、错误类、输出摘要及可选分段检查点。数据库时间判定租约；每次续租、检查点、完成与失败写入均以**当前 token** 条件更新。旧 Worker 在租约转移后无权写结果。执行语义是至少一次，副作用按幂等键收敛，不宣称进程只执行一次。
- 资产检查或解析失败保留已经正确登记的原件和正文；发布成功后预览失败也不撤销发布。C1 上传声明上限、只读对账、媒体有限签名/形状检查仍属 C1 边界，C2 不把它们说成完整解析或自动清理。

## 文件与责任（实施时按实际依赖微调）

| 文件 | 责任 |
|---|---|
| `migrations/0011_jobs.sql` | 只追加 `job_outbox`/`job`、约束、索引与 runtime 最小权限，不修改 0001–0010 或旧 outbox |
| `crates/learning-core/src/job.rs` | 版本化 job 输入/状态/结果类型，确定性幂等键、状态转换验证 |
| `crates/learning-db/src/jobs/{mod,outbox,claim,finish}.rs` | 事务转投、原子领取/续租、fencing、取消/结果读写 |
| `crates/learning-db/src/block_write.rs` | 在 v3 精确使用关系提交事务中产生新 outbox，失败同回滚 |
| `crates/learning-worker/`（如需新 crate） | 受限 Worker 循环、处理器注册、资产完整性/轻量元信息处理器 |
| `crates/learning-db/tests/{jobs_schema,jobs_outbox,jobs_lease,jobs_processor}.rs` | 真 PostgreSQL 权限、原子性、重复消费、旧 token、撤权与处理器结果 |
| `deploy/`、`docs/p0c2-{boundary,execution,verification}.md` | 隔离多进程故障夹具与证据、已实现边界 |

## Task 1：核心契约与追加迁移

**RED：** 写状态转换、坏输入/未知版本、重复幂等键、非法终态复活、runtime 不得 DDL/管理员操作、旧 `outbox_event` 不变的测试；用真实空 PostgreSQL 执行 `jobs_schema` 并记录尚无 0011/接口的实际失败。冻结旧 v1/v2/v3 内容摘要与四类升级期望，不修改旧夹具。

**GREEN：** 只新增 0011、核心 job 类型与最小查询接口。`job_outbox` 以稳定业务键唯一，job 以 outbox 事件 ID 或等价精确键唯一；约束限制状态/attempt/lease 格式，outbox 已转投标记与 job 插入在同事务。runtime 只授予必要列和操作，不能成为库/表 owner。复跑专项、工作区、格式与严格 Clippy，独立审查后提交。

## Task 2：业务事务写事件与持久转投

**RED：** 在 v3 Figure/Attachment 创建或派生的真实路径中，证明块修订、`block_asset_use`、新 outbox 同成功/同回滚；同请求键重放不新增事件，失败的 v3 引用也无事件。模拟转投前终止、插入 job 后记账前终止、重复转投和并发转投，要求一个稳定 job；旧 release `outbox_event` 不被消费成 job。

**GREEN：** 在 C1 共同事务点写版本化事件，转投器使用数据库唯一键与条件记账原子收敛。输入保留精确 BlockRef/actor，不追随 head；不要在 `register_verified` 的无使用资产上隐式排队。运行真实 PostgreSQL 专项、四套旧程序升级、独立审查并提交。

## Task 3：租约状态机与 fencing

**RED：** 两个 Worker 同时领取时只有一方拿到当前 token；租约到期由第二方领取后，第一方的续租、检查点、完成、失败写入均拒绝。测试有限重试/退避、错误分类、取消、已完成终态不能复活、重复完成不重复输出；用可控短租约与数据库时钟避免竞态依赖。

**GREEN：** 单条事务/原子条件更新领取与状态转换，记录 attempt 和租约 token，所有业务结果写入依赖当前 token。过期租约重入 `queued`/`retry_wait` 按明确状态机处理，不能让旧 token 猜中新的租约。真实 PostgreSQL 多连接测试、严格 Clippy、独立审查并提交。

## Task 4：首个真实资产处理器

**RED：** 处理器从 job 的精确 BlockRef 和发起 actor 调用 `AssetStore::open_for_use`；测试当前授权通过、撤权、跨空间、必要引用隐藏、缺失/损坏字节、相同 job 重试、旧 token；撤权或损坏时不得输出成功摘要，正文/原件不回滚。测试限定轻量元信息，不假装完整 PDF/PNG/Notebook 解析或恶意内容扫描。

**GREEN：** 写版本化检查结果与输出摘要，按 job 幂等键和 token 收敛；记录有界错误类与处理器版本，可选检查点只写当前 token。若增预览处理器，另开任务和 RED，不混入本任务验收。复跑 C1 资产授权/综合 Attention、jobs 专项、独立审查并提交。

## Task 5：真实进程终止与恢复

**RED：** 在新隔离 Compose 项目里启动两个实际 Worker 进程，第一进程领取后被真正终止，等租约到期由第二进程完成；检查旧 token 不可再提交、attempt 记录、最终唯一摘要、重复消费收敛、取消阻断和当前授权撤销。保存每个进程、SQL 快照、卷、网络与退出码，不能以同进程模拟替代崩溃。

**GREEN：** 实现可停止/重启的 Worker 入口与受限运行配置，保留可重放 outbox 和队列状态。任何真实故障均先定位并最小修复，再重跑专项和必要全量；独立审查后提交。不得触及既有用户服务或生产数据。

## Task 6：C2 总验收

- [x] 冻结精确提交/源码包/manifest，逐文件核对 Linux 前后；检查 0001–0010、旧 `outbox_event` 和 C1 摘要字节未变。
- [x] 全新隔离六空库/Compose 跑 `cargo fmt --all -- --check`、`cargo clippy --offline --locked --workspace --all-targets -- -D warnings`、`cargo test --offline --locked --workspace -- --test-threads=1`，以及 P0-A/B1/B3-schema/B2 四套冻结旧程序升级；结果以原始日志为准。
- [x] 独立完整审查同事务事件、转投幂等、受限 Worker 授权、租约 token fencing、取消/重试和处理器副作用；真实杀进程/租约易主在新网络/卷中复现，源码前后及每份证据哈希核对。
- [x] 另用**专用 Worker 容器**验收凭据隔离：仅挂 runtime secret、只读权威资产卷和必要临时卷，确认 admin/superuser secret 在容器内不存在或不可读；通过现有业务 API 建真实待处理资产任务，以两个独立 Worker 容器复现 SIGKILL、数据库时钟租约接管、旧 token 拒绝与唯一结果，并复核取消、撤权。不能用共享管理 secret 的测试容器子进程替代此门。
- [x] 上述验收通过后更新 `docs/p0c2-verification.md` 和批准记录为 `P0_C2_VERIFIED / NOT_PRODUCTION`。C3/C4、P1 与生产继续开放。

## 执行交接

用户已选择逐任务子代理实现并独立审查。C2 已按 Task 1→6 顺序完成 RED、GREEN、独立审查和隔离 Linux 总验收；冻结源码提交为 `d286b2e57267996f8e1b6e51ca2ff2c5af580a16`。最终原始证据与限制见 [C2 验证记录](../../p0c2-verification.md)。状态仅为 `P0_C2_VERIFIED / NOT_PRODUCTION`；C3/C4、P1 HTTP/前端及生产部署另行计划和验收。
