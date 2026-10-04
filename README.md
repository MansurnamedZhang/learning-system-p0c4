# 知织 · KnowWeave

知织是以“块”为最小修订单元的个人学习与知识系统内核。当前代码以 Rust、PostgreSQL 18 和私有文件存储实现内容、关系、阅读、资产、后台任务与快照能力；**没有 HTTP API、登录界面或前端，也不是生产部署版本**。

## 项目状态

| 阶段 | 能力 | 状态 |
| --- | --- | --- |
| P0-A、P0-B1/B2 | 块修订、固定组合、原子发布、个人阅读 | 隔离验收通过；非生产 |
| P0-B3/B4 | 关系、证据/猜想、影响查询 | 隔离验收通过；非生产 |
| P0-C1/C2/C3 | 原件与引用、持久 Worker、融合快照交换 | 隔离验收通过；非生产 |
| P0-C4 | 完整备份与干净实例恢复 | **实施中，未整体验收** |

C4 已通过部分单机隔离门，包括备份清单、安全封存、恢复目标准入、出生证明、干净目标 pin **候选**，以及精确子进程的只读端点绑定/同一 guard 重启拒绝（新批次结果已核对）。独立备份目标、`CompleteBackup`、恢复构建 pin、真实数据库与资产恢复及整关故障注入仍未验收。候选记录不能当作恢复许可。详见 [C4 验证记录](docs/p0c4-verification.md)。

2026-10-04 源备份生命周期新 suite `943f3a4a-16a9-44ab-af07-257151349a7a` 的九个独立新批次及离线 aggregate 已通过，独立返回审查 Spec/Quality/ActualAcceptance 均 PASS，P0/P1/P2/P3 均为 0。实际执行 **17 个 PG body（16 lifecycle + 1 legacy）**，覆盖 **8 个唯一生命周期主用例 + 1 个 legacy**；命名测试入口 `source::lifecycle_tests::real_capture_all_ready_and_retained_pin` 执行 9 次（1 个 primary + 8 次 prelude）；该计数不统计各测试体内部的 prepare_source_backup/pg_dump 总调用次数。文件系统 36 次为 4 个唯一测试体重复 9 次，相关回归 171 次为 19 个唯一测试体重复 9 次；普通 Linux Python 468 通过/27 个具名 root 跳过、独立 root 27 通过/0 跳过分别计数。现场输入为 e43 基线的 475 文件 `working-tree-green` ZIP，后续发布不改变实际执行身份。 原具备生命周期 API 的管理二进制和原编译 source/build/control binding 必须保留；latest binary 不自动兼容旧捕获。详见[生命周期 runbook](docs/p0c4-source-attempt-lifecycle.md)和[C4 验证记录](docs/p0c4-verification.md)。父 C4 Task3 三个框、Task4/5、资产/local-pin 持久 enrollment 与 GC 保护发现、独立故障域、CompleteBackup、完整恢复、C4 与生产继续开放；完整工作区 DB 与四套旧版本升级未在本切片运行。源端同身份物理克隆端点仍待验，与此前已接受的恢复目标 clone 门分别记录。 旧绑定切片已完成文档复审并正常发布，收据见[绑定页](docs/p0c4-source-control-binding.md)；旧 admission 五门仍为历史独立来源。

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
- [源备份尝试完成与放弃](docs/p0c4-source-attempt-lifecycle.md)：四管理 Rust 库入口、原构建绑定、保留证据与终态人工歧义。
- [源控制根持久绑定](docs/p0c4-source-control-binding.md)：独立构建 pin、实际四门与两项独立回归、信任边界。
- [源端维护准入](docs/p0c4-source-admission.md)：live 同库互斥、历史五门及父 Task3 尚待完成的边界。
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
