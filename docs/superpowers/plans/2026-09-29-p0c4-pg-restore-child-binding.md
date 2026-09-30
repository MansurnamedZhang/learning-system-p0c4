# P0-C4 `pg_restore` 子进程只读端点绑定 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** 2026-09-30 用户确认继续实施；本地实现与独立审查进行中，现场验收待精确包授权。

**Goal:** 在恢复首次写入前，用精确容器 ID、本地 Unix socket 和固定客户端命令建立独立的只读子进程端点准入证明。

**Architecture:** `BoundTargetGuard` 保留出生/Pin 的 Docker 身份，且必须与仍存活的 SQLx 双锁事务一起生成短生命周期的子进程目标。固定 Docker CLI 在精确容器内运行清空环境的 PostgreSQL 18 只读客户端，前后以同一 socket 再核对两把随机锁、backend PID/OID 和容器；旧宿主 `pg_restore` 写路径保持不可达。实际 dump 导入需要另一计划和新隔离写入验收。

**Tech Stack:** Rust 1.97、SQLx 0.8.6、PostgreSQL 18、固定摘要的 Docker PG18 镜像、Python 隔离验收 runner。

**Spec:** [子进程端点绑定设计](../specs/2026-09-29-p0c4-pg-restore-child-binding-design.md)。证据边界见 [C4 验证记录](../../p0c4-verification.md)。

## Global Constraints

- 本计划不执行 `pg_restore` 导入、不写 `restore.attempt`、不增加公开恢复入口、`CompleteBackup` 或服务放行。
- 用户对每个服务器源码包/runner 和新项目批次逐文件单独确认；旧批次、卷和证据不可复用。
- 宿主 root/Docker daemon 与固定 PG18 镜像在信任边界内；runtime、Worker、调用方 host/port/flags、容器名称与环境变量不在边界内。
- 固定 `/usr/bin/docker` 和 64 位小写十六进制容器 ID；容器内 `/usr/bin/env -i`、绝对客户端路径、本地 socket `/var/run/postgresql`、端口 `5432`、角色 `learning_admin`、出生数据库名。无 shell、TTY、密码参数或新密钥挂载。
- 观察前后保留 daemon、容器、镜像、网络、卷、挂载及 `StartedAt` 一致性，并经同一容器内 socket 验证 SQLx 事务的两把锁、backend PID/数据库 OID；异常只输出固定原因码/类型，不记录原始 stderr、密码、完整 DSN 或随机锁值。
- Docker 子进程在读取 stdout/stderr 期间即须有独立字节上限和总截止时间，固定 SQL 还须设置服务端 `statement_timeout`；超限/超时杀死并 wait 回收宿主 CLI，并对精确 ID 容器执行失败隔离/停机确认，无法确认则标记不可用，不能复用无界 `.output()` 后才检查长度的 helper。

## Review Focus

1. 同 system identifier/OID 的错误容器或容器名：Task 1/2 测试必须拒绝。
2. Docker `exec` 继承容器 `PG*` 环境：Task 1 测试必须要求容器内 `/usr/bin/env -i` 和显式 socket 参数。
3. 目标容器或 postmaster 在探针前后重启，或卷/镜像变化：Task 2 测试必须通过双锁和 Docker 观察失败关闭。
4. Unix socket 指向同 ID/OID 克隆、要求密码、错误数据库或错误角色：Task 2 测试必须拒绝且不创建 attempt 标记。
5. 宿主 `pg_restore` 路径或密码文件在检查后替换：Task 1 必须使旧宿主执行路径不可达，而非靠一次 `symlink_metadata` 放行。

## Task 1 — 移除旧宿主写路径并固定命令契约

**Files:** `crates/learning-backup/src/restore_policy.rs`、`crates/learning-backup/src/restore_preflight.rs`、`crates/learning-backup/src/restore_preflight/target_binding.rs` 及相邻单测。

**Interfaces:** `BoundTargetGuard` 与 `LockChallenge<Transaction<'static, Postgres>>` 的内部组合生成 `ExactRestoreChildTarget`，其字段私有、生命周期不超过两者；它只暴露固定的只读客户端 argv 构造和容器/数据库身份读取，不接受调用方 host、port、可执行文件或 PostgreSQL 参数。旧 `PgRestoreSpec` 可暂保留数据校验兼容，但移除宿主执行方法及 `RestorePreflight` 中可达的导入方法；不应存在 marker→旧宿主子进程的代码路径。

- [ ] 写 RED：容器名、非小写 64hex ID、错误数据库、TCP host、额外 `PG*` 环境和任意客户端路径不得生成命令；正常目标的 argv 精确包含 `docker exec -i --user 999:999 <ID> /usr/bin/env -i`、固定绝对客户端路径及本地 socket。
- [ ] 运行定向测试并记录预期 RED；实现不透明目标和固定 argv。移除旧宿主导入调用链，保持其他预检/锁/3a/3b 行为不变。
- [ ] 运行 `cargo test -p learning-backup --lib`、`cargo fmt --all -- --check`、严格 Clippy 和相邻 Python 回归；提交后独立审查。本地无专用 PG 环境时不得称 workspace 全部通过。

## Task 2 — 精确容器内的只读客户端能力探针

**Files:** `crates/learning-backup/src/restore_preflight/target_binding.rs`、同模块测试；必要时独立小模块承载命令结果解析。

**Interfaces:** 在已持有 `BoundTargetGuard` 和 `LockChallenge<Transaction<'static, Postgres>>` 的内部状态下执行；探针返回不包含凭据的 `ChildReadOnlyAttestation`，只在两者仍存活的作用域中有效，不能转为写权限。

- [ ] 写依赖注入 RED：固定 PG18 `pg_restore --version`、`psql --no-password` socket 查询必须看到原 SQLx 事务的两把随机锁、同一 backend PID/数据库 OID；错误版本/角色/库、socket 指向同 ID/OID 克隆、容器不重启但 postmaster 重启、密码请求、Docker 身份漂移、客户端退出失败、格式外输出、超时/超量输出、超时后容器内进程可能残留及原始错误信息均拒绝。
- [ ] 探针前后在相同事务上重新执行现有严格 `verify_sql_session`，并由与未来 child 相同的固定 socket 命令独立观察锁行；再核对精确 Docker/PG 身份。新增有界、带截止时间的 Docker 子进程执行器：在读取时分别限制 stdout/stderr，固定只读 SQL 设置 `statement_timeout`；超时/超限杀死并 wait 回收宿主 CLI，随后按已核验精确容器 ID 停机隔离，停机不可确认则留 `UNCONFIRMED_UNUSABLE`。宿主和容器内环境均清空；探针只读，不接触 dump、资产或 marker。
- [ ] 运行 focused Rust tests、格式、严格 Clippy 与旧只读门回归；独立审查并提交。成功只称 `CHILD_READ_ONLY_ATTESTED_NOT_RESTORE`。

## Task 3 — 新隔离 Linux/PG18 验收

**Files:** opt-in ignored Linux test、Python runner 与测试、`docs/p0c4-verification.md`。

- [ ] 对 Task 2 的 exact-container 双锁/PID/OID 成功、容器重启拒绝做聚焦 live 测试；postmaster 在容器未重启时变化先用依赖注入单测验证，只有额外审定的监督进程夹具才允许现场复现。如纳入同 ID/OID 物理克隆错端点，必须使用同批独立新项目。错误数据库和要求密码的 socket 先在依赖注入单测验证；只有另行准备出生前即固定认证策略的新项目才能作为 live 负例，不能改造已签发/已 Pin 的目标。证明没有 attempt 标记、导入、资产写入或服务放行。
- [ ] 子代理实现并独立审查；本地 RED→GREEN、格式/严格 Clippy/相关回归通过。只用 Git 跟踪字节制包并独立核对 ZIP/manifest/runner SHA-256。
- [ ] 按用户逐文件确认后，仅在新 UUIDv4 Compose 项目、未占用子网和新 PG18 卷执行。核对结果/证据哈希、精确 ID 停机、留卷隔离及 pending 缺失；失败批次不重跑，按结果更新 C4 验证记录。

## 后续边界

本计划通过仍不能证明真实 `pg_restore` 的第一笔写入。后续需单独设计固定子进程执行和凭据契约，先在全新隔离目标用受控小型 dump 做写入验收，再连接完整恢复与资产闭包。不能用本计划的只读收据作为 `CompleteBackup` 或恢复授权。
