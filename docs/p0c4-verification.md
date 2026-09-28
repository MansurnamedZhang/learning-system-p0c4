# P0-C4 完整备份与干净恢复验证记录

**状态：实施中；未验收；非生产。**

## 基线与范围

- C3 已验证基线：`4507f3f55f860a6d3afed2cefe2bce27f7ff6b7c`。
- C4 工作分支：`feat/p0c4-backup-recovery`；工作树：`D:\codex\DeepLearning\.worktrees\knowweave-p0c4`。
- 目标是完整 PostgreSQL 权威数据、所有 ready 原件、角色/模式重建信息和干净实例恢复；C3 融合快照不是整库备份。
- Linux 服务器仍须每份精确源码包单独确认后，在新隔离 Compose 项目验证。没有通过证据前不写 `P0_C4_VERIFIED`。

## 当前门槛

| 门槛 | 结果 | 证据或待办 |
|---|---|---|
| C3 基线源码可编译 | 已通过 | 本地 `cargo test --offline --locked --workspace --no-run`，2026-09-28，退出码 0 |
| Rust 格式基线 | 已通过 | 本地 `cargo fmt --all -- --check`，2026-09-28，退出码 0 |
| C4 Task 1 清单/契约 | 本地门通过；整项未验收 | `84d601b` 实现、`973ca89` 与 `691fd4d` 复审修复；纯契约 9/9、前置条件 2/2、lib 1/1，本地格式/严格 Clippy/工作区编译通过；独立静态复审无阻断，真实 PG18 专项待隔离运行 |
| C4 Task 2 安全复制/封存 | 未开始 | SIGKILL、链接、哈希和私有目录门 |
| C4 Task 3 写闸/dump/保护/完成收据 | 未开始 | 真 PostgreSQL 并发与独立故障域证据 |
| C4 Task 4 干净恢复 | 未开始 | 全资产/身份/权限/租约审查 |
| C4 Task 5 全链与失败注入 | 未开始 | 四套旧版升级、工作区回归、整关验收 |

所有后续结果需记录源码提交与包 SHA-256、项目名、新数据库/卷、工具版本、命令、退出码、日志哈希、失败根因及清理状态。失败批次不可覆盖或改写为通过。

## Task 1 本地证据

- 源码 HEAD：`691fd4d`（后续文档提交不改变 Task 1 代码）；独立复审已核对三次 Task 1 代码提交，最后一轮静态结论无阻断。
- 实现者观察到两轮编译 RED：缺少契约/管理入口类型，后续缺少隔离预检/迁移指纹/容量函数；修复后纯契约 9/9、空库与角色前置条件 2/2、库单测 1/1 通过。
- 根任务复核 `cargo test --offline --locked -p learning-backup --test contract`：9/9，退出码 0；`cargo test --offline --locked -p learning-backup --test catalog_pg preflight`：2/2，退出码 0；`cargo test --offline --locked -p learning-backup --lib`：1/1，退出码 0。
- 实现者运行 `cargo fmt --all -- --check`、`cargo clippy --offline --locked --workspace --all-targets -- -D warnings`、`cargo test --offline --locked --workspace --no-run`，均退出码 0；根任务曾独立复核格式和严格 Clippy，均退出码 0。
- 本机没有 PostgreSQL 命令或隔离 DSN；`catalog_pg` 的真实数据库测试**只编译未运行**。必须在全新 PG18/新数据库/新 Compose 项目中执行，核验 `system_user`、角色属性、复合类型探针及全部 ready 资产，再决定 Task 1 过门。

## 设计裁定

- Ruling: C4 走独立整库备份路径，不复用 C3 授权阅读包 — C3 排除了用户、空间、授权、作业等权威行 — 若误用会产生缺失恢复点。
- Ruling: 本轮隔离维护窗先停止 runtime/Worker、拒绝 runtime 新连接并排空旧事务；在线只停写能力留给 P1 接入后另验 — 现有写入口分散，单入口闸无法证明完整 — 代价是隔离备份期间阅读暂停。
- Ruling: Task 2 只交付 `sealed` 复制原语，Task 3 源端一致性/保留保护及目标校验后才可产生 `complete` — 先封存字节并不代表数据库与资产属于同一恢复点 — 代价是任务之间必须保持状态类型隔离。
- Ruling: SHA-256 检测损坏，不证明有包写权限者未伪造；依赖私有管理目录和独立运行证据，恢复仍全量重验 — 若目标目录不可信需以后另加认证签名/MAC — 当前不宣称抗恶意目标管理员篡改。
- Ruling: 新实例恢复使所有旧实例运行中租约失效，按重试上限和外部副作用记录分类 — 仅按过期时间处理会让旧进程令牌仍有效 — 代价是部分作业恢复后需人工核对。
