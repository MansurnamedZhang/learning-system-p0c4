# P0-C4 完整备份与干净恢复 Implementation Plan

> **For agentic workers:** 每项由实现子代理完成，独立子代理复审，根任务核对测试与证据；使用 `superpowers:subagent-driven-development`、`superpowers:test-driven-development` 与 `superpowers:verification-before-completion`。复选框只按真实结果更新。

**Goal:** 产生可独立校验的整库恢复点，在全新实例恢复 P0-A/B/C 的全部权威数据和原件，不接受残缺备份或半可用实例。

**Architecture:** 新建与业务 runtime 隔离的管理工具/契约模块；管理角色协调 PostgreSQL dump 和全量 `asset` 清单，复用文件存储的安全哈希工具，目标故障域校验后封存。恢复器预检全部文件后才在新库/资产根写入，随后运行数据闭包与 Attention 回归。

**Stack:** Rust 1.97、SQLx 0.8.6、PostgreSQL `pg_dump`/`pg_restore`、SHA-256、现有安全文件存储、Docker Compose 隔离验证。

**Spec:** [C4 详细设计](../specs/2026-09-28-p0c4-backup-recovery-design.md)，上位 [P0-C 总设计](../../../../../docs/superpowers/specs/2026-09-23-p0c-assets-jobs-portability-design.md) C4 节。

## 全局约束

- 基线 `4507f3f55f860a6d3afed2cefe2bce27f7ff6b7c`；不改旧迁移字节、旧版本身份、C1–C3 公开契约。C4 不通过 C3 授权导出接口伪造整库备份。
- 管理权限和密钥绝不进入 Worker、runtime 或备份包；所有测试使用新库/卷，保留 C3 证据。未取得实际异故障域与完整恢复证据时不写 `VERIFIED`。
- 每任务先写能定位失败的 RED，再实现、复审、执行适当本地测试；真实 PostgreSQL/Linux 门使用精确源码 ZIP，每份上传逐包确认。

## Task 1：全量备份契约、资产清单与拒绝规则

- [x] 在独立 `learning-backup` 管理 crate 定义 `BackupManifestV1`、文件与资产记录、`staging/sealed/complete` 状态和规范摘要；`asset-index.json` 按 `(space_id,id)` 排序逐行保存全部逻辑资产，文件摘要进入 manifest。API 明确区分未完成规划与可恢复备份；Task 1 不发布磁盘完成收据。
- [x] 管理角色从 `asset` 全部 ready 行采集；同 SHA 去重但保留逻辑资产计数，拒绝同 SHA 不同 size/key、非规范 key、负长度，空库可行。首版上限 100,000 行、64 MiB 规范索引，查询至上限加一行后显式失败。运行时角色即使拥有 `asset` SELECT，也不能调用管理入口。入口仅规划，不写 `complete`。
- [x] RED/GREEN：纯契约边界、同 SHA 多逻辑资产、冲突摘要、无资产、manifest 文件集合缺失/额外、非规范路径、不能从 plan 构造 complete；应用提交和完整迁移集合的规范指纹、行/索引容量上限。磁盘扫描与目标端收据留给 Task 2/3。真实 PG18 专用空库集成测试覆盖未链接 ready、同 digest 多行、经身份/权限核对的普通 runtime 拒绝。独立复审公开 API、序列化确定性与整数溢出；隔离 ws3 格式、专项、严格 Clippy 和 PG18 门通过。证据见 [C4 验证记录](../../p0c4-verification.md)；这仍不是 C4 全链验收。

## Task 2：安全文件封存与目标端全量校验

- [x] 独立私有 staging 中流式复制 dump、角色配方和每个原件；原件复用 `FsAssetStore` no-follow 打开和哈希校验，不继承 C3 的 128/512 MiB 预算。
- [x] 本任务只实现已验证的 `sealed` 复制原语；`complete` 必须等 Task 3 的源端一致性/保留保护及独立目标全量校验。中途终止、目标缺文件/改字节、链接/重解析点、同名碰撞不产生可恢复包。重复相同 digest 字节去重，保留每包完整清单。
- [x] Linux 文件系统专项验证 `fsync`/rename/读回、SIGKILL 和部分写入/同步故障注入；不对共享主机断电。Windows 本地仅能验证纯契约，不能宣称 no-follow 门通过。证据见 [C4 验证记录](../../p0c4-verification.md) 的 Task 2 ws3；同步故障为测试钩子注入，未模拟真实掉电。

## Task 3：维护窗、数据库 dump 与资产保护

2026-10-03 继续从现有源端实现补齐预备事务排空检查，并制备三个独立 PG18 聚焦门；本地通过与现场待执行分开登记，见 [维护闸补充验证](../../p0c4-maintenance-gates.md)。跨尝试准入、广义中断恢复、独立目标及完成收据仍待后续验收，本任务复选框保持未关闭。

- [ ] 定义管理进程的写闸/排空协议与可恢复控制记录；隔离门先停 runtime/Worker 进程，拒绝新 runtime 连接，再确认所有既有 runtime 会话和事务排空；其他写角色/无法证明排空则失败关闭。未来 P1 的在线只停写协议另验。旧在途事务结束后生成 dump 和清单，失败自动保持安全状态，显式恢复流程有记录。
- [ ] `pg_dump -Fc` 使用可信固定参数、管理角色与脱敏日志；收集 PG 主版本、SQLx 迁移状态、应用提交和非敏感角色配方。DB dump、清单、保留保护先持久化，再解除写闸；源端证明和独立目标全量核验都通过后才发 `complete` 收据。
- [ ] 真实 PG RED/GREEN：维护期业务写入拒绝、在途事务排空、资产清理保护、dump/清单一致、失败后保护和写闸状态明确。未解决运行时写闸绕过则不得进入整关验收。

## Task 4：只恢复到干净实例

- [ ] 在目标任何写入前验证 `complete`、manifest、全部原件/dump/角色配方、版本；目标库/资产根须为空且私有。
- [ ] 使用独立凭据建立角色及模式，`pg_restore` 严格退出码；恢复资产并按全部 `asset` 行全量复核，使**所有**源实例运行租约令牌失效，再按重试上限和外部幂等记录分类，重建派生物。成功验收前保持实例不可用。
- [ ] 真实 PG RED/GREEN：缺文件、坏 SHA、脏目标、角色错误、失败 dump、孤儿引用、租约、旧版与当前版/个人间隙/未放置/关系审查/授权。失败实例不放行。

## Task 5：完整 Attention 场景和失败注入

- [ ] 生成含原件、图片、H@1/H@2、两个复用组合、个人笔记/图片、E1/E2/X、审查、固定 Reading/Release、拆分和未放置记录的源库；备份到独立目标，干净实例恢复，逐身份/摘要/位置/权限比对。
- [ ] 注入复制中断和恢复缺文件；先前有效备份仍可恢复，失败批次不显示 complete。执行四套冻结旧程序升级、C1–C3 回归、全 workspace、格式与严格 Clippy。
- [ ] 静态独立审查与 Linux 隔离复验都过门，保存完整证据、更新路线图与 `docs/p0c4-verification.md`；只标 `P0_C4_VERIFIED / NOT_PRODUCTION`。
