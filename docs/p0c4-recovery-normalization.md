# P0-C4 Task6：恢复归一化与完整校验

更新：2026-10-20。代码检查点：`5b0fa3157a0e6c688d54006c53b10e949434b106`。**基础实现与指定 Linux 检查已通过；Task6 整体仍 OPEN，现场 PostgreSQL 当前 0/5。**

## 本轮实现

- 从同一已验证 dump 派生全部 64 表的预期行集合，以主键排序、完整字段摘要核对恢复结果，再允许修改任务。
- 在单独绑定的顶层管理事务中归一化任务，同时保留原目标 guard。清除过期及未过期旧租约；保留 ID、幂等键、attempt、checkpoint 和合法终态回执，最多三次尝试。
- 在管理私有目录持久化原状态、外部观察、HOLD、允许变更和提交意图。提交、回执同步或取消结果不确定时隔离，禁止自动重放。
- 校验全部 ready 资产与原件、真实完整性 Result-v1 摘要和引用；共享应用读取核心验证当前与历史阅读修订、三种投影和拒绝路径。此检查要求管理角色，不能描述为普通 runtime 连接验收。
- 正常恢复资格继续要求 CompleteBackup；测试 rehearsal 不能产生正常资格或开放服务。

实现入口：[任务归一化](../crates/learning-backup/src/full_restore/jobs.rs)、[完整校验](../crates/learning-backup/src/full_restore/validation.rs)、[全表行比对](../crates/learning-backup/src/full_restore/data/rows.rs)、[应用读取校验](../crates/learning-db/src/reading/recovery.rs)。

## 本轮实际验证

| 检查 | 结果 | 范围 |
| --- | --- | --- |
| 固定 Linux 格式、测试目标类型检查、严格 Clippy all-targets | 通过 | 无 PostgreSQL、网络或 Docker 卷 |
| Linux 全表行/摘要辅助测试 | 21 passed / 0 failed / 0 ignored；457 filtered | 单元测试 |
| Linux 任务规划、关联与共享预算测试 | 15 / 0 / 0；463 filtered | 单元测试 |
| Linux 取消与统一截止时间测试 | 3 / 0 / 0；475 filtered | 单元测试 |
| Linux 恢复策略测试 | 7 / 0 / 0；0 filtered | 单元测试 |
| Python 固定配置与结果封套 | 33 项通过 | 未执行现场场景 |
| 五项新 PostgreSQL 恢复场景 | 0/5，尚未运行 | 不授现场信用 |

Linux 结果摘要：`d0193b90c456e5b0d65541ede0ccb23bfcc509be0d7bbec119ce3beeef42d500`。独立复核已完成；已知编译容器自然结束并按精确 ID 移除，无清理 HOLD。旧 Docker 资源前后投影摘要相同；本轮只二次读取其哈希元数据，没有再次下载完整旧资源原文。代码提交的 21 个路径与通过检查的工作树包逐字核对；构建采用其原 base 身份，不能冒充新提交身份的现场包。

此前 Windows 行辅助 21 项、先前 planner 12 项及策略 7 项分别留档。以上不相加为语义覆盖总数，历史测试不替代当前现场验收，先前失败证据不抹除。

## 完成 Task6 还需要

1. 在全新源库与目标上，分别执行并复核旧令牌拒绝、重试上限、外部副作用、非空派生结果/阅读校验、回执失败五个固定场景。
2. 补齐可在目的端独立核验的 snapshot 字节证据链，证明 NoEffect / AlreadyCommitted 正面分支；与数据库 receipt、源端捕获及目标包绑定，不能依靠调用方枚举或恢复时回源猜测。
3. 完成全范围独立审查、回执与故障收尾。正常资格仍要求真实 CompleteBackup；Task7–9、独立存储及生产开放另有验收门。

**当前所有缺独立 snapshot 字节证据的导出作业保持 Unknown/HOLD。** 这能避免误重发，却不是 Task6 全功能完成。已有终态数据库回执保持原字节，不自动重建或重新发布外部导出。

Task5 只在代码与规定单机补验范围关闭：九项 native 及三组回归保留各自原来源；CU 业务仍 UNKNOWN，wrong-endpoint 只覆盖错误数据库路由，asset-corrupt 不证明零数据库写入。详细历史见[项目进度](项目最新进度.md)及[Task2、3、5 收尾](P0-C4补验与收尾.md)。
