# 知织 · KnowWeave

知织是以“块”为最小修订单元的个人学习与知识系统内核。当前代码以 Rust、PostgreSQL 18 和私有文件存储实现内容、关系、阅读、资产、后台任务与快照能力；**没有 HTTP API、登录界面或前端，也不是生产部署版本**。

## 项目状态

| 阶段 | 能力 | 状态 |
| --- | --- | --- |
| P0-A、P0-B1/B2 | 块修订、固定组合、原子发布、个人阅读 | 隔离验收通过；非生产 |
| P0-B3/B4 | 关系、证据/猜想、影响查询 | 隔离验收通过；非生产 |
| P0-C1/C2/C3 | 原件与引用、持久 Worker、融合快照交换 | 隔离验收通过；非生产 |
| P0-C4 | 完整备份与干净实例恢复 | **实施中，未整体验收** |

C4 已通过部分单机隔离门，包括备份清单、安全封存、恢复目标准入、出生证明、干净目标 pin **候选**，以及精确子进程的只读端点绑定/同一 guard 重启拒绝（新批次结果已核对）。独立备份目标、`CompleteBackup`、构建 pin、真实数据库与资产恢复及整关故障注入仍未验收。候选记录不能当作恢复许可。详见 [C4 验证记录](docs/p0c4-verification.md)。

2026-10-03 源端 live 准入的五个 PG18 门已获本切片限定接受，覆盖同库跨 attempt/root 互斥；跨根崩溃持久身份与真实源端完整捕获仍待父 Task3 验收。

## 代码与文档导航

- [项目架构设计文档集](docs/architecture/README.md)：总体、领域模型、Rust 分层、事务权限、资产任务、快照恢复、前端学习、部署演进与决策证据；[离线 HTML](docs/architecture/KnowWeave架构设计.html)。
- `crates/learning-core`：内容、关系、证据、阅读、资产和作业的契约类型。
- `crates/learning-db`、`migrations`：PostgreSQL 持久化、权限、原子操作和升级。
- `crates/learning-assets`：私有原件、安全文件操作与快照包。
- `crates/learning-worker`：持久任务执行器与隔离验收夹具。
- `crates/learning-backup`：C4 备份、完整收据和恢复预检；恢复执行入口仍保持内部、未开放。
- `deploy`：隔离测试配置与初始化脚本；不含生产部署配置。
- `docs/content-boundary.md`：块与内容边界；`docs/p0c3-format.md`：融合快照格式。
- `docs/p0c4-restore-target-acceptance.md`、`docs/p0c4-restore-birth-acceptance.md`、`docs/p0c4-restore-pin-acceptance.md`：C4 单机目标验收协议。
- [源端维护准入](docs/p0c4-source-admission.md)：live 同库互斥、实际五门及父 Task3 尚待完成的边界。
- `docs/superpowers/specs/2026-09-28-p0c4-backup-recovery-design.md` 与 `docs/superpowers/plans/2026-09-28-p0c4-backup-recovery.md`：C4 设计和实施任务。

各阶段的边界、执行取舍和原始证据索引见 `docs/p0*-boundary.md`、`docs/p0*-execution.md`、`docs/p0*-verification.md`。验证记录区分已运行结果、静态审查和未覆盖的门槛；不能把某一阶段通过推断为整体上线。

## 本地构建

需要 Rust 1.97 及锁定的 `Cargo.lock`。纯契约测试不需要数据库；Linux 专属文件、Docker 和 PostgreSQL 门必须在隔离环境中运行。

```sh
cargo fmt --all -- --check
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo test --offline --locked -p learning-core
cargo test --offline --locked -p learning-backup --lib
```

`--offline` 要求依赖已缓存。完整工作区数据库测试需要 PostgreSQL 18 的专用空库和明确的管理/运行角色 DSN；缺少测试环境会失败，不会静默跳过。先阅读对应阶段的验证记录与 `deploy/compose.test.yaml`，为每次验收建立新的项目、数据库和卷。不要把管理凭据交给浏览器或未经认证的主体，也不要在已有数据库上运行会插入夹具、迁移或故障触发器的测试。

本仓库只保存源码、迁移和文档；私有凭据、课程原件、测试包与服务器原始证据不属于源码交付。
