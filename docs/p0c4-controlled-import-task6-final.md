# P0-C4 受控合成导入 Task6 验收汇总

更新日期：2026-10-03。**Task6 限定验收已完成，源码与文档已正常同步到 GitHub。**

范围为单机、新隔离 PostgreSQL 18 实例、固定两行合成 fixture 的私有测试入口。累计 11/11（1 项正例、10 项预期拒绝）；当前受验源码 `17153d83590b174910058538ab2569bd809217f0` 实际运行 7/11，另四项来自 `56190262d0f05fea2357205bc71f27b956b7fe82`。独立差异复审确认旧四项相关字节未变，允许携带历史来源；没有把它们写成当前修订重跑。

## 实际场景

| case | 核心观察 | 源码修订 | 状态 |
|---|---|---|---|
| success | 固定两行与主键导入、唯一 COMMIT | `5619026` | 已接受 |
| ready-eof | DDL 前 EOF，无对象或 attempt | `5619026` | 已接受 |
| precommit-eof | 未提交 DDL/COPY 后 EOF，独立零对象读回 | `5619026` | 已接受 |
| precommit-cancel | 提交前取消，零对象读回并确认隔离 | `5619026` | 已接受 |
| ready-restart | DDL 前重启拒绝，旧 guard 不可复用 | `17153d8` | 已接受 |
| sql-error | 固定 SQL 错误，禁止 COMMIT、停机留证 | `17153d8` | 已接受 |
| copy-truncated | COPY 截断拒绝，不盲目宣称回滚 | `17153d8` | 已接受 |
| attempt-sync-failure | 初始同步故障，无 DDL 放行 | `17153d8` | 已接受 |
| commit-intent-sync-failure | 意图同步故障，无 COMMIT 尝试 | `17153d8` | 已接受 |
| commit-unknown | 提交确认未知，不报告回滚或允许重试 | `17153d8` | 已接受 |
| wrong-endpoint | 同 ID/OID 的物理克隆，实际 writer 端点拒绝 | `17153d8` | 已接受 |

## 最终回归与必要前置

| 门 | 实际结果 | 范围 |
|---|---|---|
| 当前源码 Linux 前置 | 库测试 146 通过 / 19 忽略，journal 1 通过；格式、包级严格 Clippy、编译通过 | 已接受的 A1df 前置证据复用 |
| 九目标集成回归 | exit 0；37 通过 / 0 失败 / 2 忽略 | 每个目标与二进制绑定，39 注册名称逐一列举 |
| learning-backup 文档测试 | exit 0；0 可执行用例 | 不将空测试集写成已执行 doctest |
| workspace 严格 Clippy | exit 0；all-targets / all-features / -D warnings | 工作区编译与静态检查；没有执行工作区全部测试 |

最后三门批次为 `dda83916-8dea-4b7d-96b6-6f04f569043e`，`not_run=[]`、操作器与 wrapper 均 exit 0。maintenance_pg 的两项 ignored 只列举，没有执行。旧 768 批次误计非 Linux 用例而失败的记录保留，本轮不追认或重跑旧批次。各独立规格/质量复审无阻断问题。

## 证据来源与隔离

用户终端完整回传的 publisher 结果摘要为 `4c659efca2ec7c94eb6351d7e0037bc0b66ceb9e668eca9454f85c92a3ec1482`；控制者按封存 publisher 的嵌套插入顺序重建 18,805 字节结果，哈希匹配。源码 ZIP 摘要为 `1489fe4f7a45b177c492e9d5850042895a93325b50959488a8a4044d8655cafe`，manifest 为 `36a687b1538d7fd8ec86758b327f20d67ef4a61286e837e30ea3541ad8001589`。完整回传、逐目标清单、前后源码/二进制摘要和控制批次已独立审查。

普通 SSH 的独立 Docker 元数据核验确认：本批 PG 精确容器 ID 已停机，测试卷与网络保留，builder 精确 ID 和名字均不存在。核验记录摘要为 `0152d0886773ed35e5990c3a56f0a3adfd54ea6d61a24f6a4cd2b466b9250e4a`。控制者未直接读取 root 私有日志、结果、marker 或 fsync 事实；pending 消失和持久发布来自已审操作器的成功回传。私有原始证据保留在测试主机，不进入 Git。

## 提交与同步

源码及五份验收文档已通过正常 fast-forward 同步至现有 `feat/p0c4-backup-recovery` 分支，首轮提交为 [`4b551b9`](https://github.com/MansurnamedZhang/learning-system-p0c4/commit/4b551b92a099672f6da06bf9ba46e487fd4421ed)。Git 推送退出 0；独立 GitHub 读取确认远端提交与本地相同，内容树均为 `05ea9c8607e314d5b065dffb22c20004fa41e757`。main 保持原提交；此次同步不改变受验源码 `17153d8`、用例来源或验收边界。后续文档收口提交只记录已完成的验收与同步事实。

## 当前边界与下一步

限定范围只接受 `CONTROLLED_FIXTURE_IMPORT_PASSED_SINGLE_HOST_QUARANTINED_NOT_FULL_RESTORE`；没有新增默认/发布构建写入口。上层 P0-C4 整关、一般/完整恢复、资产与任务恢复闭包、独立故障域备份、CompleteBackup 和生产仍未验收。

后续回到[上层备份恢复施工单](superpowers/plans/2026-09-28-p0c4-backup-recovery.md)的完整恢复闭包与最终整关；当前受控导入的技术条件见[批准施工单](superpowers/plans/2026-09-30-p0c4-controlled-import.md)。本文件提供可随源码发布的脱敏汇总，历史原始私有记录保持封存。
