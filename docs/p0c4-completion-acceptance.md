# P0-C4 Task7：完整 Attention 与隔离验收

## Task7 有限前置通过，整链阶段顺序修复中 · 2026-10-24 18:39:40（北京时间，固定检查点）

源码 `e2ce9af96d6fbfad8be851533232bce984984723` 的正式原生接受仍为 **0/6**，Task7 尚未完成。

本候选的 Linux 有限前置已独立核对：格式与严格 Clippy 通过，Rust 560 项实际通过、142 项 ignored 未执行，普通用户 DAC 一项通过；608 个源码 Git blob、22 个控制器 pin 与归档一致。该有限配方不是 Task8 的真实 PostgreSQL／四套升级验收。

首项新隔离整链在正式测试正文前被既有空根检查拒绝：Attention 原件预填充早于独立签发，触发 SNAPSHOT_PREPARED_PRIVATE_EMPTY_ROOT。旧失败与 HOLD 保留，不重放原批次；正在将固定原件创建移至签发通过后，空根检查及其余守卫不放宽。

源库与目标库均已精确停机留卷，仅移除自然结束业务后的已知控制容器。完整运行前 C618／N586／V2078 投影保持；未将准备失败写成业务用例通过。候选实现与有限前置的信用均不转授尚未执行的新修复源码。

Task6 私有单机 16/16 历史结论保持。Task7–9、P0-C4 整关、独立备份存储、CompleteBackup、公开恢复与生产仍 OPEN。当前仅同步候选分支 `feat/task7-attention-recovery`，未合并 main、未部署。

---

### 原始基线与待验清单

## Task7 实施中 · 2026-10-24 15:49:42（北京时间，固定检查点）

Task7 已进入完整 Attention 场景与故障注入实现，正式接受 **0/6**。当前公开的是测试基线检查点 `2ced6366a23fcc472d7b6995cddc474194313d6b`，并非 GREEN 完成版本。

全新 Linux／PG18 基线已执行真实捕获与导入，在完成全部 **64 表**的归一化前校验后，实际发现 H@1/H@2 均为 0，触发完整性守卫。精确测试退出 **101，0 passed／1 failed／0 ignored**；独立审查确认这是缺失 Attention 行为的预期 RED。失败回执与 HOLD 保留，未转为通过。

正在实现：完整历史／当前知识图、原件、Reading／Release／权限语义比较，以及缺失原件、损坏 dump、中断传输、未知效果、根目录替换五项失败注入。固定原件合同为 5 份物理文件、6 条逻辑资产，尚未取得这些 GREEN 用例的实际通过证据。

Task6 在源码 `2e5026d` 上的代码与私有单机 16/16 限定完成记录保留。Task7、Task8、Task9 与 P0-C4 整关仍 OPEN；当前没有独立备份存储，不签发 CompleteBackup 或公开恢复实例，不代表生产部署。后续证据按各自源码、编译产物和全新隔离资源记录。

详见[Task7 验收清单](../KnowWeave实施路线与架构.html#task7)。

---

| 固定场景 | 当前状态 | 通过条件 |
|---|---|---|
| `attention-full-chain` | 基线 RED 已独核；GREEN 待验 | 全量行、完整函数/ACL、Reading 与权限、原件和任务允许差异 |
| `attention-missing-asset` | 实现中，实际 GREEN 待验 | 原件缺失在目标准入前拒绝 |
| `attention-corrupt-dump` | 实现中，实际 GREEN 待验 | 损坏 dump 在目标准入前拒绝 |
| `attention-interrupted-transfer` | 实现中，实际 GREEN 待验 | 真实封存字节传输中断，保留旧 pin |
| `attention-unknown-effect` | 实现中，实际 GREEN 待验 | 导入后未知效果阻止恢复可用性 |
| `attention-root-replacement` | 实现中，实际 GREEN 待验 | 同路径新 inode 被真实根身份复核拒绝 |

基线观察摘要 SHA-256：`33ec4fbb9e87158e4595f0aaebaf254e08526d9b713a20f751a2c73bb7f7f2d4`。它证明旧空夹具缺少 Attention 历史，不替代完整夹具的验收。资源收尾核对旧 616 容器、584 网络、2067 卷的完整保护投影未变；只删除已自然结束业务子进程的本批控制容器，源／目标卷与失败原件保留。私有原件、凭据、完整 inspect 和数据库内容不发布。

当前源码包 SHA-256：`ea4c39e58f23a69e1de446ae0e66e7d487bc1532fa825863e3a5d2beb1551964`；manifest SHA-256：`4bce09c4be31e04db5b6a66a07e49f6ab9acbbad7c5db6e667a5f37ab65f8e94`；608 个源码文件、22 个控制器模块固定绑定，15 份迁移与既有源码逐字相同。静态复审与本地纯测试不构成六案现场接受。
