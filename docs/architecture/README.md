# KnowWeave 架构设计 · 2026-11-08 审查版

当前交付物是 Rust 领域内核、私有原件和后台任务体系，以及分阶段的交换、备份与恢复能力。P0-A、B1–B4、C1–C3 已分别通过历史隔离验收。C4 Task8 已完成规定私有单机回归，Task9 与 C4 整关开放。HTTP、Web 工作台和学习流程仍处于设计阶段。

本版以 GitHub 功能分支 `08c3687ef39416411af9f21a743fed09ccf8ef3f` 为审查基线。历史方案保留原合同和采用记录。准备、代码、限定检查、实际现场、整链及生产状态分别列出；日期与源码身份随证据一起读取。

| 章节 | 用途 |
| --- | --- |
| [01 总体架构与产品边界](01-overview.md) | 产品价值、当前交付与目标分层 |
| [02 块与版本化领域模型](02-domain-model.md) | 身份、修订、位置、关系、来源与学习事实 |
| [03 Rust 模块与代码边界](03-modules.md) | 五 crate、依赖、迁移和实际入口 |
| [04 事务授权与关系查询](04-transactions-and-permissions.md) | 并发、重试、授权闭包和查询预算 |
| [05 原件存储与持久任务](05-assets-and-jobs.md) | 文件、任务接管、外部效果及清理 |
| [06 快照交换与备份恢复](06-portability-and-recovery.md) | 三类产物、B1 时序、独立恢复与 HOLD |
| [07 API 前端与学习闭环](07-product-interfaces.md) | 可验证的首用切片、接口提案与验收 |
| [08 部署运维与架构演进](08-deployment-and-evolution.md) | 现有运行环境、信任边界、发布和容量 |
| [09 决策记录与证据索引](09-decisions-and-evidence.md) | 事实来源、决策状态、验收口径 |
| [10 全项目架构与进度审查](10-deep-review.md) | 七项发现、关键路径与重构方案比较 |
| [11 分线交付与验收计划](11-delivery-plan.md) | Task9 现场顺序、产品线建议和停止条件 |

若重点是项目为什么推进慢以及下一步如何做，先读 10、11，再读 06、07。[实施路线图](../../KnowWeave实施路线与架构.html)

## 文档维护

架构正文以这组 Markdown 为源。图以同一份路线图数据生成到 `docs/diagrams/`，两份 HTML 共享图和当前口径。旧路径下的图保留用于历史引用。执行层的详细参数、许可与 HOLD 由对应施工单和现场回执维护。发生冲突时按具体身份与采用范围裁定，再更新此入口的事实索引。

在源码仓库根目录先生成路线图和 SVG，再生成架构离线页。渲染使用本地 Node 与 `marked`，也可用 `KNOWWEAVE_DOCS_NODE_MODULES` 指定已有依赖目录。

```sh
node .build/build-knowweave-map.mjs
node scripts/build_architecture.mjs
node scripts/build_architecture.mjs --output KnowWeave架构设计.html
```
