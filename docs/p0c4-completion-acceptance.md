# P0-C4 Task7：完整 Attention 与隔离验收

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
