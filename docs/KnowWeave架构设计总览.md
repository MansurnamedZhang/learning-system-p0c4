# KnowWeave 架构设计文档入口

初次整理：2026-09-30；当前状态更新：2026-10-25。完整架构正文按主题拆分，公开源码仓库的`docs/architecture/`与离线HTML共同维护；历史章节保留原核对日期，状态以当前路线与验收入口为准。

- [完整离线 HTML](../KnowWeave架构设计.html)：十个阅读章节、六张架构图，可搜索和打印。
- [可编辑 Markdown 文档集](https://github.com/MansurnamedZhang/learning-system-p0c4/tree/feat/task7-attention-recovery/docs/architecture)：按主题维护、随源码交付。
- [实施路线与架构图](../KnowWeave实施路线与架构.html)：查看分期与验收门。
- [Task8 当前回归与待验边界](p0c4-task8-current-regression.md)：实际完整十门/四套升级、专项证据与未完成项。
- [限定采用与历史来源](p0c4-completion-acceptance.md)：Task6/7各自私有单机范围及历史检查点。

当前：Rust五个crate；A、B1–B4、C1–C3已各自隔离验收。Task5代码与规定单机补验、Task6的2e5026d私有16/16、Task7的Source36私有6/6已分别限定采用。当前a5完整十门、四套真实旧版升级、Root FS/OS33与源码七叶差异已限定独审通过；规定专项原件与Task8整组采用仍待闭合。Task9独立目标、CompleteBackup、正常公开恢复、C4整关及生产仍开放。登录、HTTP API、前端和完整学习闭环待建设。

历史设计保留：[总体方案](学习系统设计方案.md)、[内容与存储](内容分块与存储规范.md)、[连续阅读](连续阅读与思考块设计.md)、[技术栈](Rust与前端技术方案.md)、[部署与恢复](服务器部署与数据恢复设计.md)、[数据库首版决策](数据库选型实验审阅与首版决策.md)。模型示例、旧实验和目标说明不自动成为当前产品功能。
