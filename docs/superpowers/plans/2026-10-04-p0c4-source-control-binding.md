# P0-C4 源控制根持久绑定 Implementation Plan

> **For agentic workers:** 使用 superpowers:subagent-driven-development、test-driven-development 和 verification-before-completion；本计划是已批准父 Task3 的执行细化，继续用户指定的逐任务实现与独立审查。

**Goal:** 原维护会话消失后，捕获与关闸恢复仍只能读取独立预置、构建摘要固定的源控制根，不能通过换目录跳过旧日志。

**Architecture:** 在业务数据库之外预置 `source-binding.json`，其摘要由 `KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256` 固定到可信构建。记录绑定规范控制路径、目录句柄 dev/inode 与 PostgreSQL 数据库名/OID/system identifier；支持入口在同一 admitted 会话核对并持有该根句柄，日志扫描、创建、恢复均相对句柄操作。

**Tech Stack:** Rust 1.97、SQLx 0.8.6、现有 Linux BackupDir、固定 PG18 与离线 builder。

**Spec:** [已批准 C4 设计](../specs/2026-09-28-p0c4-backup-recovery-design.md)，[父实施计划 Task3](2026-09-28-p0c4-backup-recovery.md)。本文只细化源根身份前置条件；广义 finish/abandon、捕获恢复与整链验收继续由父计划跟踪。

## Global Constraints

- 不改旧迁移字节、旧版本身份或 C1–C3 公开契约；不增加业务迁移或把控制状态搬入业务库。
- 管理权限和密钥绝不进入 Worker、runtime 或备份包；不记录密码或完整 DSN。
- 本切片只有 `source-binding.json` 的独立构建 pin 是身份预期来源；配置路径和运行时环境不能提供替代 pin，缺失或不符即拒绝，不能自动注册。
- 所有路径均在 Linux 可信 root 所有、祖先不可被 group/other 写入的树中核验。保留原会话锁、角色/会话/预备事务谓词；验证之前不得改变 ACL、创建尝试日志或运行 dump。
- 每个新服务器批次使用新目录、Compose 项目、PG18 卷与未占用子网，旧批次留存、不重放。用户后续直接指令已授权范围内自主验证，无需再逐包确认；sudo 密码仅在用户终端输入。
- 独立故障域按用户安排延后；本轮不执行恢复、不签发 CompleteBackup、不标父 Task3/C4 或生产完成。

## Review Focus

- 原 admitted backend 已消失且 CONNECT 曾被重新 GRANT 时，替代根必须因绑定拒绝，不能以仍忙的锁或缺少其他前置条件冒充证明。
- 复制相同绑定字节到另一个根，或在原路径替换 inode，必须拒绝；固定路径本身不足以证明身份。
- 恢复入口同样受绑定约束；正确根只重新关闸并保留未完成日志，不自动继续或废弃捕获。
- 核验后路径被换名不能让扫描原根、随后却在新根写日志；操作须全程保留目录句柄。
- 非规范 JSON、未知字段/版本、链接、宽权限、过大或缺失绑定与编译 pin，均拒绝且无维护副作用。

## 当前执行状态（2026-10-04）

行为 RED 在全新 b2a63c86 实际确认；41f658ea 的 Clippy 基础设施失败保留。冻结修补经独立静态复审，新发现 0；新 27bdfd1d 实际四门 4/4、两项准入回归各 1 通过，Linux 库 158/30 ignored 含四个文件系统测试体。source/binary/binding 三次 audit 一致，PG 四容器停止、留卷/内部空网、helpers 移除、pending 不存在已由控制端核对。独立限定返回审查 Spec/Quality/Acceptance PASS、新发现 0；Task1/2 的本切片核验项和 Task3 独立文档复审/精确源码提交推送交付已完成；源码提交 `e3afb0545463ea8924df30ee340e586d9edf9cc3`、tree `fe3dc0c57b23c65f3fe2d8f0b3c3cc9a1f1d5342`、`refs/heads/feat/p0c4-backup-recovery` 已由 GitHub 独立核验。实际现场仍为 a56ab16-base working-tree-green ZIP；本次仅文档记账，不代表新的现场运行或新增验收。本版实际证据与边界见[绑定记录](../../p0c4-source-control-binding.md)。父 Task3 三项、Task4/5/C4/生产仍开放；无完整 dump/恢复/CompleteBackup，独立存储故障域延后。

## Task 1：绑定与句柄能力

**Files:** 创建 `source/binding.rs`、`source/binding_tests.rs`；修改 `source.rs`、`maintenance.rs`。

**Interfaces:** 私有绑定核验从独立编译 pin 读取预期摘要；消费同一个 `SourceAdmission` 和控制根路径，返回保留的 `BackupDir` 能力。`SourceGateJournal` 增加内部 handle-relative start/recover；原公开路径包装及 v1 日志字节保留。公开 `prepare_source_backup` 与 `force_close_release_ready` 无 unbound fallback。

- [x] 先写不需要新生产 API 的实际错误根恢复 RED 测试；冻结测试后交控制器在新 PG18/root-in-container fixture 执行。RED 必须是基线真正执行恢复并改变 ACL 的行为断言，基础设施错误不算。
- [x] 在确认 RED 后实现严格绑定解析、same-session 身份核对与句柄相对日志；补纯解析/文件拒绝、inode 替换及路径换名测试。
- [x] 定向测试、格式和严格 Clippy 后独立审查。Windows 结果只计纯契约；Linux 路径与 PG 门分别验收。

## Task 2：独立预置与实际 Linux/PG18 验证

**Files:** 创建 `scripts/p0c4_source_binding_gate_acceptance.py` 与对应 Python 测试；消费 Task1 的精确测试名称和源模块接口。

**Interfaces:** 普通 hans 编排独立新 fixture，可信 pin/root 所有权在固定镜像的 root 容器命名空间内核验；独立 helper 根据实际目录句柄和 SQL 观察预置绑定，再把摘要装入离线构建。预置和消费使用同一路径命名空间。

- [x] 先测试驱动的权限、身份、预算、脱敏与停机判定；核验冻结源码、构建、二进制及每个精确测试输出，不把列举当执行。
- [x] 只运行一次新的 RED 批次；确认上述 Task1 错误根行为失败之后才允许生产实现。
- [x] 实际 GREEN 精确门：错误根/旧会话已消失拒绝；原根未完成日志仍阻止新尝试；正确根 close-only 恢复；同服务器另一真实数据库身份拒绝。每门新项目/卷，各 exit0、1 pass/0 fail/0 ignored；绑定文件、源码与二进制前后不变。
- [x] 库测试和相关日志/准入回归按同一源码执行；独立复核实际输出/哈希、停机留卷与无 pending。所有 prior dump/pin digest 均注明 SYNTHETIC，所有 PG 门注明 NO_DUMP。

## Task 3：文档与可审查交付

**Files:** 更新 `docs/p0c4-source-admission.md`、`docs/p0c4-verification.md`、父计划、README，以及工作区实施路线图。

- [x] 记录本版实际结果和未验收边界，历史版本各自保留，不累计旧门成新版本计数。
- [x] 独立代码与证据/文档复审后提交精确文件，正常推送已授权的 `feat/p0c4-backup-recovery`，核对远端 ref/tree。
- [x] 父 Task3 仍开放；下一项显式 finish/abandon 与真实 dump/全量资产/保留保护组合链。
