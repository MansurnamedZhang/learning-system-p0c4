# KnowWeave 架构设计文档入口

初次整理：2026-09-30；当前状态更新：2026-10-26。完整架构正文按主题拆分，公开源码仓库的`docs/architecture/`与离线HTML共同维护；历史章节保留原核对日期，状态以当前路线与验收入口为准。

- [完整离线 HTML](../KnowWeave架构设计.html)：十个阅读章节、六张架构图，可搜索和打印。
- [可编辑 Markdown 文档集](https://github.com/MansurnamedZhang/learning-system-p0c4/tree/feat/task7-attention-recovery/docs/architecture)：按主题维护、随源码交付。
- [实施路线与架构图](../KnowWeave实施路线与架构.html)：查看分期与验收门。
- [Task8 当前回归与限定验收边界](p0c4-task8-current-regression.md)：实际完整十门/四套升级、专项证据与Task9未完成项。
- [限定采用与历史来源](p0c4-completion-acceptance.md)：Task6/7各自私有单机范围及历史检查点。

当前：Task8已按规定私有单机范围完成：A5完整Linux/PG18十门、四套真实旧版升级、54项fresh专项（53项A5＋1项Source679）、6项未变历史桥及整组独审。RootFS33与生命周期FS5单列；各源码与证明层级分别记账。 新21操作及全部规定正文、五FS辅助与Task8整组审查已限定收尾；Task9独立目标、C4整关与正常恢复仍开放。

历史设计保留：[总体方案](学习系统设计方案.md)、[内容与存储](内容分块与存储规范.md)、[连续阅读](连续阅读与思考块设计.md)、[技术栈](Rust与前端技术方案.md)、[部署与恢复](服务器部署与数据恢复设计.md)、[数据库首版决策](数据库选型实验审阅与首版决策.md)。模型示例、旧实验和目标说明不自动成为当前产品功能。

Task8已按规定私有单机范围完成：A5完整Linux/PG18十门、四套真实旧版升级、54项fresh专项（53项A5＋1项Source679）、6项未变历史桥及整组独审。RootFS33与生命周期FS5单列；各源码与证明层级分别记账。 全部规定专项的有限原件、来源、实际正文与精确资源收尾已限定采用；所有旧失败/HOLD保持。生命周期真实13,094日志经独审后的16k读取配方完整导出13,099文件，512MiB读/输出、32MiB单件、300/360秒与资源等式不变。Full9大negative、部分最终proof与Schema未选output、ELF/inspect/独立OS时钟等层级限制继续披露，不宣称所有物理状态或正常恢复均已外部证明。 Task8已在规定私有单机范围关闭；Task9及C4整关保持开放。
