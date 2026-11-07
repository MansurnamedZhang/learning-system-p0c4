# 06 快照交换与备份恢复

| 产物 | 权威范围 | 可用结论 |
| --- | --- | --- |
| `reading_copy_v1` | 经裁剪的可读内容与允许的媒体 | 固定版本离线阅读；缺少精确导入所需身份和闭包 |
| `exact_import_v1` | 一个 Reading 的授权闭包及选定原件 | C3 范围交换；用户、空间、grant、实例历史和任务状态另属全量恢复 |
| C4 完成备份 | 一致数据库 dump、资产全量清单及原件、角色、构建和迁移、独立证据与完成收据 | 只有实际通过独立见证才可进入正常完整恢复 |

![阅读副本、范围交换与完整恢复](../diagrams/knowweave-portability.svg)

## C3 范围交换

导出在一致读事务固定 Reading 及必要引用闭包。规划、成功结果和交付按各自边界核对当前授权。私有 attempt 目录按固定清单流式处理，检查路径、摘要、大小、预算与 no-follow。正式入口是受控目录，Linux 承担权威文件链的验收。Windows 的合同检查有独立边界。

metadata-only 保留授权资产元数据，included-originals 模式才复制并计入该预算。若包缺原件，导入须以目标的同空间、同业务资产身份、同元数据及实际字节满足条件，只有相同 SHA-256 的另一资产无法代用。

导入先校验内容、引用、兼容性和资产，再固定必要文件句柄并定稿 CAS。数据库单事务重新授权，核对不可变字段、自然键及 registry，提交 29 表范围内的事实、允许的新 head 和批次回执。已有发布与工作头按原规则保留。同 ID 同内容可复用，冲突拒绝。事务失败可能留下未登记字节，业务视图保持原子可见。[C3 格式与表白名单](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/08c3687ef39416411af9f21a743fed09ccf8ef3f/docs/p0c3-format.md)

## C4 源端、封存与完整收据

可信协调器按维护协议停止 runtime 和 Worker 写入、撤销 CONNECT 并排空相应会话，然后捕获数据库 dump 和全量资产目录。生成角色、资产索引、构建/迁移身份及版本化清单，并对私有 pin/封存包做无覆盖定稿和完整回读。逻辑资产数与唯一文件字节数分别统计，无当前业务引用的逻辑资产仍属于审查范围。协议对 CONNECT 和清理状态保留持久 journal；维护窗和占用目前需要真实测量。

`SealedBackup` 代表本地封存及核对。独立目的端实际重读、证据、受信签名与完成收据成立后，`CompleteBackup` 才可由正常入口接受。已暂存的密钥、测试签名、包内容或后来读取到的残留只支持各自的结论。

### Appendix B1 的正式权限边界

1. S 在同一原操作中保持受认证 actor、会话、租约、累积预算和根身份，完成最终重核，独占持久化并回读负 fence。认证和传输、备份签名三种密钥按用途分开。
2. D 原十一阶段通过并由原进程取得最终决定。D 在本地串行裁决内从 Pending 进入 Consuming，完整签名、禁止覆盖、同步和验证均自然返回后才进入 Committed；本地错误、超时或结果不明进入 Failed/HOLD。
3. Consuming 先赢得许可后，S 退出或迟到取消不追溯撤回。历史合法签名残留可按原 v2 条件读取；读取、原调用成功、新发布权限是三种独立证据。
4. 首版现场计划持续保留 S1 fence。可能阻塞同 registry 的后续备份和相关管理。S2/V2 使用独立身份，异常解除或补操作须按准确范围另审。

[Appendix B1 规范](../superpowers/specs/2026-10-04-p0c4-completion-design.md) · [最终决定采纳记录](../task9/b1-final-decision-adoption.md)

## 干净目标与完整恢复

| 环节 | 关键身份或约束 |
| --- | --- |
| 准入及会话 | 固定镜像和 PG18 初始化事实、角色与 ACL、目标容器/网络/卷/挂载、数据库 OID 和 system identifier。出生证明、实际启动身份、Unix socket、同一 SQLx 会话和锁持续绑定。 |
| 导入 | 依受审的模式、TOC、COPY、角色和事务约束执行完整引擎。错误端点、同 ID 克隆及重启旧 guard 分别拒绝。失败后依据实际副作用隔离目标。 |
| 恢复后校验 | 核对全量表、固定历史、个人层、关系和授权、原件真实字节。归一化任务，拒绝旧租约/token。按效果证明重建安全状态，外部效果不明则保留 Unknown/HOLD。 |
| RA 与 RB | RA 从独立完成备份在 fresh 目标正常恢复。RB 验证实验源不可访问时仍能只凭 D 的包、证据与 trust 通过相同校验。两案分别核实实际阅读副本及精确停机。 |

![Task9 从源端准入到独立目标与两次恢复](../diagrams/knowweave-recovery-gates.svg)

## 当前进展与限制

- Task5 的受控 11 项采用包含 10 项历史与 1 项当次来源。Task6 16/16、Task7 6/6 与三个阅读副本字节、Task8 完整回归及四套真实旧版升级，均按各自规定的私有单机或隔离范围采用。
- Task9 的 D 已选定，三把密钥已有各自主机暂存和托管证明。D ELF 已做实参构建；实际安装信任和启动链仍待核验。C0/CF2 的普通 Linux 检查覆盖默认关闭代码，现场十五项视图和其他原始义务单列。
- 原 S1 PG18 首启触及 QEMU 任务上限并停止，保留 HOLD。干净 S1、新临时 unit 和峰值判据已封存；等待重跑审批。S1 全部门通过才运行 S2。Task9、C4 整关、正常 CompleteBackup/RecoveredInstance 和生产均保持开放。

[限定验收台账](../p0c4-completion-acceptance.md) · [Task8 证据](../p0c4-task8-current-regression.md) · [Task9 现场顺序](../task9/installation-and-two-host-acceptance-plan.md) · [早期章节快照](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/08c3687ef39416411af9f21a743fed09ccf8ef3f/docs/architecture/06-portability-and-recovery.md)
