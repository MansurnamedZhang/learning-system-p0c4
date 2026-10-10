# P0-C4 完整备份与干净恢复 Implementation Plan

> **For agentic workers:** 每项由实现子代理完成，独立子代理复审，根任务核对测试与证据；使用 `superpowers:subagent-driven-development`、`superpowers:test-driven-development` 与 `superpowers:verification-before-completion`。复选框只按真实结果更新。

**Goal:** 产生可独立校验的整库恢复点，在全新实例恢复 P0-A/B/C 的全部权威数据和原件，不接受残缺备份或半可用实例。

**Architecture:** 新建与业务 runtime 隔离的管理工具/契约模块；管理角色协调 PostgreSQL dump 和全量 `asset` 清单，复用文件存储的安全哈希工具，目标故障域校验后封存。恢复器预检全部文件后才在新库/资产根写入，随后运行数据闭包与 Attention 回归。

**Stack:** Rust 1.97、SQLx 0.8.6、PostgreSQL `pg_dump`/`pg_restore`、SHA-256、现有安全文件存储、Docker Compose 隔离验证。

**Spec:** [C4 详细设计](../specs/2026-09-28-p0c4-backup-recovery-design.md)，上位 P0-C 总设计（私有或未公开参考） C4 节。

## 全局约束

- 基线 `4507f3f55f860a6d3afed2cefe2bce27f7ff6b7c`；不改旧迁移字节、旧版本身份、C1–C3 公开契约。C4 不通过 C3 授权导出接口伪造整库备份。
- 管理权限和密钥绝不进入 Worker、runtime 或备份包；所有测试使用新库/卷，保留 C3 证据。未取得实际异故障域与完整恢复证据时不写 `VERIFIED`。
- 每任务先写能定位失败的 RED，再实现、复审、执行适当本地测试；真实 PostgreSQL/Linux 门使用精确源码 ZIP。依据后续用户直接授权“无需要我逐项确认，自己去做”，批准范围内自主封存/上传/验证，不重复逐包确认；仍核对精确 payload pins，使用全新隔离批次和资源，保留旧失败与证据。范围发生实质变化时再升级确认，sudo 凭据仍只在用户 Linux 终端输入。

## Task 1：全量备份契约、资产清单与拒绝规则

- [x] 在独立 `learning-backup` 管理 crate 定义 `BackupManifestV1`、文件与资产记录、`staging/sealed/complete` 状态和规范摘要；`asset-index.json` 按 `(space_id,id)` 排序逐行保存全部逻辑资产，文件摘要进入 manifest。API 明确区分未完成规划与可恢复备份；Task 1 不发布磁盘完成收据。
- [x] 管理角色从 `asset` 全部 ready 行采集；同 SHA 去重但保留逻辑资产计数，拒绝同 SHA 不同 size/key、非规范 key、负长度，空库可行。首版上限 100,000 行、64 MiB 规范索引，查询至上限加一行后显式失败。运行时角色即使拥有 `asset` SELECT，也不能调用管理入口。入口仅规划，不写 `complete`。
- [x] RED/GREEN：纯契约边界、同 SHA 多逻辑资产、冲突摘要、无资产、manifest 文件集合缺失/额外、非规范路径、不能从 plan 构造 complete；应用提交和完整迁移集合的规范指纹、行/索引容量上限。磁盘扫描与目标端收据留给 Task 2/3。真实 PG18 专用空库集成测试覆盖未链接 ready、同 digest 多行、经身份/权限核对的普通 runtime 拒绝。独立复审公开 API、序列化确定性与整数溢出；隔离 ws3 格式、专项、严格 Clippy 和 PG18 门通过。证据见 [C4 验证记录](../../p0c4-verification.md)；这仍不是 C4 全链验收。

## Task 2：安全文件封存与目标端全量校验

- [x] 独立私有 staging 中流式复制 dump、角色配方和每个原件；原件复用 `FsAssetStore` no-follow 打开和哈希校验，不继承 C3 的 128/512 MiB 预算。
- [x] 本任务只实现已验证的 `sealed` 复制原语；`complete` 必须等 Task 3 的源端一致性/保留保护及独立目标全量校验。中途终止、目标缺文件/改字节、链接/重解析点、同名碰撞不产生可恢复包。重复相同 digest 字节去重，保留每包完整清单。
- [x] Linux 文件系统专项验证 `fsync`/rename/读回、SIGKILL 和部分写入/同步故障注入；不对共享主机断电。Windows 本地仅能验证纯契约，不能宣称 no-follow 门通过。证据见 [C4 验证记录](../../p0c4-verification.md) 的 Task 2 ws3；同步故障为测试钩子注入，未模拟真实掉电。

## Task 3：维护窗、数据库 dump 与资产保护

2026-10-04 源备份生命周期新 suite `943f3a4a-16a9-44ab-af07-257151349a7a` 的九个独立新批次及离线 aggregate 已通过，独立返回审查 Spec/Quality/ActualAcceptance 均 PASS，P0/P1/P2/P3 均为 0。实际执行 **17 个 PG body（16 lifecycle + 1 legacy）**，覆盖 **8 个唯一生命周期主用例 + 1 个 legacy**；命名测试入口 `source::lifecycle_tests::real_capture_all_ready_and_retained_pin` 执行 9 次（1 个 primary + 8 次 prelude）；该计数不统计各测试体内部的 prepare_source_backup/pg_dump 总调用次数。文件系统 36 次为 4 个唯一测试体重复 9 次，相关回归 171 次为 19 个唯一测试体重复 9 次；普通 Linux Python 468 通过/27 个具名 root 跳过、独立 root 27 通过/0 跳过分别计数。现场输入为 e43 基线的 475 文件 `working-tree-green` ZIP，后续发布不改变实际执行身份。 详见[生命周期runbook](../../p0c4-source-attempt-lifecycle.md)和[接受ledger](../../p0c4-verification.md)。父 C4 Task3 三个框、Task4/5、资产/local-pin 持久 enrollment 与 GC 保护发现、独立故障域、CompleteBackup、完整恢复、C4 与生产继续开放；完整工作区 DB 与四套旧版本升级未在本切片运行。源端同身份物理克隆端点仍待验，与此前已接受的恢复目标 clone 门分别记录。 当前child Task2/2A/2B限定接受不关闭下列父Task3三框；子文档及整源码复审、正常feature源码发布与独立ref/tree回读已完成，收据见[生命周期runbook](../../p0c4-source-attempt-lifecycle.md)。root静态14/14、6SVG、56HTMLlinks通过，浏览器file协议策略拒绝、未验证。当前只作发布记账，不关闭下列父Task3三框。

以下绑定段为历史切片：

2026-10-04 新源控制根绑定切片实际四门 **4/4**，另有两项 live admission 回归各通过一次；独立限定返回审查 Spec/Quality/Acceptance PASS，新增发现 0；该绑定切片文档复审与正常feature分支发布已完成，见既有e3afb收据；实际执行仍为原a56ZIP。单可信容器命名空间的独立预置 `source-binding.json` / build pin 绑定规范 path/dev/inode 和同会话 DB name/OID/system identifier；无 runtime expected pin 或自动 enrollment。原会话消失后的替代根拒绝、原根日志权威、正确根 close-only 重关闸、另一真实库拒绝均实际执行，第二门含真实 prepared-work 公共拒绝。输入为 a56ab16 基线 working-tree-green 的 469 文件 ZIP（SHA-256 `95e6c48702dd3557ce7beaa1e3427779e5944315922820ec5071d48ceae8ceae`），result SHA-256 `28a1f798b49e4dbc80cce9f92e46374653a3b1fe9b45d0ad720083f3b4e4156f`。见[绑定切片记录](../../p0c4-source-control-binding.md)。当时父Task3下述三项继续开放；其后限定finish/abandon与真实本地capture进展见上文。asset/local-pin 身份、物理同身份克隆端点仍分别跟踪，legacy full-capture/drain/release drivers 延后到更新独立 issuer/build 后。没有 full dump/restore/CompleteBackup 或独立存储验收。

以下为历史源码切片：

2026-10-03 live source admission 切片在新批次 `06a0882f-5c7a-4fa5-ab86-252ece7732fa` 实际通过并限定接受五个精确 PG18 门 5/5，各 exit 0、1 通过、0 失败/忽略、stderr 0 字节。输入为基于 `0adab2e340c0980c120364a1214ebb53277e4405` 的未提交 `working-tree-green` 快照；规范 result SHA-256 `98b0883bef44abe10259006e34b5481e322fac38511421b35c37d56afd74cd98`。同会话 authenticated owner/expected DB 与固定 two-i32 session lock 实现跨 attempt/root 的 live 同库互斥，owned guard 覆盖 SQL/catalog/journal/dump/seal/pin/release/补偿。冻结切片静态 Spec/Quality 与独立限定返回接受均 Approved，Critical/Important/Minor 均为 0；控制端已核对实际五门与精确停止/留卷/空网/构建器移除，只接受本切片 5/5。未执行真实 dump，release 前序摘要为 SYNTHETIC。详见[源端维护准入](../../p0c4-source-admission.md)和[C4 验证记录](../../p0c4-verification.md)。

历史专项，属于不同源码且不自动转移到当前候选：

2026-10-03 全新 `c9f1cdaf-de98-4f69-96a1-0c6a479fc7cb` 批次三个真实 Linux/PG18 聚焦门 **3/3**，各 1 通过、0 失败/忽略，main 退出 0：当前库预备事务在原连接关闭后仍阻止预检且 ACL/日志不变；其他库持有预备事务不误阻塞空的当前库；释放日志失败补偿 REVOKE 与显式重新关闸通过。第三项使用 SYNTHETIC 前序摘要，三项均未执行 `pg_dump`。用户完整 root 回传的规范哈希/输入 pins/源码不变已独立核对；普通用户只读观察确认三个精确容器停止、卷及内部空网络保留、构建器移除，控制器未直接读取 root 私有原始结果。详见 [维护闸补充验证](../../p0c4-maintenance-gates.md) 和 [C4 验证记录](../../p0c4-verification.md)。

上述历史专项的独立限定返回审查 Spec/Quality Approved，Critical/Important/Minor 均为 0，仅接受 c9f 聚焦三门 **3/3**；原三门未在本候选重跑，借用签名/所有权适配经静态审查未发现需要追加定向回归的实质问题，不能与当前五门相加成新版本 8/8。该历史记录当时父 Task3 **保持开放**：live 锁本身不提供会话消失后的持久身份；当前新增绑定实际四门见上文，广义中断完成/放弃恢复，以及真实 dump/全部 ready assets/index/保留保护到独立目标的组合校验和完成收据仍待验收；以下三个复选框均未勾选。用户已将独立存储故障域安排延后，当前继续代码/单机隔离范围；Task4/5、完整备份/恢复、`CompleteBackup`、C4 与生产继续开放。旧失败批次及 0/3 历史证据保留，不重放。

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
